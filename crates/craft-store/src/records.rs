//! 审计 / 用量 / 设置 / 备份 表读写。

use rusqlite::Connection;

use crate::db::{now_millis, other_err, StoreError, StoreResult};
use craft_core::contract::{AuditEntry, Backup, TokenTotals, UsageByDay, UsageByModel, UsageSummary};
use craft_core::ids::{id_prefix, new_id};

// ---------- 审计 ----------

/// 待写入的一条审计记录；ts 为 None 时取当前时间，空串可选项统一存 NULL。
pub struct AuditRecord<'a> {
    pub ts: Option<i64>,
    pub thread_id: Option<&'a str>,
    pub run_id: Option<&'a str>,
    pub tool: &'a str,
    pub risk: &'a str,
    pub decision: &'a str,
    pub scope: Option<&'a str>,
    pub detail: Option<&'a str>,
}

/// 写入审计（审批决定与工具执行都必须落审计，见 AGENTS.md 审批约束）。
pub fn add_audit(conn: &Connection, rec: AuditRecord<'_>) -> StoreResult<()> {
    conn.execute(
        "INSERT INTO audit_log (ts, thread_id, run_id, tool, risk, decision, scope, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            rec.ts.unwrap_or_else(now_millis),
            rec.thread_id.filter(|s| !s.is_empty()),
            rec.run_id.filter(|s| !s.is_empty()),
            rec.tool,
            rec.risk,
            rec.decision,
            rec.scope.filter(|s| !s.is_empty()),
            rec.detail.filter(|s| !s.is_empty()),
        ],
    )
    .map_err(other_err)?;
    Ok(())
}

/// 最近审计列表（ts DESC）；limit 非法（<=0）或超上限（>500）时回落默认 200。
pub fn list_audit(conn: &Connection, limit: i64) -> StoreResult<Vec<AuditEntry>> {
    let n = if limit <= 0 || limit > 500 { 200 } else { limit };
    let mut stmt = conn
        .prepare("SELECT * FROM audit_log ORDER BY ts DESC, id DESC LIMIT ?1")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([n], |row| {
            Ok(AuditEntry {
                id: row.get("id")?,
                ts: row.get("ts")?,
                thread_id: row.get::<_, Option<String>>("thread_id")?.unwrap_or_default(),
                run_id: row.get::<_, Option<String>>("run_id")?.unwrap_or_default(),
                tool: row.get("tool")?,
                risk: row.get("risk")?,
                decision: row.get("decision")?,
                scope: row.get::<_, Option<String>>("scope")?.unwrap_or_default(),
                detail: row.get::<_, Option<String>>("detail")?.unwrap_or_default(),
            })
        })
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

// ---------- 用量 ----------

/// 待写入的一条用量行（ts 取当前时间；thread_id 空串存 NULL）。
///
/// prompt_tokens / ttft_ms / gen_window_ms 是速率与占用口径列（V13）：
/// - prompt_tokens：完整提示词（含缓存读+写，归一化）；0 = 未填充（旧行回退 input+output）；
/// - gen_window_ms：生成窗口；无效（取消/过短）写 `GEN_WINDOW_UNRATED`（-1），0 = 旧行。
pub struct UsageRecord<'a> {
    pub thread_id: Option<&'a str>,
    pub model_id: &'a str,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub reasoning_tokens: i64,
    pub cost_micros: i64,
    pub duration_ms: i64,
    pub prompt_tokens: i64,
    pub ttft_ms: i64,
    pub gen_window_ms: i64,
}

pub fn add_usage(conn: &Connection, rec: UsageRecord<'_>) -> StoreResult<()> {
    conn.execute(
        "INSERT INTO usage (ts, thread_id, model_id, input_tokens, output_tokens, cache_tokens, reasoning_tokens, cost_micros, duration_ms, prompt_tokens, ttft_ms, gen_window_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            now_millis(),
            rec.thread_id.filter(|s| !s.is_empty()),
            rec.model_id,
            rec.input_tokens,
            rec.output_tokens,
            rec.cache_tokens,
            rec.reasoning_tokens,
            rec.cost_micros,
            rec.duration_ms,
            rec.prompt_tokens,
            rec.ttft_ms,
            rec.gen_window_ms,
        ],
    )
    .map_err(other_err)?;
    Ok(())
}

fn sum_window(conn: &Connection, since_ts: i64) -> StoreResult<TokenTotals> {
    conn.query_row(
        "SELECT COALESCE(SUM(input_tokens),0) AS i, COALESCE(SUM(output_tokens),0) AS o, COALESCE(SUM(cost_micros),0) AS c FROM usage WHERE ts >= ?1",
        [since_ts],
        |row| {
            Ok(TokenTotals {
                input_tokens: row.get(0)?,
                output_tokens: row.get(1)?,
                cost_micros: row.get(2)?,
            })
        },
    )
    .map_err(other_err)
}

