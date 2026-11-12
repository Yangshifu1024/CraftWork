//! 传输层：clone / fetch（spec 第 5 节 transport，票 `gix-migration#2` + `network#4`）。
//!
//! 职责与边界：
//! - 高层入口 [`clone_repo`] / [`fetch_repo`]：浅克隆（默认 depth 1，对齐 `git clone --depth 1`
//!   的单分支 refspec 行为，由 gix 的 clone 管线内部保证）、整体超时、网络类失败重试、
//!   GitHub 加速前缀改写与失败回退。
//! - **本模块不读设置**（craft-git 不接触 settings）：代理选项（[`GixProxyOptions`] 中性结构）
//!   与加速前缀全部由调用方组装传入——生产侧从 `craft-core::network::ProxyConfig`
//!   解析（spec D14）。
//! - 错误一律归一化为 [`crate::errors::Error`]，不向调用方外泄 gix 原始错误类型。
//!
//! # 超时机制（选型说明）
//!
//! gix 的网络传输是同步阻塞实现，没有「整体操作超时」；HTTP 传输选项里的
//! `connect_timeout` 只覆盖连接阶段。因此本模块采用「协作式中断 + 后台定时线程」：
//!
//! 1. 每次尝试前起一个定时线程，超过时限后置位 `should_interrupt`（gix 在传输各检查点
//!    观察该标志并尽快收尾）——覆盖传输、协商、收包全程；
//! 2. 同时把同一时限经 reqwest 后端的 `configure_client` 回调写入
//!    `ClientBuilder::connect_timeout`——兜住「TCP 连接黑洞、无检查点可触发协作中断」的
//!    连接阶段（gix 0.90 reqwest 后端自带 20 秒硬编码连接超时，回调在其后执行，可覆盖）；
//! 3. 操作结束后通过信号通道唤醒定时线程提前退出，成功路径不白等整个时限。
//!
//! 注意：gix 的 clone 在被中断时可能返回 `Ok`（部分检出完成），因此无论结果成败都要检查
//! 中断标志，置位即按超时归一化。
//!
//! # 代理注入（reqwest 后端的实现缺口，注释存证）
//!
//! gix 0.90 的 `http::Options` 定义了 `proxy` / `no_proxy` / `connect_timeout` 字段，但其
//! **reqwest 后端尚未消费它们**（curl 后端才消费；reqwest 后端只在构建客户端时读硬编码的
//! 20 秒连接超时）。为在零 C 依赖（决策 D13，不引入 curl）的前提下让代理真实生效：
//!
//! - 通用字段照常填写 `Options::proxy` / `no_proxy` / `connect_timeout`（未来 gix 消费后
//!   自动生效，语义不变）；
//! - 另经 `Options::backend` 注入 reqwest 专属回调，在 `ClientBuilder` 上实际装配代理、
//!   排除列表与连接超时。`Some("")` = 完全禁用代理（连环境变量一并禁用，curl 语义），
//!   对应 `ClientBuilder::no_proxy()`；`None` = 不干预（reqwest 默认行为）。
//!   file:// 等本地协议的 transport 会忽略这份配置，无需按协议分支。
//!
//! # UA 注入缺口（注释存证，勿用 configure_request 绕过）
//!
//! 通用字段 `options.user_agent` 照填统一口径（`craft_core::network::APP_USER_AGENT`），
//! 但**实际发出的仍是 gix 硬编码的 `git/oxide-<ver>`**：gix-transport 把它作为请求的静态
//! 头（`user_agent_header`）直接挂上，而 reqwest 只在请求头缺失时才补客户端默认头，客户端级
//! UA 覆盖不了请求已带的头；reqwest 后端也不消费 `options.user_agent`（与 proxy 字段同一
//! 上游缺口，未来消费后自动生效）。
//!
//! 唯一能改的钩子是 `configure_request`，但它会让 gix 把重定向策略切成
//! `RejectConfiguredHeaders`——任何重定向直接报错。代价不可接受：`http://` 地址的 301 升级
//! 与 `owner/repo → owner/repo.git` 形态的 301（gitlab 等自建服务的常见形态）都靠重定向
//! 完成克隆。故不做，缺口留在此处。
//!
//! # 回退语义（spec D14 第 4 节）
//!
//! URL host 为 github.com / raw.githubusercontent.com 且加速前缀有值 → 先用改写 URL 尝试；
//! 改写 URL 耗尽重试仍失败 → 用原始 URL 重试一次（同一代理选项）；仅一次回退，不轮询多源。
//! 成功且经回退 → 结果标注 `fell_back = true`（供商店 UI 诊断展示）。改写判定复用
//! `craft_core::network::rewrite_github_url`，本模块不重复实现 host 白名单。

use std::fmt;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use craft_core::network::{GixProxyOptions, rewrite_github_url};

use crate::errors::{Error, Result};

/// gix HTTP 传输选项（proxy / no_proxy / connect_timeout / backend）。
type HttpOptions = gix::protocol::transport::client::blocking_io::http::Options;

/// 单次尝试的整体超时默认值（spec 第 5 节：90 秒）。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);
/// 网络类失败的重试次数默认值（spec 第 5 节：3 次）。
pub const DEFAULT_RETRIES: u32 = 3;
/// 浅克隆深度默认值（对齐 `git clone --depth 1`）。
pub const DEFAULT_SHALLOW_DEPTH: u32 = 1;
/// 重试间隔（固定值而非指数退避：商店下载的重试对象是离散的下载动作而非高频 API 调用，
/// 固定 500ms 语义简单可预测，也避免指数退避在「加速不可达 → 回退」场景叠加出过长等待）。
const RETRY_BACKOFF: Duration = Duration::from_millis(500);
/// HEAD 短哈希长度（与 `git rev-parse --short` 的默认 7 位一致；不做歧义扩展）。
const SHORT_SHA_LEN: usize = 7;

