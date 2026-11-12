//! threads 表读写。
//! deleted_at 为软删标记列（历史兼容）：所有读取一律过滤 deleted_at IS NULL；
//! 当前实现只提供物理删除（delete_thread），没有写入软删标记的路径。

use rusqlite::Connection;

use crate::db::{not_found, now_millis, other_err, Db, StoreError, StoreResult};
use craft_core::ids::{id_prefix, new_id};

#[derive(Debug, Clone)]
pub struct ThreadRow {
    pub id: String,
    pub mode: String,
    pub title: String,
    /// 自动命名标记：1 表示标题仍是自动生成（可被 auto_title_thread 覆盖）；手动改名即清零
    pub title_auto: bool,
    pub model_id: String,
    /// 思考档位（空串=未选择，请求不发 reasoning 参数）
    pub reasoning_level: String,
    pub workspace_id: String,
    /// 绑定的 git worktree 目录（空串 = 未绑定）；绑定后该会话所有 run 以它为主要目录
    pub worktree_path: String,
    /// 绑定的 worktree 分支名（craftwork/<会话id 短码>；与 worktree_path 同步落库）
    pub worktree_branch: String,
    pub status: String,
    pub archived: bool,
    pub tokens: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// 父会话 id（空串 = 主会话）。决策 D15：子代理是一个子会话，字段承载归属。
    pub parent_thread_id: String,
    /// 子代理可寻址短名（空串 = 非子代理会话）
    pub agent_name: String,
    /// 子代理类型（general / explore；空串 = 非子代理会话）
    pub agent_type: String,
    /// 子代理嵌套深度（0 = 主会话，一期子代理恒为 1）
    pub agent_depth: i64,
    /// 发起该子代理的工具调用 id（空串 = 非子代理会话）
    pub spawn_call_id: String,
    /// 子代理生效档案的 JSON 快照（自定义子代理派发时写入；空串 = 内置类型或老数据）。
    /// 决策 D15 扩展：定义可被用户改删，历史子会话据此仍能解释「当时按什么跑的」。
    pub agent_profile: String,
    /// 最近一次正常完成的时刻（0 = 从未完成）；取消/失败不写
    pub last_done_at: i64,
    /// 用户最后一次打开该会话的时刻（0 = 从未打开）
    pub last_read_at: i64,
}

const COLS: &str = "id, mode, title, title_auto, model_id, reasoning_level, workspace_id, worktree_path, worktree_branch, status, archived, tokens, created_at, updated_at, parent_thread_id, agent_name, agent_type, agent_depth, spawn_call_id, agent_profile, last_done_at, last_read_at";

fn to_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadRow> {
    Ok(ThreadRow {
        id: row.get("id")?,
        mode: row.get("mode")?,
        title: row.get("title")?,
        title_auto: row.get::<_, i64>("title_auto")? == 1,
        model_id: row.get("model_id")?,
        reasoning_level: row.get("reasoning_level")?,
        workspace_id: row.get("workspace_id")?,
        worktree_path: row.get("worktree_path")?,
        worktree_branch: row.get("worktree_branch")?,
        status: row.get("status")?,
        archived: row.get::<_, i64>("archived")? == 1,
        tokens: row.get("tokens")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        parent_thread_id: row.get("parent_thread_id")?,
        agent_name: row.get("agent_name")?,
        agent_type: row.get("agent_type")?,
        agent_depth: row.get("agent_depth")?,
        spawn_call_id: row.get("spawn_call_id")?,
        agent_profile: row.get("agent_profile")?,
        last_done_at: row.get("last_done_at")?,
        last_read_at: row.get("last_read_at")?,
    })
}

/// 创建会话的入参（tokens/created_at/updated_at 由存储层填充）。
pub struct NewThread<'a> {
    pub id: &'a str,
    pub mode: &'a str,
    pub title: &'a str,
    pub title_auto: bool,
    pub model_id: &'a str,
    pub reasoning_level: &'a str,
    pub workspace_id: &'a str,
    pub status: &'a str,
    pub archived: bool,
}

