//! 视图投影：ThreadRow → ThreadView（含最近一次用量推导的上下文占用）。
//! modelNameFor 的目录解析（normalizeModelRef）经 craft-model 完成。

use rusqlite::Connection;

use crate::db::other_err;
use crate::threads::ThreadRow;
use crate::usage_stats;
use craft_core::contract::ThreadView;

/// 会话最近一次用量行的投影（上下文占用与速度推导的原始数据）。
#[derive(Debug, Clone)]
pub struct LatestUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub duration_ms: i64,
    /// 完整提示词（含缓存读+写，归一化）；0 = 旧行（V13 之前），占用回退 input+output。
    pub prompt_tokens: i64,
    /// 首 token 延迟；0 = 旧行。
    pub ttft_ms: i64,
    /// 生成窗口；-1 = 无效（unrated），0 = 旧行（回退全程 duration）。
    pub gen_window_ms: i64,
}

/// 最近一次用量（上下文占用按会话取最新一条 usage 行）。
pub fn latest_usage_by_thread(conn: &Connection, thread_id: &str) -> StoreResult<LatestUsage> {
    conn.query_row(
        "SELECT input_tokens, output_tokens, cache_tokens, duration_ms, prompt_tokens, ttft_ms, gen_window_ms FROM usage WHERE thread_id = ?1 ORDER BY ts DESC LIMIT 1",
        [thread_id],
        |row| {
            Ok(LatestUsage {
                input_tokens: row.get(0)?,
                output_tokens: row.get(1)?,
                cache_tokens: row.get(2)?,
                duration_ms: row.get(3)?,
                prompt_tokens: row.get(4)?,
                ttft_ms: row.get(5)?,
                gen_window_ms: row.get(6)?,
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
/// 输出速度 tokens/s，口径统一走 usage_stats；unrated 记 0，前端按「—」显示；
/// 用量行缺失时上下文字段归零，不阻断视图）。
pub fn to_thread_view(conn: &Connection, row: &ThreadRow) -> StoreResult<ThreadView> {
    // ContextTokens 是「覆盖口径」：最近一次用量推导的当前上下文，与累计 Tokens（计费口径）不同
    let usage = latest_usage_by_thread(conn, &row.id).ok();
    // 速率分母回放：新行直接用落库的生成窗口；旧行（gen_window_ms=0）回退全程 duration（旧口径）；
    // unrated（-1 / 窗口越界）记 0
    let speed_window = match &usage {
        Some(u) if u.gen_window_ms > 0 => Some(u.gen_window_ms),
        Some(u) if u.gen_window_ms == 0 && u.duration_ms > 0 => Some(u.duration_ms),
        _ => None,
    };
    let speed = usage
        .as_ref()
        .zip(speed_window)
        .and_then(|(u, window)| usage_stats::tokens_per_second(u.output_tokens, window))
        .unwrap_or(0.0);
    let context_tokens = usage
        .as_ref()
        .map(|u| usage_stats::context_tokens(u.prompt_tokens, u.input_tokens, u.output_tokens))
        .unwrap_or(0);
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
        context_ttft_ms: usage.as_ref().map(|u| u.ttft_ms).unwrap_or(0),
        created_at: row.created_at,
        updated_at: row.updated_at,
        parent_thread_id: row.parent_thread_id.clone(),
        agent_name: row.agent_name.clone(),
        agent_type: row.agent_type.clone(),
        // 未读只对侧栏里的主会话成立：子会话不进侧栏，即便时间戳满足也不该点亮任何标记
        has_unread: row.parent_thread_id.is_empty() && row.last_done_at > row.last_read_at,
    })
}

use crate::db::StoreResult;