// ---------- 请求与结果 ----------

/// 克隆请求（[`clone_repo`] 的入参）。
#[derive(Debug, Clone)]
pub struct CloneRequest {
    /// 仓库 URL：http/https、file:// 或本地路径。
    pub url: String,
    /// 克隆目标目录（必须不存在或为空；由调用方保证，gix 默认拒绝克隆进非空目录）。
    pub dest: PathBuf,
    /// 浅克隆深度；`None` = 默认 1。传 `0` 报内部错误（浅克隆必须至少取 1 个提交）。
    pub shallow_depth: Option<u32>,
    /// gix 代理选项（调用方从 `craft-core::network::ProxyConfig::gix_proxy_options()` 组装）。
    pub proxy: Option<GixProxyOptions>,
    /// GitHub 加速前缀；有值时对 github.com / raw.githubusercontent.com 生效并启用回退。
    pub accelerator_prefix: Option<String>,
    /// 单次尝试的整体超时。
    pub timeout: Duration,
    /// 网络类失败的重试次数（0 = 不重试）。
    pub retries: u32,
}

/// 已有仓库的刷新请求（[`fetch_repo`] 的入参）。
///
/// 刷新目标由 `repo_path` 处仓库的 `origin` 远程配置（克隆时写入的 refspec）决定；
/// `url` 仅用于加速改写判定与错误诊断，改写后的 URL 作为本次连接地址使用。
#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// 仓库 URL（应与克隆来源一致，用于加速改写与诊断）。
    pub url: String,
    /// 待刷新的本地仓库路径。
    pub repo_path: PathBuf,
    /// gix 代理选项，语义同 [`CloneRequest::proxy`]。
    pub proxy: Option<GixProxyOptions>,
    /// GitHub 加速前缀，语义同 [`CloneRequest::accelerator_prefix`]。
    pub accelerator_prefix: Option<String>,
    /// 单次尝试的整体超时。
    pub timeout: Duration,
    /// 网络类失败的重试次数。
    pub retries: u32,
}

/// 克隆结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneOutcome {
    /// 克隆完成后的 HEAD 短哈希（固定 7 位；空远程仓库为空串）。
    pub head_sha: String,
    /// 是否经「加速失败 → 原始 URL 回退」成功（供商店 UI 诊断展示）。
    pub fell_back: bool,
    /// 实际使用的最终 URL（诊断用）。
    pub url_used: String,
}

/// 刷新结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchOutcome {
    /// 远程当前位置的短哈希（优先取本次 ref advertisement 中的 HEAD；回退本地
    /// `refs/remotes/origin/HEAD`）。注意 fetch 不移动本地工作分支的 HEAD——这里指的是
    /// 远程最新位置，供调用方判断是否需要进一步 checkout。
    pub head_sha: String,
    /// 本次是否拉到新对象（`false` = 远程无更新）。
    pub updated: bool,
    /// 是否经「加速失败 → 原始 URL 回退」成功。
    pub fell_back: bool,
    /// 实际使用的最终 URL（诊断用）。
    pub url_used: String,
}

impl CloneRequest {
    /// 以默认超时 / 重试 / 浅克隆深度构造请求，其余网络选项由调用方按需填充。
    pub fn new(url: impl Into<String>, dest: PathBuf) -> Self {
        Self {
            url: url.into(),
            dest,
            shallow_depth: None,
            proxy: None,
            accelerator_prefix: None,
            timeout: DEFAULT_TIMEOUT,
            retries: DEFAULT_RETRIES,
        }
    }
}

impl FetchRequest {
    /// 以默认超时 / 重试构造请求，其余网络选项由调用方按需填充。
    pub fn new(url: impl Into<String>, repo_path: PathBuf) -> Self {
        Self {
            url: url.into(),
            repo_path,
            proxy: None,
            accelerator_prefix: None,
            timeout: DEFAULT_TIMEOUT,
            retries: DEFAULT_RETRIES,
        }
    }
}

// ---------- 公开入口 ----------

/// 克隆仓库到 `req.dest`。
///
/// 尝试顺序：加速改写 URL（如命中）→ 原始 URL（回退一次）；每个 URL 内先耗尽重试。
/// 成功返回 HEAD 短哈希与回退标注；全部尝试失败返回 [`Error::Network`]（聚合各 URL 根因），
/// 非网络类失败（如源仓库不存在）不重试、直接归一化返回。
pub fn clone_repo(req: CloneRequest) -> Result<CloneOutcome> {
    let plans = build_url_plans(&req.url, req.accelerator_prefix.as_deref());
    let (mut outcome, fell_back, _records) = run_plans(
        &plans,
        req.retries,
        req.timeout,
        RETRY_BACKOFF,
        OP_CLONE_CONTEXT,
        |plan, interrupt| attempt_clone(&req, plan, interrupt),
    )?;
    outcome.fell_back = fell_back;
    Ok(outcome)
}

