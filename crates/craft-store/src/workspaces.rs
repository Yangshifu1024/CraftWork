//! workspaces + workspace_folders 表读写（src/main/store/workspaces.ts 的直译）。

use rusqlite::{Connection, OptionalExtension};

use crate::db::{not_found, now_millis, other_err, Db, StoreError, StoreResult};
use craft_core::ids::{id_prefix, new_id};

#[derive(Debug, Clone)]
pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub path: String,
    pub trust: String,
    pub purpose: String,
    pub created_at: i64,
    pub last_opened_at: i64,
}

#[derive(Debug, Clone)]
pub struct WorkspaceFolderRow {
    pub id: String,
    pub workspace_id: String,
    pub path: String,
    pub position: i64,
}

#[derive(Debug, Clone)]
pub struct WorkspaceWithFolders {
    pub workspace: WorkspaceRow,
    pub folders: Vec<WorkspaceFolderRow>,
}

fn to_workspace(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRow> {
    Ok(WorkspaceRow {
        id: row.get("id")?,
        name: row.get("name")?,
        path: row.get("path")?,
        trust: row.get::<_, Option<String>>("trust")?.unwrap_or_else(|| "restricted".into()),
        purpose: row.get::<_, Option<String>>("purpose")?.unwrap_or_else(|| "project".into()),
        created_at: row.get("created_at")?,
        last_opened_at: row.get("last_opened_at")?,
    })
}

fn to_folder(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceFolderRow> {
    Ok(WorkspaceFolderRow {
        id: row.get("id")?,
        workspace_id: row.get("workspace_id")?,
        path: row.get("path")?,
        position: row.get("position")?,
    })
}

/// 按 path 唯一 upsert：已存在沿用原 id，只更新 name/trust/last_opened_at；
/// purpose 由首次创建决定不更新（临时目录语义不可被项目覆盖）。
pub fn upsert_workspace(conn: &Connection, name: &str, path: &str, trust: &str, purpose: &str) -> StoreResult<WorkspaceRow> {
    let now = now_millis();
    let existing = conn
        .query_row("SELECT * FROM workspaces WHERE path = ?1", [path], to_workspace)
        .optional()
        .map_err(other_err)?;
    if let Some(row) = existing {
        conn.execute(
            "UPDATE workspaces SET name = ?1, trust = ?2, last_opened_at = ?3 WHERE id = ?4",
            rusqlite::params![name, trust, now, row.id],
        )
        .map_err(other_err)?;
        return Ok(WorkspaceRow {
            name: name.to_string(),
            trust: trust.to_string(),
            last_opened_at: now,
            ..row
        });
    }
    let id = new_id(id_prefix::WORKSPACE);
    conn.execute(
        "INSERT INTO workspaces (id, name, path, trust, purpose, created_at, last_opened_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, name, path, trust, purpose, now, now],
    )
    .map_err(other_err)?;
    Ok(WorkspaceRow {
        id,
        name: name.into(),
        path: path.into(),
        trust: trust.into(),
        purpose: purpose.into(),
        created_at: now,
        last_opened_at: now,
    })
}

pub fn list_workspaces(conn: &Connection) -> StoreResult<Vec<WorkspaceWithFolders>> {
    let mut stmt = conn.prepare("SELECT * FROM workspaces ORDER BY last_opened_at DESC").map_err(other_err)?;
    let rows = stmt
        .query_map([], to_workspace)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    let mut result = Vec::with_capacity(rows.len());
    for ws in rows {
        let folders = list_workspace_folders(conn, &ws.id)?;
        result.push(WorkspaceWithFolders { workspace: ws, folders });
    }
    Ok(result)
}

pub fn get_workspace(conn: &Connection, id: &str) -> StoreResult<WorkspaceWithFolders> {
    let ws = conn
        .query_row("SELECT * FROM workspaces WHERE id = ?1", [id], to_workspace)
        .optional()
        .map_err(other_err)?
        .ok_or_else(|| StoreError::NotFound("工作区不存在".into()))?;
    let folders = list_workspace_folders(conn, &ws.id)?;
    Ok(WorkspaceWithFolders { workspace: ws, folders })
}

/// 读取工作区的源文件夹；兜底保证第一条是 workspaces.path 指向的主目录
/// （历史数据或异常删除时自动补一条，避免路径解析失去主根）。
pub fn list_workspace_folders(conn: &Connection, workspace_id: &str) -> StoreResult<Vec<WorkspaceFolderRow>> {
    let main_path: String = conn
        .query_row("SELECT path FROM workspaces WHERE id = ?1", [workspace_id], |row| row.get(0))
        .optional()
        .map_err(other_err)?
        .unwrap_or_default();
    let mut stmt = conn
        .prepare("SELECT * FROM workspace_folders WHERE workspace_id = ?1 ORDER BY position")
        .map_err(other_err)?;
    let mut folders: Vec<WorkspaceFolderRow> = stmt
        .query_map([workspace_id], to_folder)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    if !main_path.is_empty() && !folders.iter().any(|f| f.path == main_path) {
        let added = WorkspaceFolderRow {
            id: new_id(id_prefix::WORKSPACE_FOLDER),
            workspace_id: workspace_id.to_string(),
            path: main_path.clone(),
            position: 0,
        };
        conn.execute(
            "INSERT OR IGNORE INTO workspace_folders (id, workspace_id, path, position, created_at) VALUES (?1, ?2, ?3, 0, ?4)",
            rusqlite::params![added.id, workspace_id, main_path, now_millis()],
        )
        .map_err(other_err)?;
        folders.insert(0, added);
    }
    Ok(folders)
}

/// 事务内整体替换文件夹列表，并同步 workspaces.path = paths[0]。
pub fn replace_workspace_folders(db: &Db, workspace_id: &str, paths: &[String]) -> StoreResult<()> {
    db.transaction(|conn| {
        conn.execute("DELETE FROM workspace_folders WHERE workspace_id = ?1", [workspace_id])
            .map_err(other_err)?;
        for (i, p) in paths.iter().enumerate() {
            conn.execute(
                "INSERT INTO workspace_folders (id, workspace_id, path, position, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![new_id(id_prefix::WORKSPACE_FOLDER), workspace_id, p, i as i64, now_millis()],
            )
            .map_err(other_err)?;
        }
        if let Some(main_path) = paths.first() {
            conn.execute("UPDATE workspaces SET path = ?1 WHERE id = ?2", rusqlite::params![main_path, workspace_id])
                .map_err(other_err)?;
        }
        Ok(())
    })
}

pub fn update_workspace_trust(conn: &Connection, id: &str, trust: &str) -> StoreResult<()> {
    let changes = conn
        .execute("UPDATE workspaces SET trust = ?1 WHERE id = ?2", rusqlite::params![trust, id])
        .map_err(other_err)?;
    if changes == 0 {
        return not_found("工作区不存在");
    }
    Ok(())
}

pub fn update_workspace_name(conn: &Connection, id: &str, name: &str) -> StoreResult<()> {
    let changes = conn
        .execute("UPDATE workspaces SET name = ?1 WHERE id = ?2", rusqlite::params![name, id])
        .map_err(other_err)?;
    if changes == 0 {
        return not_found("工作区不存在");
    }
    Ok(())
}

pub fn update_workspace_purpose(conn: &Connection, id: &str, purpose: &str) -> StoreResult<()> {
    conn.execute("UPDATE workspaces SET purpose = ?1 WHERE id = ?2", rusqlite::params![purpose, id])
        .map_err(other_err)?;
    Ok(())
}

/// 历史无工作区会话统一归入临时目录（幂等）。
pub fn backfill_thread_workspaces(conn: &Connection, workspace_id: &str) -> StoreResult<()> {
    conn.execute("UPDATE threads SET workspace_id = ?1 WHERE workspace_id = ''", [workspace_id])
        .map_err(other_err)?;
    Ok(())
}

/// 内置临时会话目录的工作区 id（purpose=conversation；启动 ensure 后必然存在，取不到回落空串）。
pub fn default_conversation_workspace_id(conn: &Connection) -> String {
    conn.query_row(
        "SELECT id FROM workspaces WHERE purpose = 'conversation' ORDER BY created_at LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or_default()
}

pub fn delete_workspace(db: &Db, id: &str) -> StoreResult<()> {
    db.transaction(|conn| {
        conn.execute("DELETE FROM workspace_folders WHERE workspace_id = ?1", [id]).map_err(other_err)?;
        conn.execute("DELETE FROM workspaces WHERE id = ?1", [id]).map_err(other_err)?;
        Ok(())
    })
}
