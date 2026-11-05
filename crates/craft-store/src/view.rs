//! 视图投影：ThreadRow → ThreadView（含最近一次用量推导的上下文占用）。
//! modelNameFor 的目录解析（normalizeModelRef）经 craft-model 完成。

use rusqlite::Connection;

use crate::db::other_err;
use crate::threads::ThreadRow;
use craft_core::contract::ThreadView;

/// 会话最近一次用量行的投影（上下文占用与速度推导的原始数据）。
#[derive(Debug, Clone)]
pub struct LatestUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub duration_ms: i64,
}

/// 最近一次用量（上下文占用按会话取最新一条 usage 行）。
pub fn latest_usage_by_thread(conn: &Connection, thread_id: &str) -> StoreResult<LatestUsage> {
    conn.query_row(
        "SELECT input_tokens, output_tokens, cache_tokens, duration_ms FROM usage WHERE thread_id = ?1 ORDER BY ts DESC LIMIT 1",
        [thread_id],
        |row| {
            Ok(LatestUsage {
                input_tokens: row.get(0)?,
                output_tokens: row.get(1)?,
                cache_tokens: row.get(2)?,
                duration_ms: row.get(3)?,
            })
        },
    )
    .map_err(other_err)
}

type ModelNameResolver = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

static MODEL_NAME_RESOLVER: std::sync::OnceLock<ModelNameResolver> = std::sync::OnceLock::new();

/// 注入模型名解析器（壳层组装时接 Catalog.normalize；store 不反向依赖 model 包）。
pub fn set_model_name_resolver(resolver: impl Fn(&str) -> Option<String> + Send + Sync + 'static) {
    let _ = MODEL_NAME_RESOLVER.set(Box::new(resolver));
}

/// 模型显示名：目录解析器注入后返回解析名，否则非空 ID 一律「模型不可用」。
pub fn model_name_for(_conn: &Connection, model_id: &str) -> String {
    if model_id.is_empty() {
        return "未选择模型".into();
    }
    if let Some(name) = MODEL_NAME_RESOLVER.get().and_then(|resolver| resolver(model_id)) {
        return name;
    }
    "模型不可用".into()
}

/// 视图投影入口：ThreadRow + 最近一次用量 → ThreadView（context_speed 为最近一次
/// 输出速度 tokens/s；用量行缺失时上下文字段归零，不阻断视图）。
pub fn to_thread_view(conn: &Connection, row: &ThreadRow) -> StoreResult<ThreadView> {
    // ContextTokens 是「覆盖口径」：最近一次用量的 input+output，与累计 Tokens 不同
    let usage = latest_usage_by_thread(conn, &row.id).ok();
    let context_tokens = usage.as_ref().map(|u| u.input_tokens + u.output_tokens).unwrap_or(0);
    let speed = match &usage {
    Some(u) if u.duration_ms > 0 => u.output_tokens as f64 / (u.duration_ms as f64 / 1000.0),
    _ => 0.0,
    };
    Ok(ThreadView {
        id: row.id.clone(),
        mode: row.mode.clone(),
        title: row.title.clone(),
        model_id: row.model_id.clone(),
        model_name: model_name_for(conn, &row.model_id),
        reasoning_level: row.reasoning_level.clone(),
        workspace_id: row.workspace_id.clone(),
        worktree_path: row.worktree_path.clone(),
        worktree_branch: row.worktree_branch.clone(),
        status: row.status.clone(),
        archived: row.archived,
        tokens: row.tokens,
        context_tokens,
        context_cache_tokens: usage.as_ref().map(|u| u.cache_tokens).unwrap_or(0),
        context_speed: speed,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

use crate::db::StoreResult;