/// 刷新 `req.repo_path` 处的已有仓库（经其 `origin` 远程）。
///
/// 语义同 [`clone_repo`]：加速改写 → 失败回退 → 重试；`updated` 标明远程是否有新对象。
pub fn fetch_repo(req: FetchRequest) -> Result<FetchOutcome> {
    let plans = build_url_plans(&req.url, req.accelerator_prefix.as_deref());
    let (mut outcome, fell_back, _records) = run_plans(
        &plans,
        req.retries,
        req.timeout,
        RETRY_BACKOFF,
        OP_FETCH_CONTEXT,
        |plan, interrupt| attempt_fetch(&req, plan, interrupt),
    )?;
    outcome.fell_back = fell_back;
    Ok(outcome)
}

// ---------- URL 计划 ----------

/// 一次「URL 尝试计划」：按什么 URL 连接、是否为加速改写产物。
#[derive(Debug, Clone, PartialEq, Eq)]
struct UrlPlan {
    url: String,
    rewritten: bool,
}

/// 构造尝试计划：加速前缀命中 GitHub host → `[改写 URL, 原始 URL]`（改写优先、原始回退）；
/// 未命中或前缀为空 → `[原始 URL]`（单计划，无回退）。host 白名单判定在
/// `craft_core::network::rewrite_github_url` 内（非 github host 原样返回，此处据「是否变化」
/// 判定改写是否发生，不重复实现白名单）。
fn build_url_plans(url: &str, accelerator_prefix: Option<&str>) -> Vec<UrlPlan> {
    let Some(prefix) = accelerator_prefix.map(str::trim).filter(|p| !p.is_empty()) else {
        return vec![UrlPlan { url: url.to_owned(), rewritten: false }];
    };
    let rewritten = rewrite_github_url(url, prefix);
    if rewritten == url {
        vec![UrlPlan { url: url.to_owned(), rewritten: false }]
    } else {
        vec![
            UrlPlan { url: rewritten, rewritten: true },
            UrlPlan { url: url.to_owned(), rewritten: false },
        ]
    }
}

// ---------- 超时 ----------

/// 编排内部的失败形态：超时（定时线程置位中断）或上游归一化错误。
#[derive(Debug)]
enum AttemptFailure {
    /// 整体超时（协作式中断已被触发）。
    Timeout,
    /// [`crate::errors::Error`] 直接透传（是否重试按错误码判定）。
    Upstream(Error),
}

/// 判定失败是否值得重试：超时是网络类；其余看错误码——只有 `network`（HTTP 传输失败：
/// 连接失败、TLS、超时、5xx 等）值得重试，`repo_not_found` / `internal`（源仓库不存在、
/// 非法 URL、不支持的协议）是确定性错误，重试没有意义。
fn is_retryable(failure: &AttemptFailure) -> bool {
    match failure {
        AttemptFailure::Timeout => true,
        AttemptFailure::Upstream(err) => err.code() == "network",
    }
}

/// 给一次尝试套整体超时（机制见模块注释）。
fn run_with_timeout<T>(
    timeout: Duration,
    attempt: impl FnOnce(&AtomicBool) -> Result<T>,
) -> std::result::Result<T, AttemptFailure> {
    let interrupt = Arc::new(AtomicBool::new(false));
    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
    let timer_interrupt = Arc::clone(&interrupt);
    let timer = std::thread::spawn(move || {
        // 提前收到信号（Ok/Disconnected）= 操作已结束，不触发；到点超时才置位中断标志
        if cancel_rx.recv_timeout(timeout) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
            timer_interrupt.store(true, Ordering::SeqCst);
        }
    });

    let result = attempt(&interrupt);

    // 无论成败都通知定时线程退出并等待其收尾（成功路径不白等整个时限，也无线程泄漏）
    let _ = cancel_tx.send(());
    let _ = timer.join();

    // gix 中断时 clone 可能返回 Ok（部分检出），因此以标志位为准
    if interrupt.load(Ordering::SeqCst) {
        return Err(AttemptFailure::Timeout);
    }
    result.map_err(AttemptFailure::Upstream)
}

// ---------- 编排（重试 + 回退） ----------

/// 一次尝试的记录（测试断言调用序列用；生产侧忽略返回值中的记录）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct AttemptRecord {
    url: String,
    rewritten: bool,
    /// 同一 URL 的第几次尝试（0 起；重试从 1 起）。
    retry_index: u32,
}

/// 编排：按计划顺序（改写优先 → 原始回退）+ 计划内重试执行尝试。
///
/// - 网络类失败（[`is_retryable`]）在当前计划内重试 `retries` 次（固定间隔 `backoff`）；
///   计划耗尽后进入下一计划（回退），全部计划耗尽后聚合为 [`Error::Network`]；
/// - 非网络类失败立即中止整体（换 URL 也无法修复，透传原错误保留变体语义）；
/// - 回退标注：存在改写计划且最终成功的是原始 URL 计划 → `fell_back = true`；
/// - 整体失败的 `detail` 聚合每次尝试的 URL 与根因（供调用方诊断，也供测试断言尝试序列）。
fn run_plans<T>(
    plans: &[UrlPlan],
    retries: u32,
    timeout: Duration,
    backoff: Duration,
    op_context: &str,
    mut attempt: impl FnMut(&UrlPlan, &AtomicBool) -> Result<T>,
) -> Result<(T, bool, Vec<AttemptRecord>)> {
    let has_fallback = plans.len() > 1;
    let mut records: Vec<AttemptRecord> = Vec::new();
    let mut failure_lines: Vec<String> = Vec::new();

    for (plan_index, plan) in plans.iter().enumerate() {
        let is_fallback_plan = has_fallback && plan_index == 1;
        for retry_index in 0..=retries {
            records.push(AttemptRecord {
                url: plan.url.clone(),
                rewritten: plan.rewritten,
                retry_index,
            });
            match run_with_timeout(timeout, |interrupt| attempt(plan, interrupt)) {
                Ok(value) => return Ok((value, is_fallback_plan, records)),
                Err(failure) => {
                    let reason = match &failure {
                        AttemptFailure::Timeout => {
                            format!("超时（超过 {} 秒）", timeout.as_secs())
                        }
                        AttemptFailure::Upstream(err) => err.to_string(),
                    };
                    failure_lines.push(format!(
                        "连接 {}（第 {} 次）失败：{reason}",
                        plan.url,
                        retry_index + 1
                    ));
                    if !is_retryable(&failure) {
                        // 非网络类：立即透传（保留 NotFound / Internal 等变体语义）
                        return Err(match failure {
                            AttemptFailure::Upstream(err) => err,
                            AttemptFailure::Timeout => timeout_error(op_context, timeout),
                        });
                    }
                    if retry_index < retries {
                        std::thread::sleep(backoff);
                    }
                }
            }
        }
    }

    // 全部计划（含回退）耗尽 → 聚合错误
    let detail = format!(
        "已尝试 {} 个地址均失败：{}",
        plans.len(),
        failure_lines.join("；")
    );
    Err(Error::Network {
        context: aggregate_context(op_context, &detail),
        detail,
    })
}

