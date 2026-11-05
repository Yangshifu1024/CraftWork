//! 路径安全解析。
//! 核心不变量：词法校验拒绝 ../ 越界 + 真实路径（符号链接解析后）二次校验拒绝链接逃逸。

use std::path::{Component, Path, PathBuf, MAIN_SEPARATOR};

/// 路径超出工作区及附加目录。
#[derive(Debug, Clone)]
pub struct ErrOutsideWorkspace;

impl std::fmt::Display for ErrOutsideWorkspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "路径超出工作区及附加目录")
    }
}

impl std::error::Error for ErrOutsideWorkspace {}

pub type ResolveResult<T> = Result<T, ErrOutsideWorkspace>;

#[derive(Debug, Clone)]
pub struct Resolved {
    pub root_index: usize,
    pub root: String,
    pub abs: String,
    pub rel: String,
}

/// 词法规范形式（不触盘）：去 `.` 与 `..`、折叠分隔符。绝对路径保持绝对。
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // 越过根的 .. 无效果（与 Node resolve 一致）
                let poppable = out
                    .components()
                    .next_back()
                    .is_some_and(|c| matches!(c, Component::Normal(_) | Component::Prefix(_)));
                if poppable {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn eval_symlinks(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// 目标是否在 root 内：相等，或前缀为 root + 分隔符（防止 /tmp/work 误吞 /tmp/work-other）。
fn within(root: &Path, candidate: &Path) -> bool {
    if candidate == root {
        return true;
    }
    candidate.starts_with(format!("{}{}", root.display(), MAIN_SEPARATOR))
}

/// 单根解析（resolve.ts: resolveAt 直译）：
/// 1. 相对路径拼接 lexRoot（保留符号链接别名）后取词法规范形式；绝对路径保持原样
/// 2. 词法校验（目标不存在也不接受越界）
/// 3. 符号链接解析：目标不存在时解析父目录再拼回文件名（新建文件场景）
/// 4. 真实路径校验（realRoot = 解析后的根，如 macOS /var vs /private/var）
pub fn resolve_at(lex_root: &str, rel: &str) -> ResolveResult<(String, String)> {
    let root_path = Path::new(lex_root);
    let rel_path = Path::new(rel);
    let candidate = if rel_path.is_absolute() {
        lexical_normalize(rel_path)
    } else {
        lexical_normalize(&root_path.join(rel_path))
    };
    let candidate_str = candidate.to_string_lossy().to_string();
    if candidate_str.starts_with("\\\\") || candidate_str.starts_with("//") {
        return Err(ErrOutsideWorkspace);
    }
    if !within(root_path, &candidate) {
        return Err(ErrOutsideWorkspace);
    }

    let real = match eval_symlinks(&candidate) {
        Some(real) => real,
        None => {
            // 目标不存在：解析父目录的真实路径再拼回文件名
            let parent = candidate
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(MAIN_SEPARATOR.to_string()));
            let base = candidate
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let real_parent = eval_symlinks(&parent).ok_or(ErrOutsideWorkspace)?;
            real_parent.join(base)
        }
    };
    let real_root = eval_symlinks(root_path).unwrap_or_else(|| root_path.to_path_buf());
    if !within(&real_root, &real) {
        return Err(ErrOutsideWorkspace);
    }
    let rel_out = if real == real_root {
        ".".to_string()
    } else {
        real
            .to_string_lossy()
            .strip_prefix(&format!("{}{}", real_root.display(), MAIN_SEPARATOR))
            .unwrap_or(&real.to_string_lossy())
            .to_string()
    };
    Ok((real.to_string_lossy().to_string(), rel_out))
}

/// 词法校验拼接（不触盘）：新建文件（父目录缺失）场景由调用方负责建目录。
/// 安全语义与 resolve_at 的词法段一致：绝对路径归一、UNC 拒绝、越界拒绝。
pub fn lexical_at(root: &str, rel: &str) -> ResolveResult<(String, String)> {
    let root_path = Path::new(root);
    let rel_path = Path::new(rel);
    let candidate = if rel_path.is_absolute() {
        lexical_normalize(rel_path)
    } else {
        lexical_normalize(&root_path.join(rel_path))
    };
    let candidate_str = candidate.to_string_lossy().to_string();
    if candidate_str.starts_with("\\") || candidate_str.starts_with("//") {
        return Err(ErrOutsideWorkspace);
    }
    if !within(root_path, &candidate) {
        return Err(ErrOutsideWorkspace);
    }
    let rel_out = if candidate == root_path {
        ".".to_string()
    } else {
        candidate
            .to_string_lossy()
            .strip_prefix(format!("{}{}", root, MAIN_SEPARATOR).as_str())
            .map(str::to_string)
            .unwrap_or_else(|| candidate_str.clone())
    };
    Ok((candidate_str, rel_out))
}

fn root_candidates(roots: &[String]) -> Vec<String> {
    roots.iter().filter(|r| !r.is_empty()).cloned().collect()
}

/// 多根解析（roots[0] = 主要目录，resolve.ts: resolveAny 直译）：
/// - 主要目录不可用仍报错；附加目录不可达则跳过
/// - prefer_existing=true 时词法命中先不返回，确认真实存在才命中；
///   全程无真实存在则回落第一个词法命中（新建文件是合法场景）
/// - rel 为空串返回第一个可用根本身
pub fn resolve_any(roots: &[String], rel: &str, prefer_existing: bool) -> ResolveResult<Resolved> {
    let candidates = root_candidates(roots);
    if candidates.is_empty() {
        return Err(ErrOutsideWorkspace);
    }
    let clean_rel = rel.trim();
    let mut first_lexical: Option<Resolved> = None;

    for (i, root) in candidates.iter().enumerate() {
        let target = match clean_rel.is_empty() {
            true => root.clone(),
            false => match resolve_at(root, clean_rel) {
                Ok((abs, _)) => abs,
                Err(_) => continue, // 越界：尝试下一根
            },
        };
        if prefer_existing {
            let lexical = Resolved {
                root_index: i,
                root: root.clone(),
                abs: target.clone(),
                rel: if clean_rel.is_empty() { ".".to_string() } else { clean_rel.to_string() },
            };
            if Path::new(&target).exists() {
                return Ok(lexical);
            }
            if first_lexical.is_none() {
                first_lexical = Some(lexical);
            }
            continue;
        }
        let (abs, rel_out) = resolve_at(root, clean_rel)?;
        return Ok(Resolved {
            root_index: i,
            root: root.clone(),
            abs,
            rel: rel_out,
        });
    }
    first_lexical.ok_or(ErrOutsideWorkspace)
}

/// 多根解析，优先命中真实存在的目标。
pub fn resolve_existing(roots: &[String], rel: &str) -> ResolveResult<Resolved> {
    resolve_any(roots, rel, true)
}

/// 展示路径：相对主要根；越界原样返回。
pub fn relative(workspace: &str, path: &str) -> String {
    let abs = lexical_normalize(Path::new(path));
    let root = lexical_normalize(Path::new(workspace));
    if abs == root {
        return ".".into();
    }
    let prefix = format!("{}{}", root.display(), MAIN_SEPARATOR);
    match abs.to_string_lossy().strip_prefix(&prefix) {
        Some(rel) => rel.to_string(),
        None => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(tag: &str) -> String {
        let dir = std::env::temp_dir().join(format!(
            "cw-policy-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    #[test]
    fn 单根_相对路径_词法越界拒绝() {
        let root = tmp_root("single");
        std::fs::create_dir_all(format!("{root}/a")).unwrap();
        let (abs, rel) = resolve_at(&root, "a/b.txt").unwrap();
        // 返回真实路径（macOS /var → /private/var 的符号链接会被解析）
        assert!(abs.starts_with(&std::fs::canonicalize(&root).unwrap().to_string_lossy().to_string()));
        // Windows 分隔符为反斜杠：归一化后比较
        assert_eq!(rel.replace(std::path::MAIN_SEPARATOR, "/"), "a/b.txt");
        assert!(resolve_at(&root, "../outside").is_err());
        assert!(resolve_at(&root, "a/../../outside").is_err());
        assert!(resolve_at(&root, "/etc/passwd").is_err(), "绝对路径超出根必须拒绝");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 单根_新建文件_父目录真实路径拼接() {
        let root = tmp_root("newfile");
        std::fs::create_dir_all(format!("{root}/new/deep")).unwrap();
        let (abs, _) = resolve_at(&root, "new/deep/file.txt").unwrap();
        assert!(abs.starts_with(&std::fs::canonicalize(&root).unwrap().to_string_lossy().to_string()));
        // 父目录不存在时：与 TS 版一致拒绝（realpath 无法解析）
        assert!(resolve_at(&root, "missing/file.txt").is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn 单根_符号链接逃逸拒绝() {
        let root = tmp_root("symlink");
        let outside = tmp_root("outside");
        std::fs::create_dir_all(format!("{root}/inner")).unwrap();
        std::fs::write(format!("{outside}/secret.txt"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, format!("{root}/link")).unwrap();
        // 词法在根内，但真实路径在根外
        let result = resolve_at(&root, "link/secret.txt");
        assert!(result.is_err(), "符号链接逃逸必须被真实路径校验拦截");
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn 多根_主要目录优先_附加目录可达() {
        let main = tmp_root("multi-main");
        let extra = tmp_root("multi-extra");
        std::fs::write(format!("{main}/in-main.txt"), "x").unwrap();
        std::fs::write(format!("{extra}/in-extra.txt"), "x").unwrap();
        let roots = vec![main.clone(), extra.clone()];
        let resolved = resolve_existing(&roots, "in-main.txt").unwrap();
        assert_eq!(resolved.root_index, 0);
        let resolved = resolve_existing(&roots, "in-extra.txt").unwrap();
        assert_eq!(resolved.root_index, 1);
        // rel 为空返回第一个可用根
        let resolved = resolve_any(&roots, "", false).unwrap();
        assert_eq!(resolved.root_index, 0);
        assert_eq!(resolved.rel, ".");
        // 两根都越界 → 错误
        assert!(resolve_any(&roots, "../x", false).is_err());
        std::fs::remove_dir_all(&main).ok();
        std::fs::remove_dir_all(&extra).ok();
    }

    #[test]
    fn 多根_preferExisting_新建文件回落首个词法命中() {
        let main = tmp_root("lex-main");
        let extra = tmp_root("lex-extra");
        std::fs::write(format!("{extra}/exists.txt"), "x").unwrap();
        let roots = vec![main.clone(), extra.clone()];
        // 真实存在于附加目录
        let resolved = resolve_existing(&roots, "exists.txt").unwrap();
        assert_eq!(resolved.root_index, 1);
        // 均不存在（新建场景）：回落第一个词法命中（词法 abs 保留 main 前缀，未触盘解析）
        let resolved = resolve_existing(&roots, "new-file.txt").unwrap();
        assert_eq!(resolved.root_index, 0);
        assert!(resolved.abs.ends_with("new-file.txt"));
        assert!(resolved.abs.starts_with(&main) || resolved.abs.starts_with(&std::fs::canonicalize(&main).unwrap().to_string_lossy().to_string()));
        std::fs::remove_dir_all(&main).ok();
        std::fs::remove_dir_all(&extra).ok();
    }

    #[test]
    fn 展示路径() {
        let root = tmp_root("relative");
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(relative(&root, &format!("{root}{sep}a{sep}b.txt")), format!("a{sep}b.txt"));
        assert_eq!(relative(&root, &root), ".");
        // 根外路径原样返回
        assert_eq!(relative(&root, &format!("{sep}elsewhere{sep}x.txt")), format!("{sep}elsewhere{sep}x.txt"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 空根列表拒绝() {
        assert!(resolve_any(&[], "x", false).is_err());
        assert!(resolve_any(&["".to_string()], "x", false).is_err());
    }
}
