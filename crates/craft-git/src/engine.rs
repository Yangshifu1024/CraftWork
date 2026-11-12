//! 仓库发现与打开、工作树 / 快照（bare）区分、config 快照读取（spec 第 4 节 engine.rs）。
//!
//! 本模块是 craft-git 门面的入口层：后续 status / log / diff / commit / worktree /
//! transport 模块都从 [`Repository`] 出发。config 读取走 gix 的快照语义——
//! `gix::open` / `gix::discover` 在打开仓库时一次性合并 system → global → local
//! （→ worktree）配置，与 `git config` 的级联顺序一致；后续调用不再触碰文件系统。

use std::path::Path;

use crate::errors::{Error, Result};

/// 仓库形态：工作树 / 快照（bare）/ linked 工作树 / 子仓库。
///
/// 后续票据此分流行为：M6 worktree 操作只允许在带工作树的仓库上发起；
/// 快照（bare）仓库可读（log / diff）但不可执行工作树操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryKind {
    /// 普通工作树仓库：`.git` 目录 + 工作树。
    Worktree,
    /// 快照（bare）仓库：只有对象库与引用，无工作树。
    Bare,
    /// linked 工作树：`git worktree add` 创建的附属工作树（M6 使用）。
    LinkedWorkTree,
    /// 子仓库：submodule 的检出目录。
    Submodule,
}

/// git 身份：级联读取 `user.name` / `user.email` 后的结果。
///
/// 供 M5 提交管线（author / committer 签名）与迁移 `craft-core::user_display_name`
/// 使用。字符串保持配置文件中的原样（不做 trim），与 `git config` 输出一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// `user.name` 的值。
    pub name: String,
    /// `user.email` 的值。
    pub email: String,
}

/// git 仓库句柄：`gix::Repository` 的门面包装。
///
/// gix 类型不出现在 craft-git 的公共 API 签名里（消费方不接触 gix，
/// spec 第 4 节）；内部字段私有，操作以后续各模块的方法形式提供。
#[derive(Debug, Clone)]
pub struct Repository {
    inner: gix::Repository,
}

