//! 计划数据（计划模式产物）：发送计划模式消息时建行（approved=0、file_path 空），
//! 计划 run 正常完成后把最终正文落盘到 `~/.craftwork/plans/` 并回填 file_path。
//! 前端「批准并执行」提示条以「file_path 非空且未批准」为准——中断/失败无文件不出现。

use std::path::PathBuf;

use rusqlite::params;
use serde_json::json;

use crate::db::{now_millis, other_err, Db, StoreResult};
use craft_core::contract::PlanInfo;
use craft_core::ids::{id_prefix, new_id};

/// 当前会话的最新计划（按创建时间倒序取一）；无计划返回 None。
pub fn latest_plan(conn: &rusqlite::Connection, thread_id: &str) -> StoreResult<Option<PlanInfo>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, thread_id, run_id, file_path, approved, ignored, updated_at FROM plans \
             WHERE thread_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .map_err(other_err)?;
    let mut rows = stmt
        .query_map([thread_id], |row| {
            Ok(PlanInfo {
                id: row.get(0)?,
                thread_id: row.get(1)?,
                run_id: row.get(2)?,
                file_path: row.get(3)?,
                approved: row.get::<_, i64>(4)? != 0,
                ignored: row.get::<_, i64>(5)? != 0,
                updated_at: row.get(6)?,
            })
        })
        .map_err(other_err)?;
    match rows.next() {
        Some(row) => Ok(Some(row.map_err(other_err)?)),
        None => Ok(None),
    }
}

/// 新建计划行：批准状态 false、无文件（发送计划模式消息时调用）。
pub fn create_plan(conn: &rusqlite::Connection, thread_id: &str, run_id: &str) -> StoreResult<PlanInfo> {
    let id = new_id(id_prefix::PLAN);
    let now = now_millis();
    conn.execute(
        "INSERT INTO plans (id, thread_id, run_id, file_path, approved, ignored, created_at, updated_at) \
         VALUES (?1, ?2, ?3, '', 0, 0, ?4, ?4)",
        params![id, thread_id, run_id, now],
    )
    .map_err(other_err)?;
    Ok(PlanInfo {
        id,
        thread_id: thread_id.to_string(),
        run_id: Some(run_id.to_string()),
        file_path: String::new(),
        approved: false,
        ignored: false,
        updated_at: now,
    })
}

/// 回填计划文件路径（计划 run 正常完成、正文落盘后调用）。
pub fn set_plan_file(conn: &rusqlite::Connection, plan_id: &str, path: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE plans SET file_path = ?2, updated_at = ?3 WHERE id = ?1",
        params![plan_id, path, now_millis()],
    )
    .map_err(other_err)?;
    Ok(())
}

/// 忽略计划（「忽略」按钮）：ignored 置 1，提示条不再出现。
pub fn ignore_plan(conn: &rusqlite::Connection, plan_id: &str) -> StoreResult<()> {
    let affected = conn
        .execute(
            "UPDATE plans SET ignored = 1, updated_at = ?2 WHERE id = ?1",
            params![plan_id, now_millis()],
        )
        .map_err(other_err)?;
    if affected == 0 {
        return Err(crate::db::StoreError::Other("计划不存在".into()));
    }
    Ok(())
}

/// 批准计划（「批准并执行」按钮）：approved 置 1。
pub fn approve_plan(conn: &rusqlite::Connection, plan_id: &str) -> StoreResult<()> {
    let affected = conn
        .execute(
            "UPDATE plans SET approved = 1, updated_at = ?2 WHERE id = ?1",
            params![plan_id, now_millis()],
        )
        .map_err(other_err)?;
    if affected == 0 {
        return Err(crate::db::StoreError::Other("计划不存在".into()));
    }
    Ok(())
}

/// 计划落盘目录：数据目录下的 plans/（自动创建，权限 0700）。
pub fn plans_dir(db: &Db) -> StoreResult<PathBuf> {
    let dir = db.path().parent().unwrap_or_else(|| std::path::Path::new(".")).join("plans");
    std::fs::create_dir_all(&dir).map_err(|e| other_err(format!("创建计划目录失败：{e}")))?;
    Ok(dir)
}