/// 创建会话（created_at = updated_at = 当前时间，tokens 归零；worktree 未绑定走列默认空串）。
pub fn create_thread(conn: &Connection, t: NewThread<'_>) -> StoreResult<ThreadRow> {
    let now = now_millis();
    conn.execute(
        "INSERT INTO threads (id, mode, title, title_auto, model_id, reasoning_level, workspace_id, status, archived, tokens, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11)",
        rusqlite::params![
            t.id,
            t.mode,
            t.title,
            t.title_auto as i64,
            t.model_id,
            t.reasoning_level,
            t.workspace_id,
            t.status,
            t.archived as i64,
            now,
            now,
        ],
    )
    .map_err(other_err)?;
    Ok(ThreadRow {
        id: t.id.to_string(),
        mode: t.mode.to_string(),
        title: t.title.to_string(),
        title_auto: t.title_auto,
        model_id: t.model_id.to_string(),
        reasoning_level: t.reasoning_level.to_string(),
        workspace_id: t.workspace_id.to_string(),
        worktree_path: String::new(),
        worktree_branch: String::new(),
        status: t.status.to_string(),
        archived: t.archived,
        tokens: 0,
        created_at: now,
        updated_at: now,
        parent_thread_id: String::new(),
        agent_name: String::new(),
        agent_type: String::new(),
        agent_depth: 0,
        spawn_call_id: String::new(),
        agent_profile: String::new(),
        last_done_at: 0,
        last_read_at: 0,
    })
}

/// 创建子代理子会话（决策 D15）：与普通会话共享全部列，额外写入归属与档案字段。
/// 子会话不进侧栏 / 搜索（读取侧统一按 `parent_thread_id = ''` 过滤），也不参与草稿槽。
pub struct NewSubagentThread<'a> {
    pub id: &'a str,
    pub mode: &'a str,
    pub title: &'a str,
    pub model_id: &'a str,
    pub reasoning_level: &'a str,
    pub workspace_id: &'a str,
    pub parent_thread_id: &'a str,
    pub agent_name: &'a str,
    pub agent_type: &'a str,
    pub spawn_call_id: &'a str,
    /// 生效档案的 JSON 快照（内置类型传空串）
    pub agent_profile: &'a str,
}

/// 创建子会话（status 初始 idle、tokens 归零、agent_depth 恒为 1）。
pub fn create_subagent_thread(conn: &Connection, t: NewSubagentThread<'_>) -> StoreResult<ThreadRow> {
    let now = now_millis();
    conn.execute(
        "INSERT INTO threads (id, mode, title, title_auto, model_id, reasoning_level, workspace_id, status, archived, tokens, \
         created_at, updated_at, parent_thread_id, agent_name, agent_type, agent_depth, spawn_call_id, agent_profile) \
         VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, 'idle', 0, 0, ?7, ?7, ?8, ?9, ?10, 1, ?11, ?12)",
        rusqlite::params![
            t.id,
            t.mode,
            t.title,
            t.model_id,
            t.reasoning_level,
            t.workspace_id,
            now,
            t.parent_thread_id,
            t.agent_name,
            t.agent_type,
            t.spawn_call_id,
            t.agent_profile,
        ],
    )
    .map_err(other_err)?;
    Ok(ThreadRow {
        id: t.id.to_string(),
        mode: t.mode.to_string(),
        title: t.title.to_string(),
        title_auto: false,
        model_id: t.model_id.to_string(),
        reasoning_level: t.reasoning_level.to_string(),
        workspace_id: t.workspace_id.to_string(),
        worktree_path: String::new(),
        worktree_branch: String::new(),
        status: "idle".into(),
        archived: false,
        tokens: 0,
        created_at: now,
        updated_at: now,
        parent_thread_id: t.parent_thread_id.to_string(),
        agent_name: t.agent_name.to_string(),
        agent_type: t.agent_type.to_string(),
        agent_depth: 1,
        spawn_call_id: t.spawn_call_id.to_string(),
        agent_profile: t.agent_profile.to_string(),
        last_done_at: 0,
        last_read_at: 0,
    })
}

