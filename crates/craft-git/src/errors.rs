//! 错误归一化（spec 第 4 节）：craft-git 的统一错误类型。
//!
//! 原则：**不向调用方外泄 gix 原始错误形态**——gix 错误只以「上下文 + 根因文本」的形式
//! 收敛进 `Error::Internal` / `Error::Network` 的 `detail` 字段；调用方（craft-tools /
//! craft-agent / src-tauri / craft-marketplace）拿到的是稳定的枚举变体与可读中文文案。
//! `Display` 输出即渲染层 toast 直接展示的字符串（tauri-bridge 的 invoke 拒绝即错误字符串）。

use std::fmt;
use std::path::PathBuf;

/// craft-git 统一错误。
///
/// 变体设计对应 spec 第 4 节的错误归一化清单：仓库不存在 / 非 git 目录 / 无身份配置 /
/// 网络类（M2 transport 预留）/ 内部错误。
#[derive(Debug)]
pub enum Error {
    /// 目标路径不存在（打开或发现仓库之前即可判定）。
    NotFound {
        /// 不存在的路径。
        path: PathBuf,
    },
    /// 路径存在，但它（连同其全部祖先目录）不是 git 仓库。
    NotARepository {
        /// 从该路径开始向上查找仓库失败。
        path: PathBuf,
        /// 根因文本（gix 错误链的字符串形式，仅用于 `Debug` 诊断；`Display` 主文案保持简洁）。
        detail: String,
    },
    /// git 身份未配置：`user.name` / `user.email` 在 system / global / local 任一级都未找到。
    ///
    /// 对齐 spec 第 3 节「身份语义保留」：M5 提交前经 [`crate::engine::Repository::identity`]
    /// 校验，缺失时把两个布尔位透出，便于提示用户补配哪一项。
    IdentityMissing {
        /// `user.name` 是否缺失。
        name_missing: bool,
        /// `user.email` 是否缺失。
        email_missing: bool,
    },
    /// HEAD 尚未诞生（unborn）：仓库刚 init、还没有任何提交。
    ///
    /// log 类操作在此状态下与系统 git 一样无法进行；`Display` 文案逐字对齐
    /// 系统 git 的 fatal 输出（`fatal: your current branch 'x' does not have any
    /// commits yet`），保证工具层错误文本与迁移前 1:1。
    UnbornHead {
        /// 当前分支的短名（如 `master`）。
        branch: String,
    },
    /// 没有任何可提交的变更（git 拒绝空提交）。
    ///
    /// `message` 是与系统 git 空提交拒绝输出同构的完整文本（首行 `On branch …` /
    /// `HEAD detached at …`，末行 `nothing to commit…`），`Display` 原样透出——
    /// 工具层「提交失败: 」前缀拼接后与迁移前系统 git 的 stderr 呈现一致。
    /// 注：git 在 ahead/behind 等场景还会附提示行，此处不复刻（票 #5 偏差清单）。
    NothingToCommit {
        /// 与 git 同构的完整拒绝文本。
        message: String,
    },
    /// 提交指定的路径在工作树与 index 中都不存在（等价 `git add -- <path>` 的 fatal）。
    ///
    /// `Display` 逐字对齐系统 git：`fatal: pathspec '<p>' did not match any files`。
    PathspecNotFound {
        /// 找不到的路径（第一个未命中的 pathspec，与 git 报第一个一致）。
        pathspec: String,
    },
    /// 网络类错误（clone / fetch，M2 transport 接入后启用；本票仅占位）。
    ///
    /// 上游 gix / TLS 错误不外泄原始形态，只保留归一化后的中文上下文与根因文本。
    Network {
        /// 发生了什么的中文上下文，例如「克隆仓库失败」。
        context: String,
        /// 根因文本（gix 错误链的字符串形式，仅用于诊断展示）。
        detail: String,
    },
    /// 其余内部错误：仓库对象库损坏、配置读写失败等。
    ///
    /// `context` 说明当时正在做什么（中文），`detail` 保存上游错误的根因文本
    /// （不保留 gix 错误类型本身）。
    Internal {
        /// 正在进行的操作的中文上下文。
        context: String,
        /// 根因文本（gix 错误链的字符串形式，仅用于诊断展示）。
        detail: String,
    },
}

