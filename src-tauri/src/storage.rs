//! 存储占用：设置 →「数据与统计」→ 存储分节的服务端语义（决策 D27）。
//!
//! 本模块只回答三件事：**占了多少**、**属于谁**、**哪些能删**。SQL 聚合在
//! `craft_store::storage`，目录递归在 `craft_core::platform::dir_usage`；这里放的是
//! 只有在应用壳层才成立的知识——数据目录里每个子目录叫什么、删它的后果是什么。
//!
//! ## 一、固定分类，不做任意扫描
//!
//! 数据目录下的类别是**写死的一份清单**（[`CATEGORIES`]），未知子项并入「其他」。
//! 为什么不扫目录自动列表：「这一坨是什么、删了会怎样」无法从目录名推断，而每个
//! 清理按钮的文案都必须能说清后果。新增一个数据目录子目录时，在这里加一行。
//!
//! ## 二、孤儿判定 = 会话 id 精确白名单
//!
//! 「孤儿文件」= 所属会话已不存在、但仍留在磁盘上的文件。判定方式是拿库里**全部**
//! 会话 id（含归档、含子代理子会话——子会话的日志同样合法）当白名单，磁盘上按 id
//! 命名的项不在其中即为孤儿。三条纪律：
//!
//! 1. **只认完整等值**：`logs/th_abc.log` 的 `th_abc` 必须整体等于某个会话 id。
//!    不做前缀匹配、不做通配、不按修改时间猜——时间启发会删掉「三个月没打开但还要
//!    的会话」的日志，而且解释不了「为什么这个是孤儿」。
//! 2. **共享日志永不删**：`app.log` / `model.log` 没有会话归属，任何清理动作都不动。
//! 3. **`shell-tasks/` 只删整目录**：目录名非会话 id 即孤儿，不递归猜里面的文件。
//!
//! ## 三、运行门禁：有任何任务在跑就不许清任何东西
//!
//! [`busy_reason`] 非空（有 run 在跑 / 有待审批 / 有后台命令在跑）时，**全部**清理
//! 动作在服务端直接被拒（[`StorageError::Busy`]），前端禁用按钮只是体验层。理由：
//! 清日志时正在写的那个文件、VACUUM 时正在跑的写事务，都会把「清理」变成「损坏」。
//! 门禁粒度是全局而非逐项——用户问「能不能清」时答案只该有一个。
//!
//! ## 四、只清可丢弃数据
//!
//! 本模块的清理动作**绝不触碰会话事实**：不删 `threads` / `events` / `runs` /
//! `approvals` 行，不删 `keys.json`、`skills/`、`skills-marketplace/`、`workspace/`。
//! 能清的只有**可丢弃数据**——备份、日志、后台输出、孤儿工作树、孤儿计划文件，
//! 以及附件（附件是例外：它挂着历史消息的缩略图引用，删了不破坏会话事实但图片会
//! 消失，故必须走二次确认对话框，见 `CleanupKind::AttachmentsAll` 的注释）。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use craft_core::contract::{
    StorageCategoryView, StorageCleanupAction, StorageCleanupResult, StorageDatabaseView, StorageOverview,
    StorageProjectGroup, StorageTableRow, StorageThreadRow, StorageThreadsView,
};
use craft_core::platform::{dir_usage, remove_path, DIR_USAGE_DEFAULT_MAX_ENTRIES};
use craft_store::storage::{self, ThreadRowCounts};
use craft_store::threads::ThreadRow;
use craft_store::workspaces::WorkspaceRow;
use craft_store::Db;

/// 会话占用行数上限：超过即截断并计数。会话压缩后体积都很小，200 条足够覆盖
/// 「哪个会话吃空间」的实际问题；再多只是把界面变成长列表。
pub const MAX_THREAD_ROWS_PER_PROJECT: usize = 200;

/// 分类的静态定义：kind / 展示名 / 说明 / 相对数据目录的路径。
///
/// 顺序即界面顺序：先列可清理的（用户最可能动手），再列只读的。
struct CategoryDef {
    kind: &'static str,
    label: &'static str,
    description: &'static str,
    /// None = 不按单个目录统计，由 `other` 统一兜底
    rel_path: &'static str,
}

const CATEGORIES: [CategoryDef; 8] = [
    CategoryDef {
        kind: "backups",
        label: "写前备份",
        description: "改文件、改文档前保存的原件；清空不影响现有文件，但已无法从备份恢复",
        rel_path: "backups",
    },
    CategoryDef {
        kind: "logs",
        label: "会话日志",
        description: "每个会话一条排查用时间线（JSONL）；删除只影响事后排查，不影响会话内容",
        rel_path: "logs",
    },
    CategoryDef {
        kind: "shellTasks",
        label: "后台命令输出",
        description: "run_in_background 命令的输出文件；删除只影响回看历史输出",
        rel_path: "shell-tasks",
    },
    CategoryDef {
        kind: "attachments",
        label: "附件",
        description: "粘贴的图片与 MCP 工具产出的图片；删除后历史消息里的图片不再显示",
        rel_path: "attachments",
    },
    CategoryDef {
        kind: "worktrees",
        label: "git 工作树",
        description: "勾选 worktree 发送时创建的独立工作树；有未提交改动时不要清",
        rel_path: "worktrees",
    },
    CategoryDef {
        kind: "plans",
        label: "计划文件",
        description: "批准的计划正文快照（compact 续读用）；删除后该计划不再注入历史",
        rel_path: "plans",
    },
    CategoryDef {
        kind: "skillsMarketplace",
        label: "技能商店缓存",
        description: "已安装插件的版本缓存，保留最近 3 版供回滚；旧版本由应用启动时自动清理",
        rel_path: "skills-marketplace",
    },
    CategoryDef {
        kind: "workspaceDefault",
        label: "临时会话目录",
        description: "内置临时会话的工作目录；随会话删除清理，不在这里手动清",
        rel_path: "workspace/default",
    },
];

/// 未归入任何会话的数据目录项（`keys.json`、`AGENTS.md`、`skills/`、日志共享文件等）。
const OTHER_KIND: &str = "other";

