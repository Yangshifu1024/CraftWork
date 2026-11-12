//! 提供商套餐额度解析（纯域逻辑，无网络、无时钟依赖）。
//!
//! 数据源是各家 coding plan / token plan 的**未官方文档化**用量端点，协议口径
//! 由 zcode-token-usage-statusbar（quota-client.cjs）与 CodeWave
//!（`src-tauri/src/core/quota/providers/`）两个参考实现交叉印证：
//!
//! - **OpenCode Go**：`GET https://opencode.ai/zen/go/v1/usage`，
//!   `Authorization: Bearer <key>`（与推理侧只认 x-api-key 相反）。响应
//!   `usage.{rolling|weekly|monthly} = {status, percent, resetsAt}`，`percent`
//!   是**已用**百分比（0-100），`resetsAt` 是 ISO 时间或毫秒时间戳。
//!   `status` 不止 `ok`：窗口用满时上游给 `rate-limited`，所以它只作诊断信息，
//!   **绝不作为窗口过滤条件**——按 `status == "ok"` 过滤会把「已用 100%」这档
//!   最该看到的窗口静默吞掉（CodeWave 实测教训）。
//! - **智谱 GLM Coding Plan**：`GET https://bigmodel.cn/api/monitor/usage/quota/limit`
//!   （国内站）/ `GET https://api.z.ai/api/monitor/usage/quota/limit`（国际站）。
//!   两站鉴权都是 `Authorization: <key>` **裸 key，不加 Bearer**（与上游一致，
//!   Bearer 前缀会判 401）。响应信封 `{success, data: {limits: [...]}}`，每项
//!   `type`（`TOKENS_LIMIT` 模型额度 / `CREDIT_LIMIT` 积分额度 / `TIME_LIMIT`
//!   MCP 工具时长，后者不阻断模型调用、不展示）、`unit`（3 = 5 小时窗口，
//!   6 = 周窗口；当前无月度编码，未知编码跳过）、`percentage`（已用百分比）、
//!   `currentValue`/`usage`（已用量/总量，可自算更精确的百分比）、
//!   `nextResetTime`（毫秒时间戳）。
//!
//! **识别与密钥流向**：`detect_flavor` 按 provider endpoint 的主机白名单匹配
//! （子域命中，CodeWave 同款；OpenCode 另要求路径含 `/go` 段——`/zen/v1` 按量版
//! 无用量 API，`/zen/gopher` 类前缀不命中）。白名单外（第三方中转同名提供商）
//! 一律不识别——额度请求发往固定官方 URL，key 只发给 key 所属服务。
//!
//! **防御式解析**：坏窗口（percent 缺失/非有限数/越界）跳过该窗不整体失败，
//! 全部窗口不可用才判失败；两家端点形态变化时表现为「本窗口缺失」，上层据此
//! 隐藏 chip 而非报错崩溃。

use serde_json::Value;

/// 单个额度窗口：key 标识窗口类型，used_percent 为已用百分比（0-100，一位小数）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    /// five_hour | weekly | monthly
    pub key: String,
    pub used_percent: f64,
    /// 重置时间（ISO 8601 字符串）；拿不到时为 None
    pub resets_at: Option<String>,
}

/// 提供商族：决定端点 URL 与鉴权头风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// 智谱国内站（bigmodel.cn），裸 key 鉴权
    Zhipu,
    /// 智谱国际站（z.ai），裸 key 鉴权
    Zai,
    /// OpenCode Go（opencode.ai），Bearer 鉴权
    OpenCodeGo,
}

impl Flavor {
    /// 用量端点（固定官方 URL，不随 provider endpoint 拼接——见模块头密钥流向）。
    pub fn usage_url(self) -> &'static str {
        match self {
            Flavor::Zhipu => "https://bigmodel.cn/api/monitor/usage/quota/limit",
            Flavor::Zai => "https://api.z.ai/api/monitor/usage/quota/limit",
            Flavor::OpenCodeGo => "https://opencode.ai/zen/go/v1/usage",
        }
    }

    /// Authorization 头风格：智谱两家都是裸 key，OpenCode Go 走 Bearer。
    pub fn auth_header(self, key: &str) -> String {
        match self {
            Flavor::Zhipu | Flavor::Zai => key.to_string(),
            Flavor::OpenCodeGo => format!("Bearer {key}"),
        }
    }
}

