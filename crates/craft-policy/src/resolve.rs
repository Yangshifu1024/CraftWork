//! 路径安全解析：所有路径校验在后端完成，不信任渲染层传来的路径。
//! 核心不变量：词法校验拒绝 `..` 越界与 UNC 写法 + 真实路径（符号链接解析后）二次校验拒绝链接逃逸。
//! 单根解析走 [`resolve_at`] / [`lexical_at`]；多根（主要目录 + 附加目录）走 [`resolve_any`] / [`resolve_existing`]。
//!
//! Windows verbatim 前缀（`\\?\D:\...`，`std::fs::canonicalize` 的产物）在解析入口由
//! [`strip_verbatim`] 统一剥离：仅盘符形式剥成普通盘符路径参与校验并作为返回形式；
//! `\\?\UNC\...` 不剥，仍按 UNC 由词法守卫拒绝。否则根一旦以 `\\` 开头，所有
//! `root.join(rel)` 都会被 UNC 守卫判越界，工具层全灭（2026-11-10 会话级事故）。
//!
//! # 系统临时目录是隐含可访问根（决策 D28）
//!
//! [`system_temp_roots`] 给出系统临时目录（`std::env::temp_dir()`，Unix 另含 `/tmp`），
//! [`resolve_existing_with_temp`] 在工作区根之外把**绝对路径**解析进这些根。三条边界：
//!
//! - **只对绝对路径生效**：相对路径永远落在工作区根内。否则工作区里不存在的 `x.txt`
//!   会「存在优先」静默解析到临时目录里的同名文件，读到的不是用户以为的那个。
//! - **不进 `RunEnv roots`**：临时根只由解析层提供，不参与提示词的附加目录清单、
//!   Git「只认主要目录」、技能扫描与 worktree 来源——这些语义都不该被临时目录污染。
//!   要判断是否越界看 [`Resolved::system_temp`]，不要靠 `root_index` 猜。
//! - **清单固定、不扫描**：候选只有 `temp_dir()` 与 `/tmp`，每个都同时收录原样形式与
//!   canonicalize 形式（macOS 的 `/tmp` → `/private/tmp` 这类别名因此都能命中）；
//!   canonicalize 后是文件系统根的候选被拒绝——`TMPDIR=/` 不能变成「整盘放开」。

use std::path::{Component, Path, PathBuf, MAIN_SEPARATOR};

/// 路径超出工作区、附加目录与系统临时目录；词法越界与符号链接逃逸统一报此错。
#[derive(Debug, Clone)]
pub struct ErrOutsideWorkspace;

impl std::fmt::Display for ErrOutsideWorkspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "路径超出工作区、附加目录与系统临时目录")
    }
}

impl std::error::Error for ErrOutsideWorkspace {}

pub type ResolveResult<T> = Result<T, ErrOutsideWorkspace>;