/// 该 run 名下最后一段非空正文（计划 run 完成后取它作为计划文件内容）。
pub fn last_text_of_run(conn: &rusqlite::Connection, run_id: &str) -> StoreResult<Option<String>> {
    let mut stmt = conn
        .prepare(
            "SELECT payload FROM events WHERE run_id = ?1 AND kind = 'text' ORDER BY seq DESC",
        )
        .map_err(other_err)?;
    let rows: Vec<String> = stmt
        .query_map([run_id], |row| row.get::<_, String>(0))
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    for payload in rows {
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap_or(json!({}));
        let text = value.get("text").and_then(serde_json::Value::as_str).unwrap_or_default();
        if !text.trim().is_empty() {
            return Ok(Some(text.to_string()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn test_db(name: &str) -> (tempdir::TempDirLike, Db) {
        let dir = std::env::temp_dir().join(format!("craftwork-plans-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("craftwork.db")).unwrap();
        (tempdir::TempDirLike(dir), db)
    }

    /// 简易临时目录句柄：Drop 时清理（测试用，避免引入新依赖）。
    mod tempdir {
        pub struct TempDirLike(pub std::path::PathBuf);
        impl Drop for TempDirLike {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn plan_lifecycle_create_file_approve() {
        let (_dir, db) = test_db("lifecycle");
        // 建行：未批准、无文件（plans/events 无外键约束，无需先建会话行）
        let plan = db.with(|conn| create_plan(conn, "th_p", "run_p")).unwrap();
        assert_eq!(plan.file_path, "");
        assert!(!plan.approved);
        // 最新查询取到它
        let latest = db.with(|conn| latest_plan(conn, "th_p")).unwrap().unwrap();
        assert_eq!(latest.id, plan.id);

        // 落盘回填：文件路径生效
        let dir = plans_dir(&db).unwrap();
        let file = dir.join(format!("{}.md", plan.id));
        std::fs::write(&file, "# 计划").unwrap();
        db.with(|conn| set_plan_file(conn, &plan.id, file.to_str().unwrap())).unwrap();
        let latest = db.with(|conn| latest_plan(conn, "th_p")).unwrap().unwrap();
        assert_eq!(latest.file_path, file.to_str().unwrap());
        assert!(!latest.approved);

        // 批准
        db.with(|conn| approve_plan(conn, &plan.id)).unwrap();
        let latest = db.with(|conn| latest_plan(conn, "th_p")).unwrap().unwrap();
        assert!(latest.approved);

        // 不存在的计划批准报错
        assert!(db.with(|conn| approve_plan(conn, "plan_none")).is_err());
    }

    #[test]
    fn latest_plan_returns_newest_and_last_text_of_run() {
        let (_dir, db) = test_db("newest");
        db.with(|conn| {
            let p1 = create_plan(conn, "th_p", "run_1").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
            let p2 = create_plan(conn, "th_p", "run_2").unwrap();
            assert_ne!(p1.id, p2.id);
            Ok(())
        })
        .unwrap();
        let latest = db.with(|conn| latest_plan(conn, "th_p")).unwrap().unwrap();
        assert_eq!(latest.run_id.as_deref(), Some("run_2"));

        // run 名下最后一段非空 text（插入两条，取 seq 大的）
        db.with(|conn| {
            conn.execute(
                "INSERT INTO events (id, thread_id, message_id, run_id, seq, kind, payload, created_at) \
                 VALUES ('e1', 'th_p', 'm', 'run_2', 1, 'text', '{\"text\":\"\"}', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events (id, thread_id, message_id, run_id, seq, kind, payload, created_at) \
                 VALUES ('e2', 'th_p', 'm', 'run_2', 2, 'text', '{\"text\":\"最终计划\"}', 2)",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        let text = db.with(|conn| last_text_of_run(conn, "run_2")).unwrap().unwrap();
        assert_eq!(text, "最终计划");
    }
}
