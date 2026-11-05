//! threads 表读写。

use rusqlite::Connection;

use crate::db::{not_found, now_millis, other_err, Db, StoreError, StoreResult};
use craft_core::ids::{id_prefix, new_id};

#[derive(Debug, Clone)]
pub struct ThreadRow {
    pub id: String,
    pub mode: String,
    pub title: String,
    pub title_auto: bool,
    pub model_id: String,
    pub workspace_id: String,
    pub status: String,
    pub archived: bool,
    pub tokens: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLS: &str = "id, mode, title, title_auto, model_id, workspace_id, status, archived, tokens, created_at, updated_at";

fn to_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadRow> {
    Ok(ThreadRow {
        id: row.get("id")?,
        mode: row.get("mode")?,
        title: row.get("title")?,
        title_auto: row.get::<_, i64>("title_auto")? == 1,
        model_id: row.get("model_id")?,
        workspace_id: row.get("workspace_id")?,
        status: row.get("status")?,
        archived: row.get::<_, i64>("archived")? == 1,
        tokens: row.get("tokens")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub struct NewThread<'a> {
    pub id: &'a str,
    pub mode: &'a str,
    pub title: &'a str,
    pub title_auto: bool,
    pub model_id: &'a str,
    pub workspace_id: &'a str,
    pub status: &'a str,
    pub archived: bool,
}

pub fn create_thread(conn: &Connection, t: NewThread<'_>) -> StoreResult<ThreadRow> {
    let now = now_millis();
    conn.execute(
        &format!("INSERT INTO threads ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10)"),
        rusqlite::params![
            t.id,
            t.mode,
            t.title,
            t.title_auto as i64,
            t.model_id,
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
        workspace_id: t.workspace_id.to_string(),
        status: t.status.to_string(),
        archived: t.archived,
        tokens: 0,
        created_at: now,
        updated_at: now,
    })
}

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

pub fn list_threads(conn: &Connection, mode: &str, include_archived: bool) -> StoreResult<Vec<ThreadRow>> {
    let archived_clause = if include_archived { "" } else { "AND archived = 0" };
    let sql = format!(
        "SELECT {COLS} FROM threads WHERE deleted_at IS NULL AND mode = ?1 {archived_clause} ORDER BY updated_at DESC"
    );
    let mut stmt = conn.prepare(&sql).map_err(other_err)?;
    let rows = stmt
        .query_map([mode], to_thread)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 标题或事件内容匹配的搜索（LIKE + DISTINCT + LIMIT 100）。
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
         WHERE t.deleted_at IS NULL AND t.mode = ?1 AND (t.title LIKE ?2 OR e.payload LIKE ?3)
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

pub fn set_thread_model(conn: &Connection, id: &str, model_id: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE threads SET model_id = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![model_id, now_millis(), id],
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

/// 物理删除：事务内删 events + approvals + runs + threads。
pub fn delete_thread(db: &Db, id: &str) -> StoreResult<()> {
    db.transaction(|conn| {
        conn.execute("DELETE FROM events WHERE thread_id = ?1", [id]).map_err(other_err)?;
        conn.execute("DELETE FROM approvals WHERE thread_id = ?1", [id]).map_err(other_err)?;
        conn.execute("DELETE FROM runs WHERE thread_id = ?1", [id]).map_err(other_err)?;
        conn.execute("DELETE FROM threads WHERE id = ?1", [id]).map_err(other_err)?;
        Ok(())
    })
}

/// 删除工作区下全部会话（事件 / 审批 / 运行一并清理），返回删除数。项目删除时级联调用。
pub fn delete_threads_by_workspace(db: &Db, workspace_id: &str) -> StoreResult<usize> {
    let ids: Vec<String> = db.with(|conn| {
        let mut stmt = conn
            .prepare("SELECT id FROM threads WHERE workspace_id = ?1")
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