// ---------- 错误归一化 ----------

/// 克隆操作的错误上下文前缀。
const OP_CLONE_CONTEXT: &str = "克隆仓库失败";
/// 刷新操作的错误上下文前缀。
const OP_FETCH_CONTEXT: &str = "获取仓库更新失败";

/// 嗅探根因文本中的认证类失败（HTTP 401/403、凭据、权限）。仅做诊断增强（英文关键词
/// 匹配可能漏检纯数字形态的 "401"），不影响错误码与重试判定。
fn looks_like_auth_failure(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    [
        "unauthorized",
        "forbidden",
        "authentication",
        "credential",
        "permission denied",
        "access denied",
        "http status 401",
        "http status 403",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword))
}

/// 聚合 / 单次错误的上下文文案：认证类失败给出更可操作的提示。
fn aggregate_context(op_context: &str, detail: &str) -> String {
    if looks_like_auth_failure(detail) {
        format!("{op_context}：认证未通过或没有访问权限（请确认是公开仓库，或先在系统配置凭据）")
    } else {
        op_context.to_string()
    }
}

/// 把单次尝试的上游错误归一化（协议感知，[`is_retryable`] 依赖此处的错误码分类）：
/// - http/https 失败 → [`Error::Network`]（认证类失败增强文案）；
/// - file:// 或裸路径失败 → [`Error::NotFound`]（远程仓库不存在，或路径不是 git 仓库——
///   对调用方语义等价：拿不到这个仓库）；
/// - 其余协议（git://、ssh:// 等，一期不接入）→ [`Error::Internal`]。
fn normalize_transfer_error(op_context: &str, plan_url: &str, err: impl fmt::Display) -> Error {
    let detail = err.to_string();
    let lower = plan_url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Error::Network {
            context: aggregate_context(op_context, &detail),
            detail,
        }
    } else if lower.starts_with("file://") {
        Error::NotFound {
            path: PathBuf::from(plan_url.trim_start_matches("file://")),
        }
    } else if !lower.contains("://") {
        // 裸路径：gix 视作本地文件传输
        Error::NotFound {
            path: PathBuf::from(plan_url),
        }
    } else {
        Error::Internal {
            context: format!("{op_context}：暂不支持的仓库地址协议"),
            detail,
        }
    }
}

/// 超时归一化错误（整体超时被中断）。
fn timeout_error(op_context: &str, timeout: Duration) -> Error {
    Error::Network {
        context: op_context.to_string(),
        detail: format!("传输超时（超过 {} 秒，已请求中断）", timeout.as_secs()),
    }
}

// ---------- 代理与 HTTP 选项 ----------

/// 把中性 [`GixProxyOptions`] 转成 gix HTTP 传输选项（机制与语义见模块注释「代理注入」
/// 与「UA 注入缺口」）：通用字段照填；reqwest 后端经 backend 回调在 `ClientBuilder` 上
/// 实际装配代理、排除列表与连接超时；`Some("")` = 完全禁用代理的语义原样下传为
/// `ClientBuilder::no_proxy()`。
fn build_http_options(proxy: Option<&GixProxyOptions>, timeout: Duration) -> HttpOptions {
    let mut opts = HttpOptions::default();
    if let Some(proxy) = proxy {
        opts.proxy = proxy.proxy.clone();
        opts.no_proxy = proxy.no_proxy.clone();
    }
    opts.connect_timeout = Some(timeout);
    // 统一 UA 口径照填：reqwest 后端当前不消费该字段，实际头仍是 gix 的硬编码值（见模块头）
    opts.user_agent = Some(craft_core::network::APP_USER_AGENT.to_string());

    // reqwest 专属后端回调（gix 0.90 的 reqwest 后端不消费上面的通用字段）
    let mut backend = gix::protocol::transport::client::blocking_io::http::reqwest::Options::default();
    let proxy_owned = proxy.cloned();
    backend.configure_client = Some(Box::new(move |builder| {
        let builder = builder.connect_timeout(timeout);
        let Some(proxy) = proxy_owned.as_ref() else {
            return Ok(builder);
        };
        match proxy.proxy.as_deref() {
            // Some("")：完全禁用代理（连环境变量一并禁用，curl 语义）
            Some("") => Ok(builder.no_proxy()),
            // None：不干预（reqwest 默认自行读环境变量）
            None => Ok(builder),
            // Some(url)：显式代理 + 可选 NOPROXY 排除列表
            Some(url) => {
                let mut reqwest_proxy = reqwest::Proxy::all(url)
                    .map_err(|err| gix::error::message!("代理地址无效：{err}"))?;
                if let Some(list) = proxy.no_proxy.as_deref() {
                    reqwest_proxy = reqwest_proxy.no_proxy(reqwest::NoProxy::from_string(list));
                }
                Ok(builder.proxy(reqwest_proxy))
            }
        }
    }));
    opts.backend = Some(Arc::new(Mutex::new(backend)));
    opts
}