/// 清理动作（`storageCleanup(kind)` 的入参）。
///
/// `AttachmentsAll` 的例外说明：附件同样是「可丢弃数据」，但它挂着历史消息里的
/// 缩略图引用——删了不会破坏会话事实（消息正文与工具结果都在事件表里），只是图片
/// 显示不出来。因此它可清理，但必须走二次确认对话框。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupKind {
    BackupsAll,
    LogsOrphans,
    LogsAll,
    ShellTasksOrphans,
    ShellTasksAll,
    AttachmentsAll,
    WorktreesOrphans,
    PlansOrphans,
    Vacuum,
}

impl CleanupKind {
    /// 从通道参数解析；未知值返回 None（服务层报可读错误，不静默当成功）。
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "backupsAll" => Self::BackupsAll,
            "logsOrphans" => Self::LogsOrphans,
            "logsAll" => Self::LogsAll,
            "shellTasksOrphans" => Self::ShellTasksOrphans,
            "shellTasksAll" => Self::ShellTasksAll,
            "attachmentsAll" => Self::AttachmentsAll,
            "worktreesOrphans" => Self::WorktreesOrphans,
            "plansOrphans" => Self::PlansOrphans,
            "vacuum" => Self::Vacuum,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BackupsAll => "backupsAll",
            Self::LogsOrphans => "logsOrphans",
            Self::LogsAll => "logsAll",
            Self::ShellTasksOrphans => "shellTasksOrphans",
            Self::ShellTasksAll => "shellTasksAll",
            Self::AttachmentsAll => "attachmentsAll",
            Self::WorktreesOrphans => "worktreesOrphans",
            Self::PlansOrphans => "plansOrphans",
            Self::Vacuum => "vacuum",
        }
    }

    /// 审计与文案用的中文名。
    pub fn label(self) -> &'static str {
        match self {
            Self::BackupsAll => "清空写前备份",
            Self::LogsOrphans => "清理孤儿会话日志",
            Self::LogsAll => "清空会话日志",
            Self::ShellTasksOrphans => "清理孤儿后台输出",
            Self::ShellTasksAll => "清空后台命令输出",
            Self::AttachmentsAll => "清空附件",
            Self::WorktreesOrphans => "清理孤儿工作树",
            Self::PlansOrphans => "清理孤儿计划文件",
            Self::Vacuum => "压缩数据库",
        }
    }
}

/// 门禁拒绝：有任务在运行时清理一律不做。
#[derive(Debug)]
pub enum StorageError {
    /// 有 run / 待审批 / 后台命令在跑，附带可读原因
    Busy(String),
    /// 统计或清理本身失败（IO / SQL），已转成可读中文
    Failed(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(reason) => write!(f, "{reason}"),
            Self::Failed(msg) => write!(f, "{msg}"),
        }
    }
}

/// 运行门禁：返回非空原因即禁止一切清理动作。
///
/// 判定集合 = ①有在跑或待审批的 run（`running_threads` 已含 awaiting）②有后台命令
/// 任务在跑。两者任一成立就拦——清理碰的文件与它们正在写的是同一批。
pub fn busy_reason(running_threads: &[String], background_tasks: usize) -> Option<String> {
    match (running_threads.is_empty(), background_tasks) {
        (true, 0) => None,
        (false, 0) => Some(format!("有 {} 个会话正在运行，任务结束后才能清理", running_threads.len())),
        (true, n) => Some(format!("有 {n} 个后台命令正在运行，任务结束后才能清理")),
        (false, n) => Some(format!(
            "有 {} 个会话正在运行、{n} 个后台命令正在运行，任务结束后才能清理",
            running_threads.len()
        )),
    }
}

/// 一次统计的中间结果：先把磁盘上有什么算清楚，再拼视图。
struct DiskSnapshot {
    /// kind → (bytes, files, truncated)
    categories: HashMap<&'static str, (u64, u64, bool)>,
    /// 各类下按名字索引的单项体积（孤儿判定与逐会话归属都要用）
    log_files: Vec<(String, u64)>,
    shell_task_dirs: Vec<(String, u64)>,
    worktree_dirs: Vec<(String, u64)>,
    plan_files: Vec<(String, u64)>,
}

/// 扫一遍数据目录。所有条目都只读，不改任何东西。
fn scan_disk(data_dir: &Path) -> DiskSnapshot {
    let mut categories: HashMap<&'static str, (u64, u64, bool)> = HashMap::new();
    let mut counted: HashSet<PathBuf> = HashSet::new();

    for def in &CATEGORIES {
        let dir = data_dir.join(def.rel_path);
        let usage = dir_usage(&dir, DIR_USAGE_DEFAULT_MAX_ENTRIES);
        categories.insert(def.kind, (usage.bytes, usage.files, usage.truncated));
        // 记录已统计的顶层路径，供「其他」兜底时排除（避免重复计入）
        counted.insert(dir);
    }

    let log_files = list_files(&data_dir.join("logs")).into_iter().map(|(name, size, _)| (name, size)).collect();
    let shell_task_dirs = list_dirs(&data_dir.join("shell-tasks"));
    let worktree_dirs = list_dirs(&data_dir.join("worktrees"));
    let plan_files = list_files(&data_dir.join("plans")).into_iter().map(|(name, size, _)| (name, size)).collect();

    // 「其他」= 数据目录直接子项里，既不是已分类目录、也不是库文件的部分。
    // 分类里有一项是嵌套路径（`workspace/default`），所以对直接子项要**减掉**已分类的
    // 后代路径，否则 `workspace/` 整棵递归会把 `workspace/default` 重复计一遍。
    let mut other_bytes: u64 = 0;
    let mut other_files: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(data_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if counted.contains(&path) {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&path) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            // 数据库本体（含 WAL/SHM）有自己的卡片，不并入「其他」
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "craftwork.db" || name == "craftwork.db-wal" || name == "craftwork.db-shm" {
                continue;
            }
            if meta.is_dir() {
                let usage = dir_usage(&path, DIR_USAGE_DEFAULT_MAX_ENTRIES);
                // 已分类的后代路径（如 workspace/default）：从本子树里扣掉，避免重复计
                let mut subtract_bytes: u64 = 0;
                let mut subtract_files: u64 = 0;
                for classified in &counted {
                    if classified.starts_with(&path) {
                        let sub = dir_usage(classified, DIR_USAGE_DEFAULT_MAX_ENTRIES);
                        subtract_bytes += sub.bytes;
                        subtract_files += sub.files;
                    }
                }
                other_bytes += usage.bytes.saturating_sub(subtract_bytes);
                other_files += usage.files.saturating_sub(subtract_files);
            } else if meta.is_file() {
                other_bytes += meta.len();
                other_files += 1;
            }
        }
    }
    categories.insert(OTHER_KIND, (other_bytes, other_files, false));

    DiskSnapshot { categories, log_files, shell_task_dirs, worktree_dirs, plan_files }
}