/// craft-git 的统一 Result 别名。
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// 稳定的错误码：渲染层或调用方可据此做程序化分支，不必解析中文文案。
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotFound { .. } => "repo_not_found",
            Error::NotARepository { .. } => "not_a_repository",
            Error::IdentityMissing { .. } => "identity_missing",
            Error::UnbornHead { .. } => "unborn_head",
            Error::NothingToCommit { .. } => "nothing_to_commit",
            Error::PathspecNotFound { .. } => "pathspec_not_found",
            Error::Network { .. } => "network",
            Error::Internal { .. } => "internal",
        }
    }

    /// 把任意外部错误（gix 等）收敛成带中文上下文的 `Error::Internal`。
    ///
    /// 只保留根因的字符串形式，不保留错误类型——满足「不外泄 gix 原始形态」。
    pub fn internal(context: impl Into<String>, source: impl fmt::Display) -> Self {
        Error::Internal {
            context: context.into(),
            detail: source.to_string(),
        }
    }

    /// 把任意外部错误收敛成 `Error::Network`（M2 transport 使用）。
    pub fn network(context: impl Into<String>, source: impl fmt::Display) -> Self {
        Error::Network {
            context: context.into(),
            detail: source.to_string(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound { path } => {
                write!(f, "仓库不存在：{}。请确认路径是否正确", path.display())
            }
            Error::NotARepository { path, .. } => {
                write!(
                    f,
                    "目录不是 git 仓库：{}（已向上查找至根目录仍未发现 .git）",
                    path.display()
                )
            }
            Error::IdentityMissing {
                name_missing,
                email_missing,
            } => {
                let missing = match (name_missing, email_missing) {
                    (true, true) => "user.name 与 user.email".to_string(),
                    (true, false) => "user.name".to_string(),
                    (false, true) => "user.email".to_string(),
                    // 两个都配置了却构造出此变体属于逻辑漏洞，兜底文案仍可读。
                    (false, false) => "身份信息".to_string(),
                };
                write!(
                    f,
                    "git 身份未配置：缺少 {missing}。请先在 git 配置中设置用户名与邮箱（git config user.name / user.email）后再提交"
                )
            }
            Error::UnbornHead { branch } => {
                // 与系统 git log 在 unborn HEAD 下的 stderr 输出逐字一致（工具层错误文本不因迁移改变）。
                write!(f, "fatal: your current branch '{branch}' does not have any commits yet")
            }
            Error::NothingToCommit { message } => {
                write!(f, "{message}")
            }
            Error::PathspecNotFound { pathspec } => {
                write!(f, "fatal: pathspec '{pathspec}' did not match any files")
            }
            Error::Network { context, detail } => {
                write!(f, "{context}：{detail}")
            }
            Error::Internal { context, detail } => {
                write!(f, "{context}：{detail}")
            }
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_are_actionable_chinese() {
        let not_found = Error::NotFound {
            path: PathBuf::from("/tmp/missing"),
        };
        assert_eq!(
            not_found.to_string(),
            "仓库不存在：/tmp/missing。请确认路径是否正确"
        );

        let not_repo = Error::NotARepository {
            path: PathBuf::from("/tmp/plain"),
            detail: "upwards failed".into(),
        };
        assert!(not_repo.to_string().contains("目录不是 git 仓库"));

        let internal = Error::internal("读取对象库失败", "object corrupted");
        assert_eq!(internal.to_string(), "读取对象库失败：object corrupted");
    }

    #[test]
    fn unborn_head_message_matches_system_git_fatal_output() {
        let err = Error::UnbornHead { branch: "master".into() };
        assert_eq!(
            err.to_string(),
            "fatal: your current branch 'master' does not have any commits yet"
        );
    }

    #[test]
    fn identity_missing_message_lists_missing_fields() {
        let both = Error::IdentityMissing {
            name_missing: true,
            email_missing: true,
        };
        assert!(both.to_string().contains("user.name 与 user.email"));

        let name_only = Error::IdentityMissing {
            name_missing: true,
            email_missing: false,
        };
        let text = name_only.to_string();
        assert!(text.contains("缺少 user.name。"), "应明确指出缺失项：{text}");
        // 尾部引导语允许提及 email（配置指引），但「缺少」清单只应有 name。
        assert!(!text.contains("缺少 user.email"), "缺失清单不应包含已配置的 email：{text}");

        let email_only = Error::IdentityMissing {
            name_missing: false,
            email_missing: true,
        };
        assert!(email_only.to_string().contains("缺少 user.email。"));
    }

    #[test]
    fn codes_are_stable_identifiers() {
        assert_eq!(Error::NotFound { path: PathBuf::new() }.code(), "repo_not_found");
        assert_eq!(
            Error::NotARepository {
                path: PathBuf::new(),
                detail: String::new()
            }
            .code(),
            "not_a_repository"
        );
        assert_eq!(
            Error::IdentityMissing {
                name_missing: true,
                email_missing: true
            }
            .code(),
            "identity_missing"
        );
        assert_eq!(
            Error::UnbornHead { branch: "master".into() }.code(),
            "unborn_head"
        );
        assert_eq!(
            Error::NothingToCommit { message: "x".into() }.code(),
            "nothing_to_commit"
        );
        assert_eq!(Error::PathspecNotFound { pathspec: "p".into() }.code(), "pathspec_not_found");
        assert_eq!(
            Error::Network {
                context: "x".into(),
                detail: "y".into()
            }
            .code(),
            "network"
        );
        assert_eq!(
            Error::Internal {
                context: "x".into(),
                detail: "y".into()
            }
            .code(),
            "internal"
        );
    }
}