// ---------- 单次尝试（clone / fetch） ----------

/// 解析仓库 HEAD 短哈希（固定 7 位）。仓库无提交（空远程）时返回空串。
fn head_short_sha(repo: &gix::Repository) -> String {
    match repo.rev_parse_single("HEAD") {
        Ok(id) => id.to_string().chars().take(SHORT_SHA_LEN).collect(),
        // unborn HEAD（空仓库）
        Err(_) => String::new(),
    }
}

/// 从本次 ref advertisement 提取远程 HEAD 的短哈希；找不到（极少数服务端形态）返回 None，
/// 由调用方回退本地跟踪引用。
fn remote_head_short_sha(ref_map: &gix::protocol::fetch::RefMap) -> Option<String> {
    for mapping in &ref_map.mappings {
        if let gix::protocol::fetch::refmap::Source::Ref(handshake_ref) = &mapping.remote {
            let (name, target, peeled) = handshake_ref.unpack();
            if &**name == b"HEAD" {
                return peeled
                    .or(target)
                    .map(|oid| oid.to_string().chars().take(SHORT_SHA_LEN).collect());
            }
        }
    }
    None
}

/// 单次 clone 尝试（不带重试 / 回退，编排见 [`run_plans`]）。
fn attempt_clone(req: &CloneRequest, plan: &UrlPlan, interrupt: &AtomicBool) -> Result<CloneOutcome> {
    let depth = req.shallow_depth.unwrap_or(DEFAULT_SHALLOW_DEPTH);
    let depth = NonZeroU32::new(depth).ok_or_else(|| Error::Internal {
        context: "浅克隆深度必须大于 0".to_string(),
        detail: format!("shallow_depth = {depth}"),
    })?;

    // 隔离打开：克隆出的仓库不读 system/global 配置（含用户全局 http.proxy、凭据 helper），
    // 代理与访问凭据全部由本模块显式装配 / 默认匿名——商店下载场景语义确定。
    let mut prepare = gix::clone::PrepareFetch::new(
        plan.url.as_str(),
        &req.dest,
        gix::create::Kind::WithWorktree,
        gix::create::Options::default(),
        gix::open::Options::isolated(),
    )
    .map_err(|err| normalize_transfer_error(OP_CLONE_CONTEXT, &plan.url, &err))?;

    // 浅克隆 + 常规 tag + 单分支 refspec（对齐 git 浅克隆行为）由 gix 的 clone 管线在
    // with_shallow 生效时自动选择；此处只需声明深度。
    prepare = prepare
        .with_shallow(gix::remote::fetch::Shallow::DepthAtRemote(depth))
        .configure_connection({
            let http_opts = build_http_options(req.proxy.as_ref(), req.timeout);
            move |connection| {
                connection.set_transport_options(Box::new(http_opts.clone()));
                Ok(())
            }
        });

    let (mut checkout, _fetch_outcome) = prepare
        .fetch_then_checkout(gix::progress::Discard, interrupt)
        .map_err(|err| normalize_transfer_error(OP_CLONE_CONTEXT, &plan.url, &err))?;
    let (repo, _checkout_outcome) = checkout
        .main_worktree(gix::progress::Discard, interrupt)
        .map_err(|err| normalize_transfer_error(OP_CLONE_CONTEXT, &plan.url, &err))?;

    Ok(CloneOutcome {
        head_sha: head_short_sha(&repo),
        fell_back: false,
        url_used: plan.url.clone(),
    })
}

/// 单次 fetch 尝试（已有仓库刷新，经其 `origin` 远程）。
fn attempt_fetch(req: &FetchRequest, plan: &UrlPlan, interrupt: &AtomicBool) -> Result<FetchOutcome> {
    let mut repo = gix::open_opts(&req.repo_path, gix::open::Options::isolated()).map_err(|err| {
        Error::NotARepository {
            path: req.repo_path.clone(),
            detail: err.to_string(),
        }
    })?;
    // 隔离打开意味着无 system/global 身份；fetch 写 ref 事务（reflog）时需要 committer，
    // 缺失会让引用更新整体失败。与 gix clone 管线同款：缺失时内存兜底通用身份
    // （仅影响 reflog 记录，不写配置、不产生提交）。
    repo.committer_or_set_generic_fallback()
        .map_err(|err| Error::Internal {
            context: "读取仓库提交者身份失败".to_string(),
            detail: err.to_string(),
        })?;
    let remote = repo.find_remote("origin").map_err(|err| Error::Internal {
        context: format!("仓库 {} 未配置 origin 远程，无法刷新", req.repo_path.display()),
        detail: err.to_string(),
    })?;

    let mut connection = remote
        .connect(gix::remote::Direction::Fetch)
        .map_err(|err| normalize_transfer_error(OP_FETCH_CONTEXT, &plan.url, &err))?;
    connection.set_transport_options(Box::new(build_http_options(req.proxy.as_ref(), req.timeout)));
    let prepare = connection
        .prepare_fetch(gix::progress::Discard, gix::remote::ref_map::Options::default())
        .map_err(|err| normalize_transfer_error(OP_FETCH_CONTEXT, &plan.url, &err))?;
    let outcome = prepare
        .receive(gix::progress::Discard, interrupt)
        .map_err(|err| normalize_transfer_error(OP_FETCH_CONTEXT, &plan.url, &err))?;

    let updated = matches!(outcome.status, gix::remote::fetch::Status::Change { .. });
    // 远程当前位置：优先取本次 advertisement 中的 HEAD；回退本地 origin/HEAD
    // （clone 时 gix 写入的符号引用，fetch 更新跟踪分支后其指向随之推进）。
    let head_sha = remote_head_short_sha(&outcome.ref_map).unwrap_or_else(|| {
        repo.rev_parse_single("refs/remotes/origin/HEAD")
            .map(|id| id.to_string().chars().take(SHORT_SHA_LEN).collect())
            .unwrap_or_default()
    });
    Ok(FetchOutcome {
        head_sha,
        updated,
        fell_back: false,
        url_used: plan.url.clone(),
    })
}