/// 某父会话名下的全部子会话（创建序升序；子代理注册表重建与状态列表用）。
pub fn list_subagent_threads(conn: &Connection, parent_thread_id: &str) -> StoreResult<Vec<ThreadRow>> {
    let sql = format!(
        "SELECT {COLS} FROM threads WHERE deleted_at IS NULL AND parent_thread_id = ?1 ORDER BY created_at ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(other_err)?;
    let rows = stmt
        .query_map([parent_thread_id], to_thread)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 按 id 读取会话（软删行不可见；不存在返回 NotFound）。
pub fn get_thread(conn: &Connection, id: &str) -> StoreResult<ThreadRow> {
    let row = conn
        .query_row(
            &format!("SELECT {COLS} FROM threads WHERE id = ?1 AND deleted_at IS NULL"),
            [id],
            to_thread,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound("会话不存在".into()),
            other_err => StoreError::Other(other_err.to_string()),
        })?;
    Ok(row)
}

/// 列出指定模式的会话（updated_at DESC；include_archived 控制是否含归档）。
/// 只返回主会话：子代理子会话（parent_thread_id 非空）不进侧栏列表。
pub fn list_threads(conn: &Connection, mode: &str, include_archived: bool) -> StoreResult<Vec<ThreadRow>> {
    let archived_clause = if include_archived { "" } else { "AND archived = 0" };
    let sql = format!(
        "SELECT {COLS} FROM threads WHERE deleted_at IS NULL AND mode = ?1 AND parent_thread_id = '' {archived_clause} ORDER BY updated_at DESC"
    );
    let mut stmt = conn.prepare(&sql).map_err(other_err)?;
    let rows = stmt
        .query_map([mode], to_thread)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 列出工作区下全部会话（含归档；项目删除前快照用，供逐会话清理绑定的 worktree）。
pub fn list_threads_by_workspace(conn: &Connection, workspace_id: &str) -> StoreResult<Vec<ThreadRow>> {
    let sql = format!("SELECT {COLS} FROM threads WHERE deleted_at IS NULL AND workspace_id = ?1 ORDER BY updated_at DESC");
    let mut stmt = conn.prepare(&sql).map_err(other_err)?;
    let rows = stmt
        .query_map([workspace_id], to_thread)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 列出库里全部会话行（含归档、含子代理子会话；存储统计与孤儿判定用）。
///
/// 与 [`list_threads`] 的差别只在可见范围：那条服务侧栏（只给主会话、默认不含归档），
/// 这条给「磁盘上的东西属于谁」这类问题——子会话的日志与后台输出同样是活着的文件。
pub fn list_all_threads(conn: &Connection) -> StoreResult<Vec<ThreadRow>> {
    let sql = format!("SELECT {COLS} FROM threads WHERE deleted_at IS NULL ORDER BY updated_at DESC");
    let mut stmt = conn.prepare(&sql).map_err(other_err)?;
    let rows = stmt
        .query_map([], to_thread)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 标题或事件内容匹配的搜索（LIKE + DISTINCT + LIMIT 100）。只搜主会话（子代理子会话不暴露）。
pub fn search_threads(conn: &Connection, mode: &str, query: &str) -> StoreResult<Vec<ThreadRow>> {
    let like = format!("%{query}%");
    let cols = COLS
        .split(", ")
        .map(|c| format!("t.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT DISTINCT {cols}
         FROM threads t
         LEFT JOIN events e ON e.thread_id = t.id
         WHERE t.deleted_at IS NULL AND t.mode = ?1 AND t.parent_thread_id = '' AND (t.title LIKE ?2 OR e.payload LIKE ?3)
         ORDER BY t.updated_at DESC LIMIT 100"
    );
    let mut stmt = conn.prepare(&sql).map_err(other_err)?;
    let rows = stmt
        .query_map([mode, like.as_str(), like.as_str()], to_thread)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 手动改名：置 title_auto=0，此后自动命名不再覆盖。目标不存在返回 NotFound。
pub fn rename_thread(conn: &Connection, id: &str, title: &str) -> StoreResult<()> {
    let changes = conn
        .execute(
            "UPDATE threads SET title = ?1, title_auto = 0, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            rusqlite::params![title, now_millis(), id],
        )
        .map_err(other_err)?;
    if changes == 0 {
        return not_found("会话不存在");
    }
    Ok(())
}

/// 条件更新自动命名：仅当仍是自动命名会话时生效（rename_thread 置 title_auto=0 保护手动改名）。
/// 命名落地后翻转 title_auto=0，后续 run 不再重复命名。返回是否实际更新。
pub fn auto_title_thread(conn: &Connection, id: &str, title: &str) -> StoreResult<bool> {
    let changes = conn
        .execute(
            "UPDATE threads SET title = ?1, title_auto = 0, updated_at = ?2 WHERE id = ?3 AND title_auto = 1 AND deleted_at IS NULL",
            rusqlite::params![title, now_millis(), id],
        )
        .map_err(other_err)?;
    Ok(changes > 0)
}

pub fn set_thread_status(conn: &Connection, id: &str, status: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE threads SET status = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![status, now_millis(), id],
    )
    .map_err(other_err)?;
    Ok(())
}

/// 标记「本轮正常完成」：写完成时刻，未读判定由它对比 last_read_at 得出。
/// 刻意不动 updated_at：更新时间用于列表排序，`complete()` 紧随其后还会广播 thread.updated，
/// 这里只补未读所需的时刻，避免与状态写入互相覆盖排序依据。
pub fn mark_thread_done(conn: &Connection, id: &str, done_at: i64) -> StoreResult<()> {
    conn.execute("UPDATE threads SET last_done_at = ?1 WHERE id = ?2", rusqlite::params![done_at, id])
        .map_err(other_err)?;
    Ok(())
}

/// 标记「用户已读该会话」：写阅读时刻。同样不动 updated_at（浏览别人的会话不该把它顶到列表首位）。
/// `read_at` 取 now：比会话内最后一条事件的时刻更晚，保证读完即清零未读；
/// 时钟回拨导致 read_at 落后时，下一次打开会再次补写，未读最多多留一轮，不会永久卡住。
pub fn mark_thread_read(conn: &Connection, id: &str) -> StoreResult<()> {
    conn.execute("UPDATE threads SET last_read_at = ?1 WHERE id = ?2", rusqlite::params![now_millis(), id])
        .map_err(other_err)?;
    Ok(())
}

pub fn set_thread_model(conn: &Connection, id: &str, model_id: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE threads SET model_id = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![model_id, now_millis(), id],
    )
    .map_err(other_err)?;
    Ok(())
}

/// 回写子代理短名（注册表分配最终名——同名加序号后，子会话档案要与注册表一致）。
pub fn set_thread_agent_name(conn: &Connection, id: &str, agent_name: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE threads SET agent_name = ?1 WHERE id = ?2",
        rusqlite::params![agent_name, id],
    )
    .map_err(other_err)?;
    Ok(())
}

pub fn set_thread_reasoning(conn: &Connection, id: &str, level: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE threads SET reasoning_level = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![level, now_millis(), id],
    )
    .map_err(other_err)?;
    Ok(())
}

/// 绑定 git worktree（路径与分支同步落库；绑定后不可解绑，删除会话时清理）。
pub fn set_thread_worktree(conn: &Connection, id: &str, path: &str, branch: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE threads SET worktree_path = ?1, worktree_branch = ?2, updated_at = ?3 WHERE id = ?4",
        rusqlite::params![path, branch, now_millis(), id],
    )
    .map_err(other_err)?;
    Ok(())
}

pub fn set_thread_archived(conn: &Connection, id: &str, archived: bool) -> StoreResult<()> {
    conn.execute(
        "UPDATE threads SET archived = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![archived as i64, now_millis(), id],
    )
    .map_err(other_err)?;
    Ok(())
}

pub fn touch_thread(conn: &Connection, id: &str) -> StoreResult<()> {
    conn.execute("UPDATE threads SET updated_at = ?1 WHERE id = ?2", rusqlite::params![now_millis(), id])
        .map_err(other_err)?;
    Ok(())
}

pub fn add_thread_tokens(conn: &Connection, id: &str, tokens: i64) -> StoreResult<()> {
    if tokens == 0 {
        return Ok(());
    }
    conn.execute("UPDATE threads SET tokens = tokens + ?1 WHERE id = ?2", rusqlite::params![tokens, id])
        .map_err(other_err)?;
    Ok(())
}

/// 物理删除：事务内删 events + approvals + runs + plans + threads。
/// 级联：连同该会话名下的子代理子会话（决策 D15）一起删除——子会话脱离父会话没有入口。
pub fn delete_thread(db: &Db, id: &str) -> StoreResult<()> {
    db.transaction(|conn| {
        // 先收集全部待删会话 id（父 + 子），再逐表清理，避免残留孤儿行
        let mut ids: Vec<String> = vec![id.to_string()];
        let children: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT id FROM threads WHERE parent_thread_id = ?1")
                .map_err(other_err)?;
            stmt.query_map([id], |row| row.get::<_, String>(0))
                .map_err(other_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(other_err)?
        };
        ids.extend(children);
        for target in &ids {
            conn.execute("DELETE FROM events WHERE thread_id = ?1", [target]).map_err(other_err)?;
            conn.execute("DELETE FROM approvals WHERE thread_id = ?1", [target]).map_err(other_err)?;
            conn.execute("DELETE FROM runs WHERE thread_id = ?1", [target]).map_err(other_err)?;
            conn.execute("DELETE FROM plans WHERE thread_id = ?1", [target]).map_err(other_err)?;
            conn.execute("DELETE FROM threads WHERE id = ?1", [target]).map_err(other_err)?;
        }
        Ok(())
    })
}

/// 删除工作区下全部会话（事件 / 审批 / 运行 / 计划一并清理），返回删除数。项目删除时级联调用。
/// 只统计主会话；子代理子会话随其父会话一起删除（delete_thread 级联），不单独计数。
pub fn delete_threads_by_workspace(db: &Db, workspace_id: &str) -> StoreResult<usize> {
    let ids: Vec<String> = db.with(|conn| {
        let mut stmt = conn
            .prepare("SELECT id FROM threads WHERE workspace_id = ?1 AND parent_thread_id = ''")
            .map_err(other_err)?;
        let rows = stmt
            .query_map([workspace_id], |row| row.get::<_, String>(0))
            .map_err(other_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(other_err)?;
        Ok(rows)
    })?;
    let mut count = 0;
    for id in &ids {
        delete_thread(db, id)?;
        count += 1;
    }
    Ok(count)
}

/// 供测试与服务层组装用的线程 id 生成。
pub fn new_thread_id() -> String {
    new_id(id_prefix::THREAD)
}