/// 从 endpoint 取小写主机名（去协议、去端口、去路径、去 userinfo）。
fn host_of(endpoint: &str) -> Option<&str> {
    let rest = endpoint.split("://").nth(1).unwrap_or(endpoint);
    let host_part = rest.split(['/', '?', '#']).next()?;
    let host = host_part.rsplit('@').next()?;
    let host = host.split(':').next()?;
    let host = host.trim();
    (!host.is_empty()).then_some(host)
}

/// 主机命中：相等或以其为后缀的子域（`api.bigmodel.cn` 命中 `bigmodel.cn`）。
fn host_matches(host: &str, candidate: &str) -> bool {
    host == candidate || host.ends_with(&format!(".{candidate}"))
}

/// 从 endpoint 取（小写主机, 小写路径）：路径以首个 `/`、`?` 或 `#` 起。
fn endpoint_host_and_path(endpoint: &str) -> Option<(String, String)> {
    let rest = endpoint.split("://").nth(1).unwrap_or(endpoint);
    let (base, path) = match rest.find(['/', '?', '#']) {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    let host = host_of(base)?;
    Some((host.to_ascii_lowercase(), path.to_ascii_lowercase()))
}

/// 按 provider endpoint 主机白名单识别提供商族。
///
/// 返回 None = 白名单外（第三方中转）：不查询额度。OpenCode 额外要求路径含
/// **`/go` 段**（按 `/.../` 切分后恰有段等于 `go`，子串匹配会误放行 `/zen/gopher`
/// 类前缀）：仅订阅版 Go 有用量 API；`/zen/v1` 按量版不命中，避免按量用户查到 404。
pub fn detect_flavor(endpoint: &str) -> Option<Flavor> {
    let (host, path) = endpoint_host_and_path(endpoint)?;
    if host_matches(&host, "bigmodel.cn") {
        return Some(Flavor::Zhipu);
    }
    if host_matches(&host, "z.ai") {
        return Some(Flavor::Zai);
    }
    if host_matches(&host, "opencode.ai") && path.split(['/', '?', '#']).any(|seg| seg == "go") {
        return Some(Flavor::OpenCodeGo);
    }
    None
}

/// 一位小数取整并钳到 0-100（展示与测试断言都以此为准）。
fn clamp_percent(v: f64) -> f64 {
    ((v * 10.0).round() / 10.0).clamp(0.0, 100.0)
}

/// 数值取值：数字直接用，数字字符串也接受（上游改过一次形态的防御）。
fn number_at(v: &Value, key: &str) -> Option<f64> {
    let raw = v.get(key)?;
    raw.as_f64()
        .or_else(|| raw.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
        .filter(|n| n.is_finite())
}

/// 正毫秒时间戳 → RFC3339；非法值丢弃。
fn ms_to_rfc3339(ms: f64) -> Option<String> {
    if !ms.is_finite() || ms <= 0.0 {
        return None;
    }
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// 重置时间归一为 ISO 字符串：ISO 原样收（须可解析，垃圾串丢弃），正毫秒时间戳转
/// RFC3339，其余丢弃。
fn normalize_reset(v: Option<&Value>) -> Option<String> {
    let v = v?;
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        // ISO 之外的字符串（上游形态变化的垃圾值）不透传；数字字符串按毫秒转
        return chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|_| s.to_string())
            .or_else(|| s.parse::<f64>().ok().and_then(ms_to_rfc3339));
    }
    v.as_f64().and_then(ms_to_rfc3339)
}

/// 智谱错误信封判定：`success=false` 或 code≥400 时的文案与错误码归为两类——
/// msg 指向「无 coding plan 套餐」→ `Ok(Some(()))`（无套餐语义，外层转 None）；
/// 其他业务错误 → `Err(可读中文)`；非错误信封 → `Ok(None)`。
fn zhipu_envelope_error(body: &Value) -> Result<Option<()>, String> {
    let success_failed = body.get("success").and_then(Value::as_bool) == Some(false);
    let code = body.get("code").and_then(Value::as_i64);
    let code_failed = code.is_some_and(|c| c >= 400);
    if !(success_failed || code_failed) {
        return Ok(None);
    }
    let msg = body.get("msg").and_then(Value::as_str).unwrap_or_default();
    let compact = msg.to_lowercase().replace([' ', '-', '_'], "");
    let no_plan = compact.contains("codingplan")
        && ["不存在", "没有", "未开通", "not exist", "not found", "not subscribed"]
            .iter()
            .any(|kw| compact.contains(&kw.to_lowercase()));
    if no_plan {
        Ok(Some(()))
    } else if code_failed {
        Err(format!("智谱额度接口错误 {}", code.unwrap_or_default()))
    } else {
        Err(format!("智谱额度接口错误：{}", truncate_msg(msg)))
    }
}

fn truncate_msg(msg: &str) -> String {
    let mut s: String = msg.chars().take(80).collect();
    if msg.chars().count() > 80 {
        s.push('…');
    }
    s
}

/// 解析智谱 limits 响应（智谱两家共用一个形状）。
///
/// 返回 `Ok(None)` = **无套餐**（信封 `success=false` 且 msg 指向「CodingPlan 不存在」类
/// 文案，上层映射 null、不视为错误）；`Ok(Some(windows))` = 有额度；
/// `Err` = 其他业务错误 / 信封缺 limits（可读中文）。
///
/// `TOKENS_LIMIT`（模型额度）/`CREDIT_LIMIT`（积分额度）按 unit 3=5 小时、6=周分类；
/// `TIME_LIMIT`（MCP 工具时长）不阻断模型调用，不展示；未知 unit 跳过不报错
/// （上游加新窗口编码时表现为「少一档」而非报错）。percent 优先 `currentValue/usage`
/// 自算（整数百分比更粗），缺省回落 `percentage` 字段。
pub fn parse_zhipu_limits(body: &Value) -> Result<Option<Vec<QuotaWindow>>, String> {
    let no_plan = zhipu_envelope_error(body)?;
    if no_plan.is_some() {
        return Ok(None);
    }
    let limits = body
        .pointer("/data/limits")
        .or_else(|| body.get("limits"))
        .and_then(Value::as_array)
        .ok_or_else(|| "智谱额度响应缺少 data.limits".to_string())?;
    let mut windows = Vec::new();
    for limit in limits {
        if !limit.is_object() {
            continue;
        }
        let kind = limit.get("type").and_then(Value::as_str).unwrap_or_default();
        let is_quota = kind == "TOKENS_LIMIT" || kind == "CREDIT_LIMIT";
        if !is_quota {
            continue;
        }
        let unit = limit.get("unit").and_then(Value::as_i64);
        let key = match unit {
            Some(3) => "five_hour",
            Some(6) => "weekly",
            // 未知编码：跳过（可能是上游新增窗口，不误报）
            _ => continue,
        };
        let Some(percent) = zhipu_percent(limit) else {
            continue;
        };
        windows.push(QuotaWindow {
            key: key.to_string(),
            used_percent: percent,
            resets_at: normalize_reset(limit.get("nextResetTime")),
        });
    }
    if windows.is_empty() {
        return Err("智谱额度响应中未找到可展示的窗口".to_string());
    }
    Ok(Some(windows))
}

/// 智谱单窗百分比：优先 currentValue/usage 自算，回落 percentage；越界值钳 0-100。
fn zhipu_percent(limit: &Value) -> Option<f64> {
    let declared = number_at(limit, "percentage");
    match (number_at(limit, "currentValue"), number_at(limit, "usage")) {
        (Some(cur), Some(total)) if total > 0.0 && cur >= 0.0 => Some(clamp_percent(cur / total * 100.0)),
        _ => declared.map(|p| {
            // percentage 为「已用」语义（智谱用量页同口径）；负数视作上游异常走 0
            clamp_percent(if p < 0.0 { 0.0 } else { p })
        }),
    }
}

/// 解析 OpenCode Go usage 响应：rolling / weekly / monthly 三窗，
/// percent 为已用百分比；percent 缺失或非有限数的窗口跳过，全部缺失判失败。
/// `status`（`ok` / `rate-limited`）不参与过滤——它是诊断信息不是可用性判据。
pub fn parse_opencode_go(body: &Value) -> Result<Vec<QuotaWindow>, String> {
    let usage = body
        .get("usage")
        .and_then(Value::as_object)
        .ok_or_else(|| "OpenCode Go 额度响应缺少 usage 字段".to_string())?;
    let mut windows = Vec::new();
    for (raw_key, key) in [("rolling", "five_hour"), ("weekly", "weekly"), ("monthly", "monthly")] {
        let Some(w) = usage.get(raw_key) else { continue };
        let Some(percent) = number_at(w, "percent") else { continue };
        windows.push(QuotaWindow {
            key: key.to_string(),
            used_percent: clamp_percent(if percent < 0.0 { 0.0 } else { percent }),
            resets_at: normalize_reset(w.get("resetsAt")),
        });
    }
    if windows.is_empty() {
        return Err("OpenCode Go 额度响应中未找到可展示的窗口".to_string());
    }
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---------- detect_flavor：白名单矩阵 ----------

    #[test]
    fn detect_zhipu_domestic_host_and_subdomains() {
        assert_eq!(detect_flavor("https://open.bigmodel.cn/api/coding/paas/v4"), Some(Flavor::Zhipu));
        assert_eq!(detect_flavor("https://bigmodel.cn/api/coding/paas/v4"), Some(Flavor::Zhipu));
    }

    #[test]
    fn detect_zai_international_and_subdomains() {
        assert_eq!(detect_flavor("https://api.z.ai/api/coding/paas/v4"), Some(Flavor::Zai));
        assert_eq!(detect_flavor("https://z.ai/api"), Some(Flavor::Zai));
    }

    #[test]
    fn detect_opencode_go_requires_go_path_segment() {
        assert_eq!(detect_flavor("https://opencode.ai/inference/go/openai/v1"), Some(Flavor::OpenCodeGo));
        assert_eq!(detect_flavor("https://opencode.ai/zen/go"), Some(Flavor::OpenCodeGo));
        // 按量版 /zen/v1 与 gopher 前缀不命中
        assert_eq!(detect_flavor("https://opencode.ai/zen/v1"), None);
        assert_eq!(detect_flavor("https://opencode.ai/zen/gopher/openai/v1"), None);
        assert_eq!(detect_flavor("https://opencode.ai/inference/openai/v1"), None);
    }

    #[test]
    fn detect_rejects_lookalike_domains_and_relays() {
        // 伪装域（后缀拼接）不命中
        assert_eq!(detect_flavor("https://evilbigmodel.cn/api"), None);
        assert_eq!(detect_flavor("https://notz.ai/api"), None);
        assert_eq!(detect_flavor("https://opencode.ai.evil.io/go"), None);
        // 第三方中转同名提供商不命中
        assert_eq!(detect_flavor("https://my-relay.example.com/v1"), None);
        assert_eq!(detect_flavor("http://localhost:8787/v1"), None);
        assert_eq!(detect_flavor(""), None);
    }

    #[test]
    fn auth_style_bearer_only_for_opencode() {
        assert_eq!(Flavor::Zhipu.auth_header("k1"), "k1");
        assert_eq!(Flavor::Zai.auth_header("k2"), "k2");
        assert_eq!(Flavor::OpenCodeGo.auth_header("k3"), "Bearer k3");
        assert_eq!(Flavor::OpenCodeGo.usage_url(), "https://opencode.ai/zen/go/v1/usage");
        assert_eq!(Flavor::Zhipu.usage_url(), "https://bigmodel.cn/api/monitor/usage/quota/limit");
        assert_eq!(Flavor::Zai.usage_url(), "https://api.z.ai/api/monitor/usage/quota/limit");
    }

    // ---------- 智谱 limits 解析 ----------

    #[test]
    fn zhipu_parses_five_hour_and_weekly_windows() {
        let body = json!({
            "success": true, "code": 0,
            "data": {"limits": [
                {"type": "TOKENS_LIMIT", "unit": 3, "percentage": 12.5,
                 "currentValue": 250.0, "usage": 2000.0, "nextResetTime": 1731500000000i64},
                {"type": "TOKENS_LIMIT", "unit": 6, "percentage": 40,
                 "nextResetTime": 1732100000000i64},
                {"type": "TIME_LIMIT", "unit": 0, "percentage": 10,
                 "nextResetTime": 1732100000000i64}
            ]}
        });
        let windows = parse_zhipu_limits(&body).unwrap().unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].key, "five_hour");
        assert_eq!(windows[0].used_percent, 12.5); // 250/2000 自算优先于声明的 12.5
        assert_eq!(windows[1].key, "weekly");
        assert_eq!(windows[1].used_percent, 40.0);
        assert!(windows[1].resets_at.is_some());
    }

    #[test]
    fn zhipu_credit_limit_counts_and_unknown_unit_skipped() {
        let body = json!({
            "success": true,
            "data": {"limits": [
                {"type": "CREDIT_LIMIT", "unit": 3, "percentage": 5},
                {"type": "TOKENS_LIMIT", "unit": 7, "percentage": 99}, // 未知编码跳过
                {"type": "OTHER", "unit": 3, "percentage": 1}
            ]}
        });
        let windows = parse_zhipu_limits(&body).unwrap().unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].key, "five_hour");
        assert_eq!(windows[0].used_percent, 5.0);
    }

    #[test]
    fn zhipu_bad_entries_skipped_without_failing() {
        let body = json!({
            "success": true,
            "data": {"limits": [
                {"type": "TOKENS_LIMIT", "unit": 3}, // 无 percent
                {"type": "TOKENS_LIMIT", "unit": 6, "percentage": "not-a-number"},
                {"type": "TOKENS_LIMIT", "unit": 6, "percentage": 66.6}
            ]}
        });
        let windows = parse_zhipu_limits(&body).unwrap().unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used_percent, 66.6);
    }

    #[test]
    fn zhipu_no_plan_envelope_maps_to_none_and_errors_stay_errors() {
        // 无套餐信封 → Ok(None)（上层映射 null、隐藏 chip），不误报为错误
        let no_plan = json!({"success": false, "code": 404,
            "msg": "CodingPlan 不存在"});
        assert_eq!(parse_zhipu_limits(&no_plan).unwrap(), None);
        // 其他业务错误 → Err（可读文案）
        let business = json!({"success": false, "code": 500,
            "msg": "系统繁忙，请稍后重试"});
        assert!(parse_zhipu_limits(&business).is_err());
        // success=true 但缺 limits → Err（形态异常）
        let missing = json!({"success": true, "data": {}});
        assert!(parse_zhipu_limits(&missing).is_err());
        let empty = json!({"success": true, "data": {"limits": []}});
        assert!(parse_zhipu_limits(&empty).is_err());
    }

    #[test]
    fn zhipu_percent_clamped_to_unit_range() {
        let body = json!({"success": true, "data": {"limits": [
            {"type": "TOKENS_LIMIT", "unit": 3, "percentage": 150.0},
            {"type": "TOKENS_LIMIT", "unit": 6, "currentValue": -4.0, "usage": 10.0}
        ]}});
        let windows = parse_zhipu_limits(&body).unwrap().unwrap();
        assert_eq!(windows[0].used_percent, 100.0);
        // currentValue 为负时回落声明的 percentage（缺省时该窗跳过）
        assert_eq!(windows.len(), 1);
    }

    // ---------- OpenCode Go 解析 ----------

    #[test]
    fn opencode_parses_three_windows_and_maps_keys() {
        let body = json!({
            "usage": {
                "rolling": {"status": "ok", "percent": 12.5, "resetsAt": "2026-11-12T18:00:00Z"},
                "weekly": {"status": "ok", "percent": 30, "resetsAt": "2026-11-16T00:00:00Z"},
                "monthly": {"status": "ok", "percent": 8.25, "resetsAt": "2026-12-01T00:00:00Z"}
            }
        });
        let windows = parse_opencode_go(&body).unwrap();
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].key, "five_hour"); // rolling → five_hour
        assert_eq!(windows[0].used_percent, 12.5);
        assert_eq!(windows[1].key, "weekly");
        assert_eq!(windows[2].key, "monthly");
    }

    #[test]
    fn opencode_rate_limited_window_is_kept_not_filtered() {
        // CodeWave 实测：用满时 status 是 rate-limited 而非 ok，必须保留该窗
        let body = json!({"usage": {
            "weekly": {"status": "rate-limited", "percent": 100, "resetsAt": "2026-11-16T00:00:00Z"}
        }});
        let windows = parse_opencode_go(&body).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].key, "weekly");
        assert_eq!(windows[0].used_percent, 100.0);
    }

    #[test]
    fn opencode_bad_windows_skipped_all_bad_fails() {
        let body = json!({"usage": {
            "rolling": {"status": "ok"}, // 无 percent
            "weekly": {"percent": "abc"},
            "monthly": {"percent": -5}
        }});
        let windows = parse_opencode_go(&body).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].key, "monthly");
        assert_eq!(windows[0].used_percent, 0.0); // 负数钳 0

        assert!(parse_opencode_go(&json!({})).is_err());
        assert!(parse_opencode_go(&json!({"usage": {}})).is_err());
        assert!(parse_opencode_go(&json!({"usage": {"rolling": {"percent": "x"}}})).is_err());
    }

    #[test]
    fn opencode_numeric_string_percent_and_ms_resets_accepted() {
        let body = json!({"usage": {
            "rolling": {"percent": "42.5", "resetsAt": 1731500000000i64},
            "weekly": {"percent": 0, "resetsAt": "garbage"}
        }});
        let windows = parse_opencode_go(&body).unwrap();
        assert_eq!(windows[0].used_percent, 42.5);
        assert!(windows[0].resets_at.as_deref().is_some_and(|s| s.contains('T'))); // ms → RFC3339
        assert_eq!(windows[1].used_percent, 0.0);
        assert_eq!(windows[1].resets_at, None); // 非时间字符串丢弃
    }
}