// ---------- 测试 ----------

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造注入层的网络类失败（可重试）。
    fn network_err() -> Error {
        Error::Network {
            context: "测试网络错误".into(),
            detail: "connection refused".into(),
        }
    }

    /// 构造注入层的非网络类失败（不重试）。
    fn not_found_err() -> Error {
        Error::NotFound {
            path: PathBuf::from("/tmp/no-such"),
        }
    }

    /// 取出 backend 里的 reqwest 专属选项并执行 `f`（MutexGuard 需独立绑定以满足生命周期）。
    fn with_reqwest_backend<T>(
        backend: &Arc<Mutex<dyn std::any::Any + Send + Sync>>,
        f: impl FnOnce(
            &mut gix::protocol::transport::client::blocking_io::http::reqwest::Options,
        ) -> T,
    ) -> T {
        let mut guard = backend.lock().expect("锁不应毒化");
        let opts = guard
            .downcast_mut::<gix::protocol::transport::client::blocking_io::http::reqwest::Options>()
            .expect("backend 应为 reqwest 专属选项");
        f(opts)
    }

    // ---------- URL 计划构造 ----------

    #[test]
    fn build_url_plans_rewrites_github_host_and_appends_fallback() {
        let plans = build_url_plans(
            "https://github.com/owner/repo.git",
            Some("https://gh-proxy.com/"),
        );
        assert_eq!(
            plans,
            vec![
                UrlPlan {
                    url: "https://gh-proxy.com/https://github.com/owner/repo.git".into(),
                    rewritten: true
                },
                UrlPlan { url: "https://github.com/owner/repo.git".into(), rewritten: false },
            ],
            "github.com host 应生成改写计划 + 原始回退计划"
        );
    }

    #[test]
    fn build_url_plans_skips_rewrite_for_non_github_host() {
        let plans = build_url_plans("https://example.com/x.git", Some("https://gh-proxy.com/"));
        assert_eq!(
            plans,
            vec![UrlPlan { url: "https://example.com/x.git".into(), rewritten: false }],
            "非 github host 不改写、无回退计划"
        );
    }

    #[test]
    fn build_url_plans_skips_rewrite_for_empty_prefix() {
        let plans = build_url_plans("https://github.com/o/r", Some("  "));
        assert_eq!(
            plans,
            vec![UrlPlan { url: "https://github.com/o/r".into(), rewritten: false }],
            "空白前缀视同未开启加速"
        );
        let plans = build_url_plans("https://github.com/o/r", None);
        assert_eq!(plans.len(), 1);
        assert!(!plans[0].rewritten);
    }

    #[test]
    fn accelerator_rewrite_orthogonal_to_proxy_mode() {
        // spec D14：改写只换 URL 前缀，代理只进传输选项，二者正交——
        // 「无代理（Some("") 禁用一切代理）」模式下加速改写照常生效。
        let plans = build_url_plans("https://github.com/o/r", Some("https://gh-proxy.com/"));
        assert_eq!(plans.len(), 2);
        assert!(plans[0].rewritten, "none 模式下改写计划仍应生成");

        // 传输选项同时携带禁代理语义与改写无关：两侧互不影响
        let opts = build_http_options(
            Some(&GixProxyOptions { proxy: Some(String::new()), no_proxy: None }),
            DEFAULT_TIMEOUT,
        );
        assert_eq!(opts.proxy.as_deref(), Some(""));
    }

    // ---------- 代理选项转换 ----------

    #[test]
    fn proxy_options_pass_through_preserves_empty_string_semantics() {
        // Some("") = 完全禁用代理：通用字段与 backend 回调两侧语义都要保留
        let opts = build_http_options(
            Some(&GixProxyOptions { proxy: Some(String::new()), no_proxy: None }),
            Duration::from_secs(9),
        );
        assert_eq!(opts.proxy.as_deref(), Some(""));
        assert_eq!(opts.no_proxy, None);
        assert_eq!(opts.connect_timeout, Some(Duration::from_secs(9)));
        let backend = opts.backend.expect("reqwest 后端回调必须装配");
        with_reqwest_backend(&backend, |reqwest_opts| {
            let configure = reqwest_opts
                .configure_client
                .as_mut()
                .expect("configure_client 必须存在");
            let built =
                configure(reqwest::blocking::ClientBuilder::new()).expect("no_proxy 装配应成功");
            // ClientBuilder 无法直接断言内部代理状态；能构建成功即验证回调语义链路。
            let _ = built;
        });
    }

    /// 统一 UA 口径照填到通用字段：reqwest 后端当前不消费（实际头仍是 gix 的
    /// `git/oxide-<ver>`，见模块头「UA 注入缺口」），但字段必须在，未来上游消费即生效。
    /// 这条断言同时钉住「别为了让 UA 真的生效而改用 configure_request」——那会废掉重定向。
    #[test]
    fn http_options_carry_unified_user_agent() {
        let opts = build_http_options(None, DEFAULT_TIMEOUT);
        assert_eq!(opts.user_agent.as_deref(), Some(craft_core::network::APP_USER_AGENT));
        assert!(craft_core::network::APP_USER_AGENT.starts_with("CraftWork/"));
    }

    #[test]
    fn proxy_options_none_leaves_client_defaults_untouched() {
        let opts = build_http_options(None, Duration::from_secs(9));
        assert_eq!(opts.proxy, None, "未传代理时通用字段不应注入");
        let backend = opts.backend.expect("reqwest 后端回调必须装配");
        with_reqwest_backend(&backend, |reqwest_opts| {
            let configure = reqwest_opts.configure_client.as_mut().expect("回调必须存在");
            let _ = configure(reqwest::blocking::ClientBuilder::new())
                .expect("无代理配置时回调应直通成功");
        });
    }

    #[test]
    fn proxy_options_custom_url_and_no_proxy_are_assembled() {
        let opts = build_http_options(
            Some(&GixProxyOptions {
                proxy: Some("socks5://127.0.0.1:1080".into()),
                no_proxy: Some("localhost,127.0.0.0/8".into()),
            }),
            Duration::from_secs(1),
        );
        assert_eq!(opts.proxy.as_deref(), Some("socks5://127.0.0.1:1080"));
        let backend = opts.backend.expect("reqwest 后端回调必须装配");
        with_reqwest_backend(&backend, |reqwest_opts| {
            let configure = reqwest_opts.configure_client.as_mut().expect("回调必须存在");
            let _ = configure(reqwest::blocking::ClientBuilder::new())
                .expect("socks5 代理 + 排除列表应装配成功");
        });
    }

    // ---------- 重试与回退编排 ----------

    /// 断言便捷：跑编排并取（结果, 记录）。
    fn run_simple<T>(
        plans: &[UrlPlan],
        retries: u32,
        attempt: impl FnMut(&UrlPlan, &AtomicBool) -> Result<T>,
    ) -> Result<(T, bool, Vec<AttemptRecord>)> {
        run_plans(plans, retries, Duration::from_secs(1), Duration::ZERO, "测试操作", attempt)
    }

    #[test]
    fn network_failures_retry_up_to_limit_then_aggregate() {
        let plans = build_url_plans("https://example.com/r.git", None);
        let outcome = run_simple(&plans, 2, |_, _| Err::<CloneOutcome, _>(network_err()));
        assert!(outcome.is_err(), "全部失败应返回错误");
        let err = outcome.expect_err("已断言 is_err");
        assert_eq!(err.code(), "network");
        let text = err.to_string();
        // 重试计数：1 + 2 次尝试都应出现在聚合 detail 中
        assert!(text.contains("第 1 次"), "聚合应含第 1 次尝试：{text}");
        assert!(text.contains("第 3 次"), "聚合应含第 3 次（1 + retries）尝试：{text}");
        assert!(text.contains("https://example.com/r.git"), "聚合应含尝试 URL：{text}");
    }

    #[test]
    fn non_network_errors_fail_fast_without_retry() {
        let plans = build_url_plans("/tmp/local-source", None);
        let mut calls = 0usize;
        let outcome = run_simple(&plans, 3, |_, _| {
            calls += 1;
            Err::<CloneOutcome, _>(not_found_err())
        });
        let err = outcome.expect_err("NotFound 应直接失败");
        assert_eq!(err.code(), "repo_not_found", "应透传原始错误变体");
        assert_eq!(calls, 1, "非网络类失败不应重试");
    }

    #[test]
    fn fallback_tries_rewritten_then_original_and_marks_fell_back() {
        let plans = build_url_plans(
            "https://github.com/o/r.git",
            Some("https://unreachable.invalid/"),
        );
        let outcome = run_simple(&plans, 0, |plan, _| {
            if plan.rewritten {
                Err(network_err())
            } else {
                Ok(CloneOutcome {
                    head_sha: "abc1234".into(),
                    fell_back: false,
                    url_used: plan.url.clone(),
                })
            }
        });
        let (value, fell_back, records) = outcome.expect("回退后应成功");
        assert!(fell_back, "经回退成功必须标注 fell_back");
        assert_eq!(value.url_used, "https://github.com/o/r.git", "最终 URL 应为原始地址");
        assert_eq!(
            records
                .iter()
                .map(|r| (r.url.as_str(), r.rewritten, r.retry_index))
                .collect::<Vec<_>>(),
            vec![
                ("https://unreachable.invalid/https://github.com/o/r.git", true, 0),
                ("https://github.com/o/r.git", false, 0),
            ],
            "尝试序列应为改写一次 → 原始一次"
        );
    }

    #[test]
    fn fallback_plan_exhausts_retries_before_falling_back() {
        let plans = build_url_plans("https://github.com/o/r", Some("https://p.invalid/"));
        let outcome = run_simple(&plans, 1, |plan, _| {
            if plan.rewritten {
                Err(network_err())
            } else {
                Ok(CloneOutcome {
                    head_sha: "abc1234".into(),
                    fell_back: false,
                    url_used: plan.url.clone(),
                })
            }
        });
        let (_, fell_back, records) = outcome.expect("回退后应成功");
        assert!(fell_back);
        assert_eq!(records.len(), 3, "改写计划 1 + 重试 1 次，再回退原始 1 次");
        assert!(records[0].rewritten && records[1].rewritten, "前两次应落在改写计划");
        assert!(!records[2].rewritten);
    }

    #[test]
    fn exhausted_fallback_failure_aggregates_all_attempted_urls() {
        let plans = build_url_plans("https://github.com/o/r", Some("https://p.invalid/"));
        let outcome = run_simple(&plans, 0, |_, _| Err::<CloneOutcome, _>(network_err()));
        let err = outcome.expect_err("全部失败应返回错误");
        let text = err.to_string();
        assert!(
            text.contains("https://p.invalid/https://github.com/o/r")
                && text.contains("https://github.com/o/r"),
            "聚合错误应同时含改写与原始 URL（证明回退被尝试）：{text}"
        );
    }

    #[test]
    fn timeout_failure_is_retryable_and_aggregated() {
        // 注入层无法直接产出 Timeout（由 run_with_timeout 生成）；这里验证网络类重试的
        // 计数路径（Timeout 与 Network 走同一分支，见 is_retryable）。
        let plans = build_url_plans("https://example.com/r.git", None);
        let outcome = run_simple(&plans, 1, |_, _| Err::<CloneOutcome, _>(network_err()));
        let err = outcome.expect_err("已断言失败");
        assert_eq!(err.code(), "network");
        assert!(err.to_string().contains("第 2 次"), "1 + 1 次尝试应被聚合：{err}");
    }

    // ---------- 超时机制 ----------

    #[test]
    fn timeout_thread_triggers_interrupt_and_downgrades_ok_to_timeout() {
        let started = std::time::Instant::now();
        let outcome: std::result::Result<String, AttemptFailure> = run_with_timeout(
            Duration::from_millis(50),
            |interrupt| {
                // 模拟一个「卡住但最终正常返回」的传输：中断标志已置位后仍返回 Ok——
                // 验证「成功结果在中断后也按超时降级」的分支。
                std::thread::sleep(Duration::from_millis(150));
                assert!(
                    interrupt.load(Ordering::SeqCst),
                    "150ms 后中断标志应已被 50ms 定时线程置位"
                );
                Ok("partial".to_string())
            },
        );
        match outcome {
            Err(AttemptFailure::Timeout) => {}
            other => panic!("中断后应返回 Timeout，实际：{other:?}"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "定时线程应按 50ms 触发而不是等满操作时长"
        );
    }

    #[test]
    fn timeout_thread_exits_early_when_operation_finishes() {
        let started = std::time::Instant::now();
        let outcome: std::result::Result<u8, AttemptFailure> = run_with_timeout(
            Duration::from_secs(30),
            |_interrupt| Ok(7u8),
        );
        assert_eq!(outcome.expect("及时完成应成功"), 7);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "操作完成后定时线程应被信号唤醒提前退出，而不是等满 30 秒时限"
        );
    }

    #[test]
    fn upstream_error_propagates_without_timeout_interference() {
        let outcome: std::result::Result<u8, AttemptFailure> =
            run_with_timeout(Duration::from_secs(5), |_| Err(not_found_err()));
        match outcome {
            Err(AttemptFailure::Upstream(err)) => assert_eq!(err.code(), "repo_not_found"),
            other => panic!("上游错误应原样透传，实际：{other:?}"),
        }
    }

    // ---------- 错误归一化 ----------

    #[test]
    fn normalization_maps_protocol_to_error_variants() {
        // http → Network
        let err = normalize_transfer_error(OP_CLONE_CONTEXT, "https://example.com/r", "boom");
        assert_eq!(err.code(), "network");
        // file:// → NotFound（去前缀）
        let err = normalize_transfer_error(OP_CLONE_CONTEXT, "file:///tmp/missing", "boom");
        match &err {
            Error::NotFound { path } => {
                assert_eq!(path, &PathBuf::from("/tmp/missing"), "file:// 前缀应剥去");
            }
            other => panic!("file:// 失败应归一化为 NotFound，实际：{other:?}"),
        }
        // 裸路径 → NotFound（保留路径）
        let err = normalize_transfer_error(OP_CLONE_CONTEXT, "/tmp/missing", "boom");
        match &err {
            Error::NotFound { path } => assert_eq!(path, &PathBuf::from("/tmp/missing")),
            other => panic!("裸路径失败应归一化为 NotFound，实际：{other:?}"),
        }
        // 其他协议 → Internal
        let err = normalize_transfer_error(OP_CLONE_CONTEXT, "ssh://example.com/r", "boom");
        assert_eq!(err.code(), "internal");
    }

    #[test]
    fn auth_failures_get_actionable_context() {
        let err = normalize_transfer_error(
            OP_CLONE_CONTEXT,
            "https://example.com/r",
            "Received HTTP status 401",
        );
        let text = err.to_string();
        assert!(
            text.contains("认证未通过或没有访问权限"),
            "认证类失败应有可操作提示：{text}"
        );
        // 非认证类保持简洁上下文
        let err = normalize_transfer_error(OP_CLONE_CONTEXT, "https://example.com/r", "connect refused");
        assert!(!err.to_string().contains("认证"), "非认证失败不应误报认证提示");
        assert!(looks_like_auth_failure("Permission Denied (publickey)"));
        assert!(!looks_like_auth_failure("connection refused"));
    }
}