/// 列出目录下的文件（名字, 字节, 是否符号链接）；不存在或无权限返回空。
fn list_files(dir: &Path) -> Vec<(String, u64, bool)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
        let is_link = meta.file_type().is_symlink();
        if meta.is_file() || is_link {
            out.push((entry.file_name().to_string_lossy().to_string(), if is_link { 0 } else { meta.len() }, is_link));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 列出目录下的子目录（名字, 递归体积）；不存在或无权限返回空。
fn list_dirs(dir: &Path) -> Vec<(String, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
        if meta.is_dir() && !meta.file_type().is_symlink() {
            let usage = dir_usage(&entry.path(), DIR_USAGE_DEFAULT_MAX_ENTRIES);
            out.push((entry.file_name().to_string_lossy().to_string(), usage.bytes));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 日志文件名是否属于共享日志（`app.log` / `model.log` 没有会话归属，永不删）。
fn is_shared_log(name: &str) -> bool {
    name == "app.log" || name == "model.log"
}

/// 从 `<id>.log` 取出会话 id；不符合该形状返回 None。
fn thread_id_from_log(name: &str) -> Option<&str> {
    name.strip_suffix(".log")
}

/// 计算首屏视图。
pub fn overview(
    db: &Db,
    data_dir: &str,
    running_threads: &[String],
    background_tasks: usize,
) -> Result<StorageOverview, StorageError> {
    let data_path = Path::new(data_dir);
    let snapshot = scan_disk(data_path);

    let event_bytes = db
        .with(|conn| {
            let map = storage::thread_event_bytes(conn)?;
            Ok(map.values().sum::<u64>())
        })
        .map_err(|e| StorageError::Failed(format!("统计数据库事件字节失败：{e}")))?;

    let db_bytes = db.disk_usage_bytes();
    let main_bytes = std::fs::metadata(db.path()).map(|m| m.len()).unwrap_or(0);
    let wal_bytes = {
        let mut p = db.path().as_os_str().to_owned();
        p.push("-wal");
        std::fs::metadata(PathBuf::from(p)).map(|m| m.len()).unwrap_or(0)
    };

    let categories = build_categories(&snapshot);
    let files_total_bytes = categories.iter().map(|c| c.bytes).sum();

    Ok(StorageOverview {
        database: StorageDatabaseView { bytes: db_bytes, main_bytes, wal_bytes, event_bytes },
        categories,
        files_total_bytes,
        data_dir: data_dir.to_string(),
        busy_reason: busy_reason(running_threads, background_tasks).unwrap_or_default(),
    })
}

/// 把磁盘快照拼成带说明与可清理动作的分类视图。
fn build_categories(snapshot: &DiskSnapshot) -> Vec<StorageCategoryView> {
    let mut out = Vec::with_capacity(CATEGORIES.len() + 1);
    for def in &CATEGORIES {
        let (bytes, files, truncated) = snapshot.categories.get(def.kind).copied().unwrap_or((0, 0, false));
        out.push(StorageCategoryView {
            kind: def.kind.to_string(),
            label: def.label.to_string(),
            description: def.description.to_string(),
            bytes,
            files,
            truncated,
            cleanups: cleanups_for(def.kind),
        });
    }
    let (bytes, files, truncated) = snapshot.categories.get(OTHER_KIND).copied().unwrap_or((0, 0, false));
    out.push(StorageCategoryView {
        kind: OTHER_KIND.to_string(),
        label: "其他".to_string(),
        description: "密钥文件、自定义指令、技能目录等应用自管数据；不可清理".to_string(),
        bytes,
        files,
        truncated,
        cleanups: Vec::new(),
    });
    out
}

/// 某一类可执行的清理动作（空 = 只读展示）。
///
/// 技能商店缓存与临时会话目录**故意没有清理按钮**：前者的历史版本是 D12 承诺的
/// 回滚资产、旧版本已由启动时自动清理；后者随会话删除而清理。把有价资产放进「清理」
/// 清单会让整张表的清理论述失去可信度。
fn cleanups_for(kind: &str) -> Vec<StorageCleanupAction> {
    let action = |kind: &str, label: &str, destructive: bool, confirm: &str| StorageCleanupAction {
        kind: kind.to_string(),
        label: label.to_string(),
        destructive,
        confirm: confirm.to_string(),
    };
    match kind {
        "backups" => vec![action(
            "backupsAll",
            "全部清理",
            true,
            "将删除全部写前备份。已有文件不受影响，但删除后无法再从备份恢复任何被改过的文件。",
        )],
        "logs" => vec![
            action("logsOrphans", "清理孤儿", false, "将删除所属会话已不存在的日志文件。"),
            action(
                "logsAll",
                "全部清理",
                true,
                "将删除全部会话日志（含正在使用的会话）。日志只用于排查问题，不影响会话内容，但删除后无法回溯历史运行时间线。",
            ),
        ],
        "shellTasks" => vec![
            action("shellTasksOrphans", "清理孤儿", false, "将删除所属会话已不存在的后台命令输出。"),
            action("shellTasksAll", "全部清理", true, "将删除全部后台命令输出文件；历史命令的完整输出将无法回看。"),
        ],
        "attachments" => vec![action(
            "attachmentsAll",
            "全部清理",
            true,
            "将删除全部附件。会话文字与工具结果都在，但历史消息里贴过的图片、MCP 工具产出的截图将无法显示，且无法恢复。",
        )],
        "worktrees" => vec![action(
            "worktreesOrphans",
            "清理孤儿",
            false,
            "将删除所属会话已不存在的工作树目录（含其中未提交的改动），不影响主仓库。",
        )],
        "plans" => vec![action("plansOrphans", "清理孤儿", false, "将删除库里已无登记记录的计划正文文件。")],
        _ => Vec::new(),
    }
}

/// 会话占用统计的原始输入（四个查询同一次持锁读出，避免多次取锁与口径漂移）。
type ThreadStats = (
    Vec<ThreadRow>,
    HashMap<String, u64>,
    HashMap<String, ThreadRowCounts>,
    Vec<WorkspaceRow>,
);

/// 计算会话占用视图（按项目分组，按体积降序）。
pub fn threads(db: &Db, data_dir: &str) -> Result<StorageThreadsView, StorageError> {
    let data_path = Path::new(data_dir);
    let snapshot = scan_disk(data_path);

    let (all_threads, event_bytes, row_counts, workspaces): ThreadStats = db
        .with(|conn| {
            Ok((
                craft_store::threads::list_all_threads(conn)?,
                storage::thread_event_bytes(conn)?,
                storage::thread_row_counts(conn)?,
                craft_store::workspaces::list_workspaces(conn)?.into_iter().map(|w| w.workspace).collect(),
            ))
        })
        .map_err(|e| StorageError::Failed(format!("统计会话占用失败：{e}")))?;

    // 计划文件按「basename → 体积」索引；用 basename 而不是完整路径，因为
    // `plans.file_path` 记的是绝对路径，而存档目录随 CRAFTWORK_HOME 变化。
    let plan_sizes: HashMap<&str, u64> = snapshot.plan_files.iter().map(|(n, s)| (n.as_str(), *s)).collect();
    let plan_owner: HashMap<String, String> = db
        .with(storage::plan_file_owners)
        .map_err(|e| StorageError::Failed(format!("读取计划文件登记失败：{e}")))?;

    // 逐会话归属磁盘文件
    let mut rows: Vec<StorageThreadRow> = Vec::with_capacity(all_threads.len());
    let mut total: u64 = 0;
    for row in &all_threads {
        total += 1;
        let log_bytes = snapshot
            .log_files
            .iter()
            .filter(|(name, _)| thread_id_from_log(name) == Some(row.id.as_str()))
            .map(|(_, size)| *size)
            .sum::<u64>();
        let shell_task_bytes = snapshot
            .shell_task_dirs
            .iter()
            .filter(|(name, _)| name == &row.id)
            .map(|(_, size)| *size)
            .sum::<u64>();
        let worktree_bytes = snapshot
            .worktree_dirs
            .iter()
            .filter(|(name, _)| name == &row.id)
            .map(|(_, size)| *size)
            .sum::<u64>();
        let plan_bytes = plan_owner
            .iter()
            .filter(|(_, owner)| *owner == &row.id)
            .filter_map(|(name, _)| plan_sizes.get(name.as_str()).copied())
            .sum::<u64>();
        let db_bytes = event_bytes.get(&row.id).copied().unwrap_or(0);
        let counts = row_counts.get(&row.id).copied().unwrap_or_default();
        rows.push(StorageThreadRow {
            thread_id: row.id.clone(),
            title: row.title.clone(),
            mode: row.mode.clone(),
            archived: row.archived,
            is_subagent: !row.parent_thread_id.is_empty(),
            db_bytes,
            file_bytes: log_bytes + shell_task_bytes + worktree_bytes + plan_bytes,
            log_bytes,
            shell_task_bytes,
            worktree_bytes,
            plan_bytes,
            events: counts.events,
            runs: counts.runs,
            approvals: counts.approvals,
            updated_at: row.updated_at,
        });
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.db_bytes + r.file_bytes));

    // 按工作区归组；工作区已不存在的会话进 orphan_threads
    let ws_by_id: HashMap<&str, &WorkspaceRow> = workspaces.iter().map(|w| (w.id.as_str(), w)).collect();
    let ws_of_thread: HashMap<&str, &str> =
        all_threads.iter().map(|t| (t.id.as_str(), t.workspace_id.as_str())).collect();
    let mut grouped: HashMap<String, Vec<StorageThreadRow>> = HashMap::new();
    let mut orphans: Vec<StorageThreadRow> = Vec::new();
    for row in rows {
        let ws_id = ws_of_thread.get(row.thread_id.as_str()).copied().unwrap_or_default();
        if ws_by_id.contains_key(ws_id) {
            grouped.entry(ws_id.to_string()).or_default().push(row);
        } else {
            orphans.push(row);
        }
    }

    let mut projects: Vec<StorageProjectGroup> = grouped
        .into_iter()
        .filter_map(|(ws_id, mut threads)| {
            let ws = ws_by_id.get(ws_id.as_str())?;
            let db_sum = threads.iter().map(|t| t.db_bytes).sum();
            let file_sum = threads.iter().map(|t| t.file_bytes).sum();
            let total_threads = threads.len() as u64;
            let omitted = total_threads.saturating_sub(MAX_THREAD_ROWS_PER_PROJECT as u64);
            threads.truncate(MAX_THREAD_ROWS_PER_PROJECT);
            Some(StorageProjectGroup {
                workspace_id: ws.id.clone(),
                workspace_name: ws.name.clone(),
                is_conversation: ws.purpose == "conversation",
                db_bytes: db_sum,
                file_bytes: file_sum,
                thread_count: total_threads,
                threads,
                omitted_threads: omitted,
            })
        })
        .collect();
    projects.sort_by_key(|p| std::cmp::Reverse(p.db_bytes + p.file_bytes));

    Ok(StorageThreadsView { projects, orphan_threads: orphans, thread_total: total })
}

/// 表级明细（dbstat）；不可用或失败时返回空列表（界面隐藏明细）。
pub fn db_tables(db: &Db) -> Vec<StorageTableRow> {
    match db.with(storage::db_table_bytes) {
        Ok(Some(rows)) => rows.into_iter().map(|(name, bytes)| StorageTableRow { name, bytes }).collect(),
        // dbstat 不可用 / 查询失败：明细整体隐藏，不给估算替代
        _ => Vec::new(),
    }
}

/// 执行一次清理。**先过全局门禁**，再按动作派发。
///
/// 返回释放字节与逐条失败摘要；单个文件删不掉只记 failures，不整体失败（部分成功
/// 优于全盘放弃）。
pub fn cleanup(
    db: &Db,
    data_dir: &str,
    kind: CleanupKind,
    running_threads: &[String],
    background_tasks: usize,
) -> Result<StorageCleanupResult, StorageError> {
    if let Some(reason) = busy_reason(running_threads, background_tasks) {
        return Err(StorageError::Busy(reason));
    }
    let data = Path::new(data_dir);
    let mut result = StorageCleanupResult {
        kind: kind.as_str().to_string(),
        freed_bytes: 0,
        removed_items: 0,
        skipped_items: 0,
        failures: Vec::new(),
    };

    match kind {
        CleanupKind::Vacuum => {
            db.with(|conn| {
                // VACUUM 在 WAL 模式下会把整个重写日志写进 `-wal`（约等于重写后的主库
                // 大小），提交后若不 checkpoint，`-wal` 保持大文件——disk_usage_bytes
                // 把「主库 + 这个大 WAL」两个副本都算进去，界面看到「压缩后反而翻倍」。
                // 修法：VACUUM 提交后在同一连接上立即 wal_checkpoint(TRUNCATE)，把 WAL
                // 内容并回主库并截断归零，此时才是压缩后的真实占用。两者都在同一把
                // Mutex 锁内串行执行，无并发写冲突；checkpoint 失败（返回非 0）不阻断，
                // 只记进 failures（WAL 会随下一次正常 checkpoint 自然归零）。
                conn.execute_batch("VACUUM").map_err(|e| craft_store::StoreError::Other(format!("压缩数据库失败：{e}")))?;
                let ckpt = conn
                    .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get::<_, i64>(0))
                    .unwrap_or(-1);
                if ckpt != 0 {
                    result.failures.push(format!("压缩后检查点未完全归零（返回码 {ckpt}），WAL 会随后续写入自然回收"));
                }
                Ok(())
            })
            .map_err(|e| StorageError::Failed(e.to_string()))?;
            result.removed_items = 1;
            return Ok(result);
        }
        CleanupKind::BackupsAll => {
            for (name, size, _) in list_files(&data.join("backups")) {
                let path = data.join("backups").join(&name);
                if remove_path(&path) {
                    result.freed_bytes += size;
                    result.removed_items += 1;
                } else {
                    result.failures.push(format!("无法删除备份 {name}"));
                }
            }
        }
        CleanupKind::LogsOrphans | CleanupKind::LogsAll => {
            let keep = if kind == CleanupKind::LogsOrphans {
                Some(thread_ids(db)?)
            } else {
                None
            };
            let dir = data.join("logs");
            for (name, size, is_link) in list_files(&dir) {
                // 共享日志永不删：它们不属于任何会话，删了只是丢排查能力
                if is_shared_log(&name) {
                    result.skipped_items += 1;
                    continue;
                }
                if let Some(keep) = &keep {
                    let Some(id) = thread_id_from_log(&name) else {
                        result.skipped_items += 1;
                        continue;
                    };
                    if keep.contains(id) {
                        result.skipped_items += 1;
                        continue;
                    }
                }
                let path = dir.join(&name);
                if is_link || remove_path(&path) {
                    result.freed_bytes += size;
                    result.removed_items += 1;
                } else {
                    result.failures.push(format!("无法删除日志 {name}"));
                }
            }
        }
        CleanupKind::ShellTasksOrphans | CleanupKind::ShellTasksAll => {
            let keep = if kind == CleanupKind::ShellTasksOrphans {
                Some(thread_ids(db)?)
            } else {
                None
            };
            let dir = data.join("shell-tasks");
            for (name, size) in list_dirs(&dir) {
                // 孤儿模式只删不在白名单里的目录；全量模式一律删。
                // 白名单为空集（全量）时 contains 恒假 → 等价「全部删」，无需分支。
                if keep.as_ref().is_some_and(|k| k.contains(&name)) {
                    result.skipped_items += 1;
                    continue;
                }
                if remove_path(&dir.join(&name)) {
                    result.freed_bytes += size;
                    result.removed_items += 1;
                } else {
                    result.failures.push(format!("无法删除后台输出目录 {name}"));
                }
            }
        }
        CleanupKind::AttachmentsAll => {
            let dir = data.join("attachments");
            for (name, size, _) in list_files(&dir) {
                let path = dir.join(&name);
                if remove_path(&path) {
                    result.freed_bytes += size;
                    result.removed_items += 1;
                } else {
                    result.failures.push(format!("无法删除附件 {name}"));
                }
            }
        }
        CleanupKind::WorktreesOrphans => {
            let keep = thread_ids(db)?;
            let dir = data.join("worktrees");
            for (name, size) in list_dirs(&dir) {
                if keep.contains(&name) {
                    result.skipped_items += 1;
                    continue;
                }
                if remove_path(&dir.join(&name)) {
                    result.freed_bytes += size;
                    result.removed_items += 1;
                } else {
                    result.failures.push(format!("无法删除工作树 {name}"));
                }
            }
        }
        CleanupKind::PlansOrphans => {
            let registered: HashSet<String> = registered_plan_names(db)?;
            let dir = data.join("plans");
            for (name, size, _) in list_files(&dir) {
                if registered.contains(&name) {
                    result.skipped_items += 1;
                    continue;
                }
                if remove_path(&dir.join(&name)) {
                    result.freed_bytes += size;
                    result.removed_items += 1;
                } else {
                    result.failures.push(format!("无法删除计划文件 {name}"));
                }
            }
        }
    }
    Ok(result)
}

/// 库里全部会话 id（孤儿白名单）。
fn thread_ids(db: &Db) -> Result<HashSet<String>, StorageError> {
    db.with(storage::all_thread_ids)
        .map_err(|e| StorageError::Failed(format!("读取会话清单失败：{e}")))
}

/// 库里登记的计划文件名集合（basename）。
fn registered_plan_names(db: &Db) -> Result<HashSet<String>, StorageError> {
    let owners = db
        .with(storage::plan_file_owners)
        .map_err(|e| StorageError::Failed(format!("读取计划文件登记失败：{e}")))?;
    Ok(owners.into_keys().collect())
}

/// 删除会话附带的全部文件（日志 / 后台输出 / 计划文件 / 工作树目录）。
///
/// 会话删除路径（`services::delete_thread_fully`）与存储页共用这一份口径：删会话就
/// 该删干净，否则存储页会立刻把刚删掉的会话的日志报成「孤儿」。
///
/// 纪律：**只删按会话 id 精确命名的路径**。日志是 `logs/<id>.log` 完整等值匹配、
/// 后台输出与工作树是 `shell-tasks/<id>` / `worktrees/<id>` 整目录；计划文件按
/// `plans.file_path` 列出的完整路径逐条删。不做前缀匹配、不扫描目录猜归属。
///
/// 返回删除失败的摘要（best-effort：失败不阻塞会话删除，只回报给调用方记日志）。
pub fn remove_thread_files(data_dir: &str, thread_id: &str, plan_paths: &[String]) -> Vec<String> {
    let mut failures = Vec::new();
    let data = Path::new(data_dir);

    let log = data.join("logs").join(format!("{thread_id}.log"));
    if log.exists() && !remove_path(&log) {
        failures.push(format!("删除会话日志失败：{}", log.display()));
    }
    for dir in ["shell-tasks", "worktrees"] {
        let path = data.join(dir).join(thread_id);
        if path.exists() && !remove_path(&path) {
            failures.push(format!("删除 {dir} 目录失败：{}", path.display()));
        }
    }
    for plan in plan_paths {
        if plan.is_empty() {
            continue;
        }
        let path = Path::new(plan);
        if path.exists() && !remove_path(path) {
            failures.push(format!("删除计划文件失败：{plan}"));
        }
    }
    failures
}

/// 把清理结果写成一行审计 `detail`（服务层落库用）。
pub fn audit_detail(result: &StorageCleanupResult) -> String {
    let label = CleanupKind::parse(&result.kind).map(|k| k.label()).unwrap_or("存储清理");
    let failures = if result.failures.is_empty() {
        String::new()
    } else {
        format!(" · {} 项失败", result.failures.len())
    };
    format!(
        "{label}：释放 {} 字节 / 删除 {} 项 / 跳过 {} 项{}",
        result.freed_bytes, result.removed_items, result.skipped_items, failures
    )
}

/// 序列化辅助：契约结构自身可序列化，这里只用于类型收敛（服务层统一转 Value）。
pub fn to_value<T: Serialize>(value: &T) -> serde_json::Result<serde_json::Value> {
    serde_json::to_value(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("craftwork-storage-svc-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn test_db(dir: &Path) -> Db {
        Db::open(&dir.join("craftwork.db")).unwrap()
    }

    fn insert_thread(db: &Db, id: &str, workspace_id: &str) {
        db.with(|conn| {
            craft_store::threads::create_thread(
                conn,
                craft_store::threads::NewThread {
                    id,
                    mode: "code",
                    title: "t",
                    title_auto: false,
                    model_id: "",
                    reasoning_level: "",
                    workspace_id,
                    status: "idle",
                    archived: false,
                },
            )
            .map(|_| ())
        })
        .unwrap();
    }

    #[test]
    fn busy_reason_reports_threads_and_background_tasks() {
        assert!(busy_reason(&[], 0).is_none());
        let one = busy_reason(&["th_a".into()], 0).unwrap();
        assert!(one.contains("1 个会话"), "{one}");
        let two = busy_reason(&[], 3).unwrap();
        assert!(two.contains("3 个后台命令"), "{two}");
        let both = busy_reason(&["th_a".into()], 2).unwrap();
        assert!(both.contains("会话") && both.contains("后台命令"), "{both}");
    }

    #[test]
    fn shared_logs_are_never_treated_as_orphans() {
        assert!(is_shared_log("app.log"));
        assert!(is_shared_log("model.log"));
        assert!(!is_shared_log("th_abc.log"));
        assert_eq!(thread_id_from_log("th_abc.log"), Some("th_abc"));
        assert_eq!(thread_id_from_log("notes.txt"), None);
    }

    /// 孤儿清理必须只认完整等值：活会话的日志留下，已删会话的日志删掉，
    /// 共享日志与无法识别的文件一律保留。
    #[test]
    fn logs_orphan_cleanup_keeps_live_sessions_and_shared_logs() {
        let dir = temp_dir("logs");
        let db = test_db(&dir);
        insert_thread(&db, "th_live", "ws1");
        let logs = dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("th_live.log"), b"live").unwrap();
        std::fs::write(logs.join("th_dead.log"), b"dead!").unwrap();
        std::fs::write(logs.join("app.log"), b"app").unwrap();
        std::fs::write(logs.join("model.log"), b"model").unwrap();
        std::fs::write(logs.join("random.txt"), b"x").unwrap();

        let result = cleanup(&db, &dir.to_string_lossy(), CleanupKind::LogsOrphans, &[], 0).unwrap();
        assert!(logs.join("th_live.log").exists(), "活会话日志必须保留");
        assert!(logs.join("app.log").exists(), "共享日志必须保留");
        assert!(logs.join("model.log").exists(), "共享日志必须保留");
        assert!(logs.join("random.txt").exists(), "无法识别的文件不碰");
        assert!(!logs.join("th_dead.log").exists(), "孤儿日志应删除");
        assert_eq!(result.removed_items, 1);
        assert_eq!(result.freed_bytes, 5);
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 全量清理也不删共享日志（它们没有会话归属，删了只是丢排查能力）。
    #[test]
    fn logs_all_cleanup_still_keeps_shared_logs() {
        let dir = temp_dir("logs-all");
        let db = test_db(&dir);
        let logs = dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("th_a.log"), b"a").unwrap();
        std::fs::write(logs.join("app.log"), b"app").unwrap();

        cleanup(&db, &dir.to_string_lossy(), CleanupKind::LogsAll, &[], 0).unwrap();
        assert!(!logs.join("th_a.log").exists());
        assert!(logs.join("app.log").exists());
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 门禁：有任务在跑时任何清理都必须被拒绝，而不是只把前端按钮变灰。
    #[test]
    fn cleanup_refuses_while_any_task_is_running() {
        let dir = temp_dir("busy");
        let db = test_db(&dir);
        let logs = dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("th_dead.log"), b"dead").unwrap();

        let err = cleanup(&db, &dir.to_string_lossy(), CleanupKind::LogsOrphans, &["th_live".into()], 0)
            .expect_err("有 run 在跑时必须拒绝");
        assert!(matches!(err, StorageError::Busy(_)), "应报 Busy：{err}");
        assert!(logs.join("th_dead.log").exists(), "被拒绝时不得动任何文件");

        let err = cleanup(&db, &dir.to_string_lossy(), CleanupKind::LogsOrphans, &[], 1)
            .expect_err("有后台命令在跑时必须拒绝");
        assert!(matches!(err, StorageError::Busy(_)));
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn worktree_orphans_keep_live_sessions() {
        let dir = temp_dir("worktrees");
        let db = test_db(&dir);
        insert_thread(&db, "th_live", "ws1");
        let wts = dir.join("worktrees");
        std::fs::create_dir_all(wts.join("th_live")).unwrap();
        std::fs::create_dir_all(wts.join("th_dead")).unwrap();
        std::fs::write(wts.join("th_dead/a.txt"), b"12345").unwrap();

        let result = cleanup(&db, &dir.to_string_lossy(), CleanupKind::WorktreesOrphans, &[], 0).unwrap();
        assert!(wts.join("th_live").exists());
        assert!(!wts.join("th_dead").exists());
        assert_eq!(result.removed_items, 1);
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn plan_orphans_keep_registered_files() {
        let dir = temp_dir("plans");
        let db = test_db(&dir);
        insert_thread(&db, "th_live", "ws1");
        let plan_path = dir.join("plans/plan_live.md").to_string_lossy().to_string();
        db.with(|conn| {
            let plan = craft_store::plans::create_plan(conn, "th_live", "r1")?;
            craft_store::plans::set_plan_file(conn, &plan.id, &plan_path)
        })
        .unwrap();
        let plans = dir.join("plans");
        std::fs::create_dir_all(&plans).unwrap();
        std::fs::write(plans.join("plan_live.md"), b"plan").unwrap();
        std::fs::write(plans.join("plan_dead.md"), b"gone").unwrap();

        cleanup(&db, &dir.to_string_lossy(), CleanupKind::PlansOrphans, &[], 0).unwrap();
        assert!(plans.join("plan_live.md").exists(), "登记在册的计划文件必须保留");
        assert!(!plans.join("plan_dead.md").exists());
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// VACUUM 回归：压缩后立即统计，占用必须是「主库」而不是「主库 + 重写副本的大 WAL」。
    ///
    /// WAL 模式下 VACUUM 会把整个重写日志写进 `-wal`（约等于主库大小），若不 checkpoint，
    /// disk_usage_bytes 会把两本都算进去 → 用户看到「压缩后反而翻倍」。修复是 VACUUM 后
    /// 同一事务边界内立即 wal_checkpoint(TRUNCATE)。本测试断言 VACUUM 后 WAL 归零、
    /// disk_usage_bytes 等于主库文件本身。
    #[test]
    fn vacuum_checkpoints_wal_so_usage_does_not_double() {
        let dir = temp_dir("vacuum");
        let db = test_db(&dir);
        // 制造会话 + 足够多的事件，让库有实际可压缩的内容
        insert_thread(&db, "th_vac", "ws1");
        for i in 0..500 {
            craft_store::events::record_event(
                &db,
                &craft_store::events::EventRecord {
                    thread_id: "th_vac".into(),
                    message_id: "m".into(),
                    run_id: None,
                    kind: "text".into(),
                    payload: serde_json::json!({ "text": "x".repeat(1000), "i": i }),
                },
            )
            .unwrap();
        }

        cleanup(&db, &dir.to_string_lossy(), CleanupKind::Vacuum, &[], 0).unwrap();

        // 修复后：WAL 应已 checkpoint 归零（或接近 0），disk_usage_bytes = 主库 + WAL + SHM 三文件之和
        let main = std::fs::metadata(dir.join("craftwork.db")).map(|m| m.len()).unwrap_or(0);
        let wal = std::fs::metadata(dir.join("craftwork.db-wal")).map(|m| m.len()).unwrap_or(0);
        let shm = std::fs::metadata(dir.join("craftwork.db-shm")).map(|m| m.len()).unwrap_or(0);
        assert!(wal < 128 * 1024, "VACUUM 后 WAL 应归零（实际 {wal} 字节），否则占用会双副本翻倍");
        assert_eq!(
            db.disk_usage_bytes(),
            main + wal + shm,
            "disk_usage_bytes 应等于主库+WAL+SHM 三文件之和"
        );
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backups_and_attachments_clear_whole_directory() {
        let dir = temp_dir("flat");
        let db = test_db(&dir);
        for sub in ["backups", "attachments"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            std::fs::write(dir.join(sub).join("a.bin"), b"123").unwrap();
            std::fs::write(dir.join(sub).join("b.bin"), b"45").unwrap();
        }
        let backups = cleanup(&db, &dir.to_string_lossy(), CleanupKind::BackupsAll, &[], 0).unwrap();
        assert_eq!(backups.removed_items, 2);
        assert_eq!(backups.freed_bytes, 5);
        let attachments = cleanup(&db, &dir.to_string_lossy(), CleanupKind::AttachmentsAll, &[], 0).unwrap();
        assert_eq!(attachments.removed_items, 2);
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 分类顺序即界面顺序：可清理的在前，只读的「其他」在最后。
    #[test]
    fn overview_lists_every_category_with_cleanups_where_allowed() {
        let dir = temp_dir("overview");
        let db = test_db(&dir);
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::write(dir.join("logs/app.log"), b"1234").unwrap();

        let view = overview(&db, &dir.to_string_lossy(), &[], 0).unwrap();
        assert_eq!(view.categories.len(), CATEGORIES.len() + 1);
        assert_eq!(view.categories.last().unwrap().kind, OTHER_KIND, "「其他」必须垫底");
        let logs = view.categories.iter().find(|c| c.kind == "logs").unwrap();
        assert_eq!(logs.bytes, 4);
        assert_eq!(logs.cleanups.len(), 2, "日志有孤儿与全量两种清理");
        let skills = view.categories.iter().find(|c| c.kind == "skillsMarketplace").unwrap();
        assert!(skills.cleanups.is_empty(), "技能商店缓存不提供清理按钮（D12 回滚资产）");
        assert!(view.busy_reason.is_empty());
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 记账恒等式：各分类体积之和必须等于数据目录里真实文件的字节数（不含数据库本体），
    /// 且「其他」不能把已分类的目录重复计一遍（`workspace/default` 是嵌套分类，最容易漏扣）。
    #[test]
    fn overview_categories_sum_matches_actual_disk_usage() {
        let dir = temp_dir("sum");
        let db = test_db(&dir);
        // 造一套贴近真实的数据目录
        let write = |rel: &str, bytes: usize| {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, vec![b'x'; bytes]).unwrap();
        };
        write("backups/a.bak", 100);
        write("logs/th_a.log", 20);
        write("attachments/img.png", 30);
        write("shell-tasks/th_a/out.log", 40);
        write("worktrees/th_a/src/main.rs", 50);
        write("plans/plan_a.md", 60);
        write("skills-marketplace/cache/x/1.0/skill.md", 70);
        write("workspace/default/note.txt", 80);
        write("keys.json", 90);

        let view = overview(&db, &dir.to_string_lossy(), &[], 0).unwrap();
        let classified: u64 = view.categories.iter().map(|c| c.bytes).sum();
        // 真实占用：把数据目录整棵量一遍，减去数据库三个文件
        let total = dir_usage(&dir, 0).bytes;
        let db_files = ["craftwork.db", "craftwork.db-wal", "craftwork.db-shm"]
            .iter()
            .filter_map(|n| std::fs::metadata(dir.join(n)).ok())
            .map(|m| m.len())
            .sum::<u64>();
        assert_eq!(
            classified,
            total - db_files,
            "分类之和必须等于数据目录真实占用（无重复计、无遗漏）；got {classified} vs {}",
            total - db_files
        );
        // 分类各自准确
        let get = |kind: &str| view.categories.iter().find(|c| c.kind == kind).unwrap().bytes;
        assert_eq!(get("backups"), 100);
        assert_eq!(get("logs"), 20);
        assert_eq!(get("attachments"), 30);
        assert_eq!(get("shellTasks"), 40);
        assert_eq!(get("worktrees"), 50);
        assert_eq!(get("plans"), 60);
        assert_eq!(get("skillsMarketplace"), 70);
        assert_eq!(get("workspaceDefault"), 80, "嵌套分类必须单独统计");
        assert_eq!(get("other"), 90, "「其他」只含未分类项（keys.json），不得重复计 workspace/default");
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 清理后体积确实下降：overview 前后对比（这是用户判断「清理有没有用」的唯一依据）。
    #[test]
    fn cleanup_reduces_reported_usage() {
        let dir = temp_dir("reduces");
        let db = test_db(&dir);
        std::fs::create_dir_all(dir.join("backups")).unwrap();
        std::fs::write(dir.join("backups/big.bin"), vec![b'x'; 1000]).unwrap();
        let before = overview(&db, &dir.to_string_lossy(), &[], 0).unwrap().files_total_bytes;
        assert!(before >= 1000);

        cleanup(&db, &dir.to_string_lossy(), CleanupKind::BackupsAll, &[], 0).unwrap();
        let after = overview(&db, &dir.to_string_lossy(), &[], 0).unwrap().files_total_bytes;
        assert!(after < before, "清理后报告体积必须下降：{before} → {after}");
        assert_eq!(before - after, 1000, "下降量应等于被删的字节数");
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 删除会话的文件清理只认精确路径：同名前缀的其他文件不受牵连。
    #[test]
    fn remove_thread_files_uses_exact_names_only() {
        let dir = temp_dir("threadfiles");
        for sub in ["logs", "shell-tasks", "worktrees", "plans"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join("logs/th_a.log"), b"a").unwrap();
        std::fs::write(dir.join("logs/th_ab.log"), b"ab").unwrap();
        std::fs::create_dir_all(dir.join("shell-tasks/th_a")).unwrap();
        std::fs::create_dir_all(dir.join("worktrees/th_a")).unwrap();
        std::fs::write(dir.join("plans/plan_a.md"), b"p").unwrap();

        let failures = remove_thread_files(
            &dir.to_string_lossy(),
            "th_a",
            &[dir.join("plans/plan_a.md").to_string_lossy().to_string()],
        );
        assert!(failures.is_empty(), "{failures:?}");
        assert!(!dir.join("logs/th_a.log").exists());
        assert!(dir.join("logs/th_ab.log").exists(), "前缀相同的其他会话日志不得误删");
        assert!(!dir.join("shell-tasks/th_a").exists());
        assert!(!dir.join("worktrees/th_a").exists());
        assert!(!dir.join("plans/plan_a.md").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