impl Repository {
    /// 从 `path` 向上查找最近的一个 git 仓库并打开。
    ///
    /// 语义对齐 `gix::discover`（即 `git rev-parse` 家族的向上查找）：从 `path`
    /// （或其所在工作树子目录）逐级向上直到发现 `.git`；支持 bare 仓库目录本身、
    /// `.git` 文件（worktree / submodule）等 git 认可的形态。`path` 不存在时返回
    /// [`Error::NotFound`]；找到根目录仍无仓库时返回 [`Error::NotARepository`]。
    pub fn discover(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(Error::NotFound { path: path.to_path_buf() });
        }
        gix::discover(path)
            .map(Self::from_gix)
            .map_err(|err| Error::NotARepository {
                path: path.to_path_buf(),
                detail: err.to_string(),
            })
    }

    /// 严格打开 `path` 处的仓库：`path` 必须本身是工作树根、`.git` 目录或 bare 仓库根，
    /// **不**向上查找。
    ///
    /// 供「明确知道仓库位置」的调用方使用（如 D9 worktree 服务直接持有目录路径）；
    /// 交互场景（用户任意选择子目录）应使用 [`Repository::discover`]。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(Error::NotFound { path: path.to_path_buf() });
        }
        gix::open(path)
            .map(Self::from_gix)
            .map_err(|err| Error::NotARepository {
                path: path.to_path_buf(),
                detail: err.to_string(),
            })
    }

    /// 隔离模式的发现打开：[`Repository::discover`] 语义 + 不读 system / global 配置。
    ///
    /// 供黄金对比测试使用——测试进程跑在用户机器上，普通 `discover` 会把宿主机
    /// 全局配置（core.quotepath、core.abbrev、status.showUntrackedFiles 等）并进
    /// config 快照，导致与「完全隔离环境的系统 git 基准」两侧输入不对称；隔离
    /// 打开后两侧都只见 fixture 仓库自身的 local 配置，对比才成立。生产路径
    /// （工具 / 摘要 / 侧栏）保持读全局，与 git 语义一致，不要使用本方法。
    pub fn discover_isolated(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(Error::NotFound { path: path.to_path_buf() });
        }
        gix::discover_opts(path, Default::default(), gix::open::Options::isolated())
            .map(Self::from_gix)
            .map_err(|err| Error::NotARepository {
                path: path.to_path_buf(),
                detail: err.to_string(),
            })
    }

    /// 隔离模式的严格打开：[`Repository::open`] 语义 + 不读 system / global 配置。
    ///
    /// 用途同 [`Repository::discover_isolated`]。
    pub fn open_isolated(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(Error::NotFound { path: path.to_path_buf() });
        }
        gix::open_opts(path, gix::open::Options::isolated())
            .map(Self::from_gix)
            .map_err(|err| Error::NotARepository {
                path: path.to_path_buf(),
                detail: err.to_string(),
            })
    }

    /// 当前仓库形态（工作树 / bare / linked 工作树 / 子仓库）。
    ///
    /// gix 的 [`gix::repository::Kind`] 不单列 bare，需结合 `core.bare` 判定：
    /// `Common` 且 bare 归为 [`RepositoryKind::Bare`]。
    pub fn kind(&self) -> RepositoryKind {
        match self.inner.kind() {
            gix::repository::Kind::Submodule => RepositoryKind::Submodule,
            gix::repository::Kind::LinkedWorkTree => RepositoryKind::LinkedWorkTree,
            gix::repository::Kind::Common => {
                if self.inner.is_bare() {
                    RepositoryKind::Bare
                } else {
                    RepositoryKind::Worktree
                }
            }
        }
    }

    /// 工作树根目录；bare 仓库返回 `None`。
    pub fn workdir(&self) -> Option<&Path> {
        self.inner.workdir()
    }

    /// HEAD 符号指向的分支短名（`refs/heads/master` → `master`）。
    ///
    /// unborn HEAD（仓库刚 init）同样返回分支名——符号引用存在、只是目标分支尚未
    /// 诞生（等价 `git symbolic-ref --short -q HEAD` 在 unborn 下的成功输出）；
    /// 分离 HEAD（detached）与 HEAD 读取失败返回 `None`（对应 `-q` 的静默失败）。
    pub fn head_branch_name(&self) -> Option<String> {
        match self.inner.head().ok()?.kind {
            gix::head::Kind::Unborn(name) => Some(String::from_utf8_lossy(name.shorten()).into_owned()),
            gix::head::Kind::Symbolic(reference) => {
                Some(String::from_utf8_lossy(reference.name.shorten()).into_owned())
            }
            gix::head::Kind::Detached { .. } => None,
        }
    }

    /// 私有 git 目录（普通仓库为 `<root>/.git`，linked worktree 为其私有 git 目录）。
    pub fn git_dir(&self) -> &Path {
        self.inner.git_dir()
    }

    /// 公共 git 目录（普通仓库与 `<root>/.git` 相同；linked worktree 指向主仓库的
    /// `.git`）。worktree 管理目录（`.git/worktrees/*`）与共享引用都在这里，prune
    /// 等管理类操作的扫描根。
    pub fn common_dir(&self) -> &Path {
        self.inner.common_dir()
    }

    /// 级联读取 git 身份（`user.name` + `user.email`）。
    ///
    /// 级联来源与 git 一致：system → global → local（→ worktree），由 gix 打开时的
    /// config 快照保证。任一项缺失即返回 [`Error::IdentityMissing`]，并在变体里标明
    /// 缺的是哪一项（对齐 spec 第 3 节「身份语义保留」：M5 提交前的校验入口）。
    pub fn identity(&self) -> Result<Signature> {
        let snapshot = self.inner.config_snapshot();
        signature_from_config_parts(config_string(&snapshot, "user.name"), config_string(&snapshot, "user.email"))
    }

    /// 按完整级联读取单个配置项（点分隔键名，如 `"user.name"`）。
    ///
    /// 取「级联中最后一次定义」的值（git 语义：越靠近仓库的配置越优先）。
    /// 键名非法或未配置时返回 `None`。
    pub fn config_string(&self, key: &str) -> Option<String> {
        config_string(&self.inner.config_snapshot(), key)
    }

    /// 从 gix 仓库句柄构造门面（crate 内部各模块经 [`Self::inner`] 访问引擎）。
    pub(crate) fn from_gix(inner: gix::Repository) -> Self {
        Self { inner }
    }

    /// crate 内部访问 gix 引擎句柄（status / log / diff / commit 各模块共用）。
    pub(crate) fn inner(&self) -> &gix::Repository {
        &self.inner
    }
}