/// 多根解析结果：`root_index` 命中的根下标、`root` 该根原文、`abs` 目标真实绝对路径、
/// `rel` 展示用相对路径（目标即根时为 "."）、`system_temp` 是否命中隐含的系统临时根。
#[derive(Debug, Clone)]
pub struct Resolved {
    pub root_index: usize,
    pub root: String,
    pub abs: String,
    pub rel: String,
    /// 命中 [`system_temp_roots`] 里的根（而不是调用方给的 `roots`）。`root_index` 此时是
    /// 「roots 长度 + 临时根序号」；要区分两类根一律看本字段，不要靠下标猜。
    pub system_temp: bool,
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

/// `std::fs::canonicalize` 的 Option 化封装：目标不存在或不可达时返回 None。
fn eval_symlinks(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// 剥离 Windows verbatim 前缀：`\\?\D:\...` → `D:\...`，仅盘符形式。
/// `\\?\UNC\...` 与 `\\?\Volume{...}` 不剥：前者是 UNC 语义（交由词法守卫拒绝），
/// 后者剥掉后不可用。非 verbatim 输入原样返回。
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let Some(rest) = s.strip_prefix(r"\\?\") else {
        return path.to_path_buf();
    };
    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        (Some(d), Some(':')) if d.is_ascii_alphabetic() => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
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
    let root_path = strip_verbatim(Path::new(lex_root));
    let rel_path = Path::new(rel);
    let candidate = if rel_path.is_absolute() {
        strip_verbatim(&lexical_normalize(rel_path))
    } else {
        strip_verbatim(&lexical_normalize(&root_path.join(rel_path)))
    };
    let candidate_str = candidate.to_string_lossy().to_string();
    if candidate_str.starts_with("\\\\") || candidate_str.starts_with("//") {
        return Err(ErrOutsideWorkspace);
    }
    if !within(&root_path, &candidate) {
        return Err(ErrOutsideWorkspace);
    }

    let real = match eval_symlinks(&candidate) {
        Some(real) => strip_verbatim(&real),
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
            let real_parent = strip_verbatim(&eval_symlinks(&parent).ok_or(ErrOutsideWorkspace)?);
            real_parent.join(base)
        }
    };
    let real_root = eval_symlinks(&root_path)
        .map(|p| strip_verbatim(&p))
        .unwrap_or_else(|| root_path.clone());
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

/// 纯词法校验（不触盘，不解析符号链接）：绝对路径归一、拒绝 `//` 与 `\` 开头、越界拒绝。
/// 供目标尚不存在、canonicalize 必然失败的预检场景（如新建文件前）；建目录由调用方负责。
pub fn lexical_at(root: &str, rel: &str) -> ResolveResult<(String, String)> {
    let root_path = strip_verbatim(Path::new(root));
    let rel_path = Path::new(rel);
    let candidate = if rel_path.is_absolute() {
        strip_verbatim(&lexical_normalize(rel_path))
    } else {
        strip_verbatim(&lexical_normalize(&root_path.join(rel_path)))
    };
    let candidate_str = candidate.to_string_lossy().to_string();
    if candidate_str.starts_with("\\") || candidate_str.starts_with("//") {
        return Err(ErrOutsideWorkspace);
    }
    if !within(&root_path, &candidate) {
        return Err(ErrOutsideWorkspace);
    }
    let rel_out = if candidate == root_path {
        ".".to_string()
    } else {
        candidate
            .to_string_lossy()
            .strip_prefix(format!("{}{}", root_path.display(), MAIN_SEPARATOR).as_str())
            .map(str::to_string)
            .unwrap_or_else(|| candidate_str.clone())
    };
    Ok((candidate_str, rel_out))
}

fn root_candidates(roots: &[String]) -> Vec<String> {
    roots.iter().filter(|r| !r.is_empty()).cloned().collect()
}

/// 系统临时目录的候选根清单：`std::env::temp_dir()`，Unix 另含 `/tmp`。
/// 解析用（[`system_temp_roots`]）与展示用（[`system_temp_roots_display`]）共用这份清单，
/// 避免两处各写一遍 `#[cfg(unix)]` 分支（Windows 上 `mut` 会因分支缺席而告警）。
fn temp_root_candidates() -> Vec<PathBuf> {
    #[cfg(unix)]
    {
        vec![std::env::temp_dir(), PathBuf::from("/tmp")]
    }
    #[cfg(not(unix))]
    {
        vec![std::env::temp_dir()]
    }
}

/// 系统临时目录的候选根（决策 D28）：`std::env::temp_dir()`，Unix 另含 `/tmp`。
///
/// 每个候选收录两种形式——原样路径与 canonicalize 后的真实路径——所以 macOS 的
/// `/tmp`（符号链接到 `/private/tmp`）两种写法都能命中。过滤规则：
/// - 必须存在的目录（`temp_dir()` 被删掉不是错误，静默跳过）；
/// - canonicalize 后是文件系统根（`/`、`C:\`）的候选**丢弃**：`TMPDIR=/` 不能变成整盘放开；
/// - 按「原样 → 真实」顺序去重，保证返回顺序稳定（提示词与审批卡展示直接用它）。
///
/// 为什么 `temp_dir()` 已经返回 `/tmp` 还要显式列 `/tmp`：`TMPDIR` 被设成别的目录时
/// （如某些 shell 会话），`/tmp` 仍是用户与系统共同认可的临时位置，两者都该能用。
pub fn system_temp_roots() -> Vec<String> {
    let candidates = temp_root_candidates();

    let mut out: Vec<String> = Vec::new();
    for candidate in candidates {
        let raw = strip_verbatim(&lexical_normalize(&candidate));
        if !is_usable_temp_root(&raw) {
            continue;
        }
        push_unique(&mut out, &raw);
        if let Some(real) = eval_symlinks(&raw) {
            let real = strip_verbatim(&real);
            if is_usable_temp_root(&real) {
                push_unique(&mut out, &real);
            }
        }
    }
    out
}

/// 临时根可用性：存在的目录，且（能 canonicalize 时）不是文件系统根。
fn is_usable_temp_root(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    match eval_symlinks(path) {
        Some(real) => real.parent().is_some(),
        // 解析不了真实路径时按路径本身判：`/` 与 `C:\` 没有父目录
        None => path.parent().is_some(),
    }
}

/// 把规范化后的路径字符串按序去重压入（临时根清单用）。
fn push_unique(out: &mut Vec<String>, path: &Path) {
    let text = path.to_string_lossy().to_string();
    if !out.iter().any(|existing| existing == &text) {
        out.push(text);
    }
}

/// 展示用临时根清单：每个候选只取**原样形式**（不列 canonicalize 出来的别名）。
///
/// 解析必须用 [`system_temp_roots`]——那份同时收录 `/tmp` 与 `/private/tmp` 这类别名，
/// 否则 macOS 上 canonicalize 后的绝对路径会解析失败。但展示是另一回事：给模型与用户看
/// 四条「同两个目录的两种写法」只是噪音，故这里每个候选只留一条。
pub fn system_temp_roots_display() -> Vec<String> {
    let candidates = temp_root_candidates();

    let mut out: Vec<String> = Vec::new();
    for candidate in candidates {
        let raw = strip_verbatim(&lexical_normalize(&candidate));
        if is_usable_temp_root(&raw) {
            push_unique(&mut out, &raw);
        }
    }
    out
}

/// 多根解析（roots[0] = 主要目录，resolve.ts: resolveAny 直译）：
/// - 主要目录不可用仍报错；附加目录不可达则跳过
/// - prefer_existing=true 时词法命中先不返回，确认真实存在才命中；
///   全程无真实存在则回落第一个词法命中（新建文件是合法场景）
/// - rel 为空串返回第一个可用根本身
pub fn resolve_any(roots: &[String], rel: &str, prefer_existing: bool) -> ResolveResult<Resolved> {
    resolve_any_ext(roots, &[], rel, prefer_existing)
}

/// [`resolve_any`] 的带隐含根版本：`temp_roots` 是追加在用户根之后的系统临时根
/// （调用方一般传 [`system_temp_roots`] 的结果）。**只对绝对路径生效**——相对路径
/// 永远落在工作区根内，避免「存在优先」把工作区里不存在的同名文件解析到临时目录。
pub fn resolve_any_ext(
    roots: &[String],
    temp_roots: &[String],
    rel: &str,
    prefer_existing: bool,
) -> ResolveResult<Resolved> {
    let candidates = root_candidates(roots);
    if candidates.is_empty() {
        return Err(ErrOutsideWorkspace);
    }
    let clean_rel = rel.trim();
    // 临时根只在目标是绝对路径时参与：相对路径属于工作区语义（见模块头边界）
    let use_temps = !clean_rel.is_empty() && Path::new(clean_rel).is_absolute();
    let temps: Vec<String> = if use_temps {
        root_candidates(temp_roots)
    } else {
        Vec::new()
    };
    let mut first_lexical: Option<Resolved> = None;

    let ordered = candidates
        .iter()
        .map(|root| (root.clone(), false))
        .chain(temps.iter().map(|root| (root.clone(), true)))
        .collect::<Vec<(String, bool)>>();

    for (i, (root, system_temp)) in ordered.iter().enumerate() {
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
                system_temp: *system_temp,
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
            system_temp: *system_temp,
        });
    }
    first_lexical.ok_or(ErrOutsideWorkspace)
}

