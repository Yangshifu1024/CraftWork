//! git 引擎统一落点（决策 D13）：应用内置 Git 能力的唯一 gix 封装层。
//!
//! gix（gitoxide）依赖只允许出现在本 crate；消费方（craft-tools / craft-agent /
//! craft-marketplace / src-tauri）一律经本 crate 的门面 API，不直接接触 gix。
//! 行为规格见 D13。
//!
//! 模块划分（spec 第 4 节，按里程碑逐步补齐）：
//! - [`engine`]：仓库发现 / 打开、工作树与快照（bare）区分、config 快照读取（M1）。
//! - [`errors`]：错误归一化——可读中文错误码，不向调用方外泄 gix 原始错误（M1）。
//! - [`transport`]：clone / fetch（浅克隆、超时重试、代理与 GitHub 加速回退，M2）。
//! - [`status`]：工作树状态采集 + `--short --branch` 同构渲染，含 ahead/behind（M3）。
//! - [`log`]：rev-walk + oneline/decorate 渲染 + 路径过滤（M3）。
//! - [`diff`]：两模式 tree-diff + blob UnifiedDiff + 文件级 patch 头拼装（M4）。
//! - [`commit`]：两模式提交管线（变更收集 → blob 写入 → TreeEditor 构建 tree →
//!   repo.commit_as → index 写回）（M5）。
//! - [`worktree`]：linked worktree 的 add（分支创建 + 检出 + 占用检查）/ remove /
//!   未提交与未合并检测 / prune 自写（M6）。

pub mod commit;
pub mod diff;
pub mod engine;
pub mod errors;
pub mod log;
pub mod status;
pub mod transport;
pub mod worktree;

pub use engine::{Repository, RepositoryKind, Signature};
pub use errors::{Error, Result};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

/// 读取 git 全局配置的 `user.name`（只读 global 级，等价 `git config --global --get user.name`）。
///
/// 供显示名解析使用：未配置或值为空返回 `None`。不依赖任何仓库（gix 的全局配置
/// 文件直读，含 `GIT_CONFIG_GLOBAL` 覆盖语义，与系统 git 一致）。
fn global_user_name() -> Option<String> {
    // 只装配 Global 级来源（~/.gitconfig 与 XDG config；GitInstallation/System 不参与，
    // 与 `git config --global` 的读取范围一致）。定位失败或非文件路径的来源直接跳过
    // （与 gix 的 from_globals 对 path=None 条目的忽略语义一致）。
    let mut metas = Vec::new();
    for source in gix::config::source::Kind::Global.sources() {
        let path = source
            .storage_location(&mut gix::path::env::var)
            .and_then(|p| p.is_file().then_some(p));
        if let Some(path) = path {
            metas.push(gix::config::file::Metadata {
                path: Some(path),
                source: *source,
                level: 0,
                trust: gix::sec::Trust::Full,
            });
        }
    }
    let home = gix::path::env::home_dir();
    let options = gix::config::file::init::Options {
        includes: gix::config::file::includes::Options::follow_without_conditional(home.as_deref()),
        ..Default::default()
    };
    let file = gix::config::File::from_paths_metadata(metas, options).ok().flatten()?;
    let value = file
        .string("user.name")
        .map(|value| String::from_utf8_lossy(&value).into_owned())?;
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

/// 用户显示名：git 全局 `user.name` → 系统用户名（USER / USERNAME）→ 兜底 "CraftWork"。
///
/// 回退链与迁移前 `craft_core::platform::user_display_name` 一致（git 读取从进程调用
/// 换为 gix 的全局配置解析，语义等价）；仅供侧栏账户位展示，不参与任何授权判断。
/// 本函数放在 craft-git 而非 craft-core：git 配置读取是 gix 能力且只允许本 crate
/// 使用，而 craft-git 依赖 craft-core，反向依赖会成环（票 #3 的依赖决策）。
pub fn user_display_name() -> String {
    if let Some(name) = global_user_name() {
        return name;
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "CraftWork".into())
}

/// 在后台线程执行只读 git 操作，超过 `timeout` 视为失败并协作式中断（spec 第 4 节
/// 「超时与中断用 gix 的 should_interrupt 协作式取消 + 包装超时」的等价物）。
///
/// * `f` 收到中断标志；耗时类操作（status / log）应把它传给 gix 的
///   `should_interrupt_owned`，或在循环中轮询，超时后尽快退出。
/// * 超时时置位标志并返回 [`Error::Internal`]（`context` 为传入的中文上下文）；
///   后台线程随后自行结束（只读操作幂等，结果丢弃），不会泄漏进程。
///
/// 现有调用方的超时语义（工具 20s/30s、envcontext 3s、侧栏 5s）由各自传入等时时长保留。
pub fn with_timeout<T, F>(timeout: Duration, context: &'static str, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&Arc<AtomicBool>) -> Result<T> + Send + 'static,
{
    let flag = Arc::new(AtomicBool::new(false));
    let worker_flag = Arc::clone(&flag);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // 发送端关闭（主线程已超时放弃）时结果自然丢弃。
        let _ = tx.send(f(&worker_flag));
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => {
            flag.store(true, Ordering::SeqCst);
            Err(Error::Internal {
                context: context.to_string(),
                detail: format!("超过 {} 毫秒未完成，已中止", timeout.as_millis()),
            })
        }
    }
}