/// 用量汇总：今天/近7天/近30天/按模型近30天/按天近14天；窗口从本地时区今天零点回推。
pub fn usage_summary(conn: &Connection) -> StoreResult<UsageSummary> {
    let midnight = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(chrono::Local)
        .single()
        .map(|dt| dt.timestamp_millis())
        .unwrap_or_else(now_millis);
    let day: i64 = 24 * 60 * 60 * 1000;

    let today = sum_window(conn, midnight)?;
    let week = sum_window(conn, midnight - 6 * day)?;
    let month = sum_window(conn, midnight - 29 * day)?;

    let mut stmt = conn
        .prepare(
            "SELECT model_id, COALESCE(SUM(input_tokens),0) AS i, COALESCE(SUM(output_tokens),0) AS o, COALESCE(SUM(cost_micros),0) AS c FROM usage WHERE ts >= ?1 GROUP BY model_id ORDER BY i + o DESC LIMIT 20",
        )
        .map_err(other_err)?;
    let by_model = stmt
        .query_map([midnight - 29 * day], |row| {
            Ok(UsageByModel {
                model_id: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                cost_micros: row.get(3)?,
            })
        })
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;

    let mut stmt = conn
        .prepare(
            "SELECT date(ts/1000,'unixepoch','localtime') AS d, COALESCE(SUM(input_tokens),0) AS i, COALESCE(SUM(output_tokens),0) AS o, COALESCE(SUM(cost_micros),0) AS c FROM usage WHERE ts >= ?1 GROUP BY d ORDER BY d ASC",
        )
        .map_err(other_err)?;
    let by_day = stmt
        .query_map([midnight - 13 * day], |row| {
            Ok(UsageByDay {
                day: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                cost_micros: row.get(3)?,
            })
        })
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;

    Ok(UsageSummary {
        today,
        week,
        month,
        by_model,
        by_day,
    })
}

// ---------- 设置 ----------

/// 读取设置的 JSON 值；键不存在或值损坏（非法 JSON）一律返回 None，由调用方用默认值兜底。
pub fn get_setting(conn: &Connection, key: &str) -> StoreResult<Option<serde_json::Value>> {
    let row: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0))
        .ok();
    match row {
        None => Ok(None),
        Some(text) => match serde_json::from_str(&text) {
            Ok(value) => Ok(Some(value)),
            Err(_) => Ok(None),
        },
    }
}

pub fn set_setting(conn: &Connection, key: &str, value: &serde_json::Value) -> StoreResult<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, serde_json::to_string(value).unwrap_or_else(|_| "null".into())],
    )
    .map_err(other_err)?;
    Ok(())
}

pub fn delete_setting(conn: &Connection, key: &str) -> StoreResult<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", [key]).map_err(other_err)?;
    Ok(())
}

// ---------- 备份 ----------

/// 待登记的一次写前备份（hash 为原文件 SHA-256；备份文件本体由调用方落盘）。
pub struct BackupRecord<'a> {
    pub path: &'a str,
    pub backup_path: &'a str,
    pub hash: &'a str,
    pub size: i64,
}

pub fn add_backup(conn: &Connection, rec: BackupRecord<'_>) -> StoreResult<String> {
    let id = new_id(id_prefix::BACKUP);
    conn.execute(
        "INSERT INTO backups (id, path, backup_path, hash, size, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, rec.path, rec.backup_path, rec.hash, rec.size, now_millis()],
    )
    .map_err(other_err)?;
    Ok(id)
}

pub fn get_backup(conn: &Connection, id: &str) -> StoreResult<Backup> {
    conn.query_row("SELECT * FROM backups WHERE id = ?1", [id], to_backup)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound("备份不存在".into()),
            other => other_err(other),
        })
}

fn to_backup(row: &rusqlite::Row<'_>) -> rusqlite::Result<Backup> {
    Ok(Backup {
        id: row.get("id")?,
        path: row.get("path")?,
        backup_path: row.get("backup_path")?,
        hash: row.get("hash")?,
        size: row.get("size")?,
        created_at: row.get("created_at")?,
    })
}

/// 最近备份列表（created_at DESC）；limit 非法（<=0）或超上限（>200）时回落默认 50。
pub fn list_backups(conn: &Connection, limit: i64) -> StoreResult<Vec<Backup>> {
    let n = if limit <= 0 || limit > 200 { 50 } else { limit };
    let mut stmt = conn
        .prepare("SELECT * FROM backups ORDER BY created_at DESC LIMIT ?1")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([n], to_backup)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}