/// 多根解析，优先命中真实存在的目标。
pub fn resolve_existing(roots: &[String], rel: &str) -> ResolveResult<Resolved> {
    resolve_any(roots, rel, true)
}

/// 多根解析，目标为绝对路径时把系统临时目录作为隐含可访问根参与（决策 D28）。
///
/// 工具层（`Registry::assess`）用这个入口，于是 `/tmp/x.py` 这类临时产物路径不再被判
/// 越界；工作区根的语义与 [`resolve_existing`] 完全一致，命中临时根时结果带
/// `system_temp = true` 供调用方区分（Git 工具据它快速失败）。
pub fn resolve_existing_with_temp(roots: &[String], rel: &str) -> ResolveResult<Resolved> {
    resolve_any_ext(roots, &system_temp_roots(), rel, true)
}

/// 展示路径：相对主要根；越界原样返回。
pub fn relative(workspace: &str, path: &str) -> String {
    let abs = strip_verbatim(&lexical_normalize(Path::new(path)));
    let root = strip_verbatim(&lexical_normalize(Path::new(workspace)));
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

    /// 进程内唯一的测试临时根目录。
    ///
    /// 不用纳秒时间戳：macOS 上 `SystemTime` 的分辨率实际只有微秒级（实测连续取
    /// 10000 次有 9315 次重复），并发测试会撞名并共用同一目录，导致偶发失败。
    /// 自增序号在进程内绝对唯一；本 crate 零依赖，故本地实现而不复用 craft-core。
    fn tmp_root(tag: &str) -> String {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cw-policy-{tag}-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    #[test]
    fn single_root_relative_path_lexical_escape_rejected() {
        let root = tmp_root("single");
        std::fs::create_dir_all(format!("{root}/a")).unwrap();
        let (abs, rel) = resolve_at(&root, "a/b.txt").unwrap();
        // 返回真实路径（macOS /var → /private/var 的符号链接会被解析）；Windows 上剥 verbatim 前缀后比较
        assert!(abs.starts_with(&strip_verbatim(&std::fs::canonicalize(&root).unwrap()).to_string_lossy().to_string()));
        // Windows 分隔符为反斜杠：归一化后比较
        assert_eq!(rel.replace(std::path::MAIN_SEPARATOR, "/"), "a/b.txt");
        assert!(resolve_at(&root, "../outside").is_err());
        assert!(resolve_at(&root, "a/../../outside").is_err());
        assert!(resolve_at(&root, "/etc/passwd").is_err(), "绝对路径超出根必须拒绝");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn single_root_new_file_parent_realpath_join() {
        let root = tmp_root("newfile");
        std::fs::create_dir_all(format!("{root}/new/deep")).unwrap();
        let (abs, _) = resolve_at(&root, "new/deep/file.txt").unwrap();
        assert!(abs.starts_with(&strip_verbatim(&std::fs::canonicalize(&root).unwrap()).to_string_lossy().to_string()));
        // 父目录不存在时：与 TS 版一致拒绝（realpath 无法解析）
        assert!(resolve_at(&root, "missing/file.txt").is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn single_root_symlink_escape_rejected() {
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

    /// Windows 等价（无真机，CI 的 windows runner 是唯一验证场）：
    /// 用 `mklink /J` 建 junction——无需特权（symlink 需要管理员或开发者模式），
    /// canonicalize 经 GetFinalPathNameByHandle 同样解析到真实路径，逃逸语义与
    /// Unix symlink 一致。
    #[cfg(windows)]
    #[test]
    fn single_root_symlink_escape_rejected_windows() {
        let root = tmp_root("symlink");
        let outside = tmp_root("outside");
        std::fs::create_dir_all(std::path::Path::new(&root).join("inner")).unwrap();
        std::fs::write(std::path::Path::new(&outside).join("secret.txt"), "x").unwrap();
        let link = std::path::Path::new(&root).join("link");
        // mklink 是 cmd 内建命令，目标参数必须反斜杠（cmd 不解析正斜杠路径）
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &link.to_string_lossy().replace('/', "\\"),
                &outside.replace('/', "\\"),
            ])
            .status()
            .expect("mklink 应可运行");
        assert!(status.success(), "junction 创建失败：{status:?}");
        let result = resolve_at(&root, "link/secret.txt");
        assert!(result.is_err(), "junction 逃逸必须被真实路径校验拦截");
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    // ---------- verbatim 前缀（跨平台纯用例，不触盘） ----------

    /// [`strip_verbatim`] 只剥盘符形式：`\\?\UNC\...` 与 `\\?\Volume{...}` 原样保留，
    /// 非 verbatim 输入原样返回。
    #[test]
    fn strip_verbatim_prefix_only_for_drive_paths() {
        let f = |s: &str| strip_verbatim(Path::new(s)).to_string_lossy().to_string();
        assert_eq!(f(r"\\?\D:\Code\CraftWork"), r"D:\Code\CraftWork");
        assert_eq!(f(r"\\?\d:\x"), r"d:\x");
        assert_eq!(f(r"\\?\UNC\server\share\proj"), r"\\?\UNC\server\share\proj");
        assert_eq!(f(r"\\?\Volume{0123}\x"), r"\\?\Volume{0123}\x");
        assert_eq!(f(r"D:\Code\CraftWork"), r"D:\Code\CraftWork");
        assert_eq!(f("/usr/bin"), "/usr/bin");
        assert_eq!(f(r"\\?\"), r"\\?\");
    }

    /// verbatim UNC 根：词法守卫必须仍然拒绝——UNC 语义不因 verbatim 形式放行。
    #[test]
    fn verbatim_unc_root_lexically_rejected() {
        let root = r"\\?\UNC\server\share\proj";
        assert!(resolve_at(root, "x.txt").is_err());
        assert!(lexical_at(root, "x.txt").is_err());
        // 普通 UNC 写法同样拒绝（既有语义回归）
        assert!(resolve_at(r"\\server\share\proj", "x.txt").is_err());
    }

    // ---------- Windows（无真机，CI 的 windows runner 是唯一验证场） ----------
    // 风险点：canonicalize 在 Windows 返回 \\?\C:\...（verbatim 前缀），与普通路径
    // 比较可能失配；junction 无需特权、symlink 需要特权——两者的逃逸语义都要验证。

    /// 正常文件必须能解析：真实路径校验两侧（real 与 real_root）同为 canonicalize
    /// 产物时自洽，不能因 verbatim 前缀失配把根内文件误判为越界。
    #[cfg(windows)]
    #[test]
    fn normal_paths_resolve_survives_verbatim_prefix_windows() {
        let root = tmp_root("verbatim");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(std::path::Path::new(&root).join("a.txt"), "x").unwrap();
        let (abs, rel) = resolve_at(&root, "a.txt").expect("根内正常文件必须解析成功");
        assert_eq!(rel, "a.txt", "相对路径展示不应受前缀影响：{rel}");
        assert!(abs.to_lowercase().ends_with("a.txt"), "绝对路径应指向目标：{abs}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 根本身携带 verbatim 前缀（canonicalize 产物曾直接入库的历史形态）必须能正常解析，
    /// 且返回的 abs 是剥离前缀的干净形式——否则所有 `root.join(rel)` 都以 `\\` 开头，
    /// 被 UNC 词法守卫全灭（2026-11-10 会话级事故）。
    #[cfg(windows)]
    #[test]
    fn verbatim_root_relative_path_resolves_windows() {
        let root = tmp_root("verbatim-root");
        std::fs::write(std::path::Path::new(&root).join("a.txt"), "x").unwrap();
        let verbatim_root = format!(r"\\?\{}", root.replace('/', "\\"));
        let (abs, rel) = resolve_at(&verbatim_root, "a.txt").expect("verbatim 根内文件必须解析成功");
        assert_eq!(rel, "a.txt");
        assert!(
            !abs.starts_with(r"\\?\"),
            "abs 应为非 verbatim 形式：{abs}"
        );
        assert!(
            abs.starts_with(&strip_verbatim(&std::fs::canonicalize(std::path::Path::new(&root)).unwrap())
                .to_string_lossy()
                .to_string()),
            "abs 应落在真实根下：{abs}"
        );
        // 新建文件预检（目标不存在，走 lexical_at）同样放行
        let (abs2, _) = lexical_at(&verbatim_root, "new.txt").expect("verbatim 根新建预检必须成功");
        assert!(!abs2.starts_with(r"\\?\"));
        std::fs::remove_dir_all(&root).ok();
    }

    /// junction（目录联接）无需特权，是 Windows 上最易触发的逃逸载体：
    /// 词法在根内、真实路径在根外，必须被真实路径校验拦截。
    ///
    /// 为什么不用 std：标准库的 `std::os::windows::fs::junction_point` 至今仍是
    /// nightly 未稳定特性（`feature = "junction_point"`，见 rust-lang/rust#121709），
    /// 稳定工具链无法调用；`mklink /J` 是 Windows 上创建 junction 的唯一免特权稳定手段
    /// （symlink 需要 SeCreateSymbolicLinkPrivilege，junction 不需要）。
    #[cfg(windows)]
    #[test]
    fn single_root_junction_escape_rejected_windows() {
        let root = std::path::PathBuf::from(tmp_root("junction"));
        let outside = std::path::PathBuf::from(tmp_root("junction-out"));
        std::fs::write(outside.join("secret.txt"), "x").unwrap();
        let link = root.join("link");
        let output = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&link)
            .arg(&outside)
            .output()
            .expect("执行 mklink 失败：无法启动 cmd");
        assert!(
            output.status.success(),
            "创建 junction 失败（mklink /J {} {}）：{}{}",
            link.display(),
            outside.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let result = resolve_at(&root.to_string_lossy(), "link/secret.txt");
        assert!(result.is_err(), "junction 逃逸必须被真实路径校验拦截");
        // 先删 junction 本体再删根目录：避免 remove_dir_all 沿 reparse point 进入根外目录
        let _ = std::fs::remove_dir(&link);
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    /// 文件级符号链接逃逸（symlink 需要特权：开发者模式/管理员）。创建失败时跳过
    /// 而不是误报——特权不足的环境里 junction 用例已覆盖逃逸校验本体。
    #[cfg(windows)]
    #[test]
    fn single_root_symlink_file_escape_rejected_windows() {
        let root = std::path::PathBuf::from(tmp_root("symlink-win"));
        let outside = std::path::PathBuf::from(tmp_root("symlink-win-out"));
        std::fs::create_dir_all(root.join("inner")).unwrap();
        std::fs::write(outside.join("secret.txt"), "x").unwrap();
        if std::os::windows::fs::symlink_file(outside.join("secret.txt"), root.join("inner").join("link.txt")).is_err() {
            eprintln!("跳过：当前环境无符号链接创建特权（junction 用例已覆盖校验本体）");
            std::fs::remove_dir_all(&root).ok();
            std::fs::remove_dir_all(&outside).ok();
            return;
        }
        let result = resolve_at(&root.to_string_lossy(), "inner/link.txt");
        assert!(result.is_err(), "符号链接逃逸必须被真实路径校验拦截");
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn multi_root_main_preferred_extra_reachable() {
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
    fn multi_root_prefer_existing_new_file_fallback_first_lexical_hit() {
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
        assert!(resolved.abs.starts_with(&main) || resolved.abs.starts_with(&strip_verbatim(&std::fs::canonicalize(&main).unwrap()).to_string_lossy().to_string()));
        std::fs::remove_dir_all(&main).ok();
        std::fs::remove_dir_all(&extra).ok();
    }

    #[test]
    fn display_path() {
        let root = tmp_root("relative");
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(relative(&root, &format!("{root}{sep}a{sep}b.txt")), format!("a{sep}b.txt"));
        assert_eq!(relative(&root, &root), ".");
        // 根外路径原样返回
        assert_eq!(relative(&root, &format!("{sep}elsewhere{sep}x.txt")), format!("{sep}elsewhere{sep}x.txt"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_root_list_rejected() {
        assert!(resolve_any(&[], "x", false).is_err());
        assert!(resolve_any(&["".to_string()], "x", false).is_err());
    }

    // ---------- 系统临时目录 = 隐含可访问根（D28） ----------

    /// 临时目录里的文件按绝对路径解析成功，且结果标记 `system_temp`；
    /// 同一文件用相对路径仍按工作区语义（不因存在而漂移到临时目录）。
    #[test]
    fn system_temp_absolute_path_resolves_outside_workspace() {
        let workspace = tmp_root("temp-ws");
        let temp = std::env::temp_dir().join(format!("cw-policy-temp-file-{}-0", std::process::id()));
        std::fs::write(&temp, "x").unwrap();
        let temp_str = temp.to_string_lossy().to_string();
        let roots = vec![workspace.clone()];

        let resolved = resolve_existing_with_temp(&roots, &temp_str).expect("临时目录文件必须可解析");
        assert!(resolved.system_temp, "命中临时根时必须置 system_temp");
        assert_eq!(resolved.abs, strip_verbatim(&std::fs::canonicalize(&temp).unwrap()).to_string_lossy().to_string());

        // 相对路径不走临时根：工作区里没有同名文件 → 回落词法命中（工作区路径）
        let rel = resolve_existing_with_temp(&roots, "same-name.txt").expect("新建文件预检");
        assert!(!rel.system_temp, "相对路径必须落在工作区根内");
        assert!(rel.abs.starts_with(&workspace) || rel.abs.starts_with(
            &strip_verbatim(&std::fs::canonicalize(&workspace).unwrap()).to_string_lossy().to_string()
        ));

        std::fs::remove_file(&temp).ok();
        std::fs::remove_dir_all(&workspace).ok();
    }

    /// 临时目录内的新建文件（目标不存在）同样放行：模型先写临时脚本再执行是主场景。
    #[test]
    fn system_temp_new_file_preflight_allowed() {
        let workspace = tmp_root("temp-new-ws");
        let target = std::env::temp_dir().join(format!("cw-policy-temp-new-{}-0.py", std::process::id()));
        let target_str = target.to_string_lossy().to_string();
        let roots = vec![workspace.clone()];
        let resolved =
            resolve_existing_with_temp(&roots, &target_str).expect("临时目录内新建文件必须放行");
        assert!(resolved.system_temp);
        std::fs::remove_dir_all(&workspace).ok();
    }

    /// 临时根清单不是「整盘放开」：系统根目录、普通绝对路径仍被拒绝；
    /// 清单本身不含文件系统根；展示清单是解析清单的去别名视图。
    #[test]
    fn system_temp_roots_do_not_open_whole_disk() {
        let roots = system_temp_roots();
        assert!(!roots.is_empty(), "系统临时目录必须至少有一个可用根");
        let temp_dir = strip_verbatim(&std::env::temp_dir()).to_string_lossy().to_string();
        let temp_real = std::fs::canonicalize(&temp_dir).ok().map(|p| strip_verbatim(&p).to_string_lossy().to_string());
        assert!(
            roots.iter().any(|r| r == &temp_dir || Some(r) == temp_real.as_ref()),
            "清单必须包含系统临时目录：{roots:?}"
        );
        for root in &roots {
            assert_ne!(root, "/", "文件系统根不能当临时根");
            assert!(!root.is_empty());
        }
        // 展示清单是同一批候选的去别名视图：只留原样形式，且必须是解析清单的子集
        let display = system_temp_roots_display();
        assert!(!display.is_empty(), "展示清单不应为空");
        for root in &display {
            assert!(roots.iter().any(|r| r == root), "展示项 {root} 必须也在解析清单里");
        }
        assert!(display.len() <= roots.len(), "去别名后条目不会变多");

        let workspace = tmp_root("temp-deny-ws");
        let roots = vec![workspace.clone()];
        // 工作区与临时目录之外的路径仍越界
        assert!(resolve_existing_with_temp(&roots, "/etc/passwd").is_err());
        assert!(resolve_existing_with_temp(&roots, "../../../etc/passwd").is_err());
        // 双斜杠开头的绝对写法不因临时根而放行（UNC 语义在 Windows 上由词法守卫拒绝）
        assert!(resolve_existing_with_temp(&roots, "//server/share/x").is_err());
        std::fs::remove_dir_all(&workspace).ok();
    }

    /// 临时目录内的符号链接逃逸仍被真实路径校验拦下：临时根不是「免检区」。
    /// 目标必须指向临时根**之外**的真实目录（`/etc`）；指向临时根内的链接属正常访问。
    #[cfg(unix)]
    #[test]
    fn system_temp_symlink_escape_still_rejected() {
        let workspace = tmp_root("temp-link-ws");
        let link_dir = std::env::temp_dir().join(format!("cw-policy-temp-link-{}", std::process::id()));
        std::fs::create_dir_all(&link_dir).unwrap();
        std::os::unix::fs::symlink("/etc", link_dir.join("link")).unwrap();

        let target = link_dir.join("link").join("hosts");
        let target_str = target.to_string_lossy().to_string();
        let roots = vec![workspace.clone()];
        let result = resolve_existing_with_temp(&roots, &target_str);
        assert!(result.is_err(), "临时目录内的符号链接逃逸必须被拒绝");

        std::fs::remove_dir_all(&link_dir).ok();
        std::fs::remove_dir_all(&workspace).ok();
    }
}