/// 从「已按级联解析出的 name / email 两个可选值」组装身份或缺失错误。
///
/// 拆成纯函数便于单元测试覆盖全部缺失组合，不依赖真实仓库。
fn signature_from_config_parts(name: Option<String>, email: Option<String>) -> Result<Signature> {
    match (name, email) {
        (Some(name), Some(email)) => Ok(Signature { name, email }),
        (name, email) => Err(Error::IdentityMissing {
            name_missing: name.is_none(),
            email_missing: email.is_none(),
        }),
    }
}

/// 从 config 快照读取字符串值；键非法按未配置处理（返回 `None`）。
///
/// 非 UTF-8 字节按 lossy 转换（替换非法序列），与 `git config` 终端输出的宽容语义一致。
fn config_string(snapshot: &gix::config::Snapshot<'_>, key: &str) -> Option<String> {
    snapshot
        .string(key)
        .map(|value| String::from_utf8_lossy(&value).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_missing_reported_when_no_config_source_has_user_identity() {
        // 手写最小 git 仓库结构（不经任何进程）：HEAD + objects/ + refs/ + 本地 config（无 user 节）。
        // 再以 `Options::isolated()` 打开（不读 system/global 配置），即可在不受宿主机
        // 环境影响的前提下验证「全级联无身份 → IdentityMissing」。
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let git_dir = root.path().join(".git");
        std::fs::create_dir_all(git_dir.join("objects")).expect("创建 objects 失败");
        std::fs::create_dir_all(git_dir.join("refs/heads")).expect("创建 refs 失败");
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/master\n").expect("写 HEAD 失败");
        std::fs::write(
            git_dir.join("config"),
            "[core]\n\trepositoryformatversion = 0\n\tbare = false\n",
        )
        .expect("写 config 失败");

        let repo = gix::open_opts(
            root.path(),
            gix::open::Options::isolated(),
        )
        .expect("打开最小仓库应成功");
        let facade = Repository::from_gix(repo);

        match facade.identity() {
            Err(Error::IdentityMissing {
                name_missing,
                email_missing,
            }) => {
                assert!(name_missing && email_missing, "隔离打开且无本地身份时两侧都应缺失");
            }
            other => panic!("隔离打开无身份仓库应返回 IdentityMissing，实际：{other:?}"),
        }
    }

    #[test]
    fn identity_assembles_when_both_parts_present() {
        let identity = signature_from_config_parts(Some("张三".into()), Some("zhang@example.com".into()))
            .expect("both parts present must succeed");
        assert_eq!(identity.name, "张三");
        assert_eq!(identity.email, "zhang@example.com");
    }

    #[test]
    fn identity_reports_which_parts_are_missing() {
        // 全缺：两个布尔位都为 true。
        let err = signature_from_config_parts(None, None).expect_err("missing both must fail");
        let Error::IdentityMissing {
            name_missing,
            email_missing,
        } = err
        else {
            panic!("expected IdentityMissing, got {err:?}");
        };
        assert!(name_missing && email_missing);

        // 只缺 name。
        let err = signature_from_config_parts(None, Some("e@example.com".into()))
            .expect_err("missing name must fail");
        let Error::IdentityMissing {
            name_missing,
            email_missing,
        } = err
        else {
            panic!("expected IdentityMissing, got {err:?}");
        };
        assert!(name_missing && !email_missing);

        // 只缺 email。
        let err = signature_from_config_parts(Some("n".into()), None)
            .expect_err("missing email must fail");
        let Error::IdentityMissing {
            name_missing,
            email_missing,
        } = err
        else {
            panic!("expected IdentityMissing, got {err:?}");
        };
        assert!(!name_missing && email_missing);
    }
}
