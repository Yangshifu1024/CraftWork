//! MCP 服务器配置模型：形状、校验归一与旧数据升级。
//!
//! 配置持久化在 settings 表 `mcp.servers` 键（JSON 数组）。本模块只做纯数据，
//! 不触碰数据库与网络，便于独立测试。
//!
//! 刻意收窄的边界：不实现 OAuth、协议版本 pin、作用域（user/project）
//! 与项目级信任门——CraftWork 的 MCP 配置只有「用户级」一层，由用户在设置页显式添加，
//! 因此不存在「检出仓库即自动跑本地进程」的风险面（见 D16）。

use serde_json::{Value, json};

/// 传输类型。`Sse` 是旧式 HTTP 事件流；rmcp 的 streamable HTTP 客户端会自动
/// 兼容这种行为，因此两者在连接实现上共用一条路径，配置上仍分开以便用户表达意图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    Http,
    Sse,
}

impl Transport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Transport::Stdio => "stdio",
            Transport::Http => "http",
            Transport::Sse => "sse",
        }
    }

    /// 解析传输类型；未知值返回 None（由调用方决定报错还是推断）。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stdio" => Some(Transport::Stdio),
            "http" | "streamablehttp" | "streamable-http" => Some(Transport::Http),
            "sse" => Some(Transport::Sse),
            _ => None,
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Transport::Http | Transport::Sse)
    }
}

/// 默认超时：连接、工具列表与单次调用共用（毫秒）。MCP 服务器冷启动可能较慢，
/// 30 秒是 MCP 生态的通行取值，冷启动较慢也有余量。
pub const DEFAULT_TIMEOUT_MS: i64 = 30_000;
/// 超时下限（毫秒）：1 秒。比这更短的超时在 MCP 上没有实际意义（握手就要一个往返），
/// 只会把「配置填错」伪装成「服务器太慢」。
pub const MIN_TIMEOUT_MS: i64 = 1_000;
/// 超时上限（毫秒）：30 分钟。个别服务器（下载、渲染类）需要长超时，但不能无界。
pub const MAX_TIMEOUT_MS: i64 = 30 * 60 * 1000;

/// 一个 MCP 服务器的配置。
#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    /// ID 前缀 `mcp_`（craft_core::ids::id_prefix::MCP_SERVER）
    pub id: String,
    /// 用户可读名称，同时是模型侧工具名的第一段来源，必须非空且唯一
    pub name: String,
    pub transport: Transport,
    /// stdio：可执行文件
    pub command: String,
    pub args: Vec<String>,
    /// stdio：用户显式填写的环境变量。合并规则见 `stdio_env`。
    pub env: Vec<(String, String)>,
    /// stdio：工作目录（空串 = 继承进程当前目录）
    pub cwd: String,
    /// 远程：服务地址
    pub url: String,
    /// 远程：附加请求头
    pub headers: Vec<(String, String)>,
    pub timeout_ms: i64,
    pub enabled: bool,
}

impl ServerConfig {
    /// 默认配置（用于表单新建）。
    pub fn new(name: String, transport: Transport) -> Self {
        Self {
            id: String::new(),
            name,
            transport,
            command: String::new(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: String::new(),
            url: String::new(),
            headers: Vec::new(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            enabled: true,
        }
    }

    /// 校验归一：名称非空、传输类型与目标字段自洽、超时落在合法区间。
    ///
    /// 传输类型缺省时按目标字段推断（有 command 即 stdio，有 url 即 http）——
    /// 旧留桩数据与手工粘贴的配置都可能没有 type 字段。
    pub fn normalize(mut self) -> Result<Self, String> {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            return Err("服务器名称不能为空".into());
        }
        self.command = self.command.trim().to_string();
        self.url = self.url.trim().to_string();
        self.cwd = self.cwd.trim().to_string();
        self.args.retain(|a| !a.is_empty());
        self.env = normalize_pairs(self.env);
        self.headers = normalize_pairs(self.headers);

        if self.transport == Transport::Stdio && self.command.is_empty() {
            return Err("stdio 服务器必须填写启动命令".into());
        }
        if self.transport.is_remote() && self.url.is_empty() {
            return Err(format!("{} 服务器必须填写服务地址", self.transport.as_str()));
        }
        // 超时：夹在 [1 秒, 30 分钟]。下限不是 0——1 毫秒的超时必然瞬间失败，
        // 用户只会看到「连接超时（1 毫秒）」这种无从排查的报错；非正值回落默认值。
        if self.timeout_ms <= 0 {
            self.timeout_ms = DEFAULT_TIMEOUT_MS;
        }
        self.timeout_ms = self.timeout_ms.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
        Ok(self)
    }

    /// 推断传输类型（缺省字段时使用）。
    pub fn infer_transport(&self) -> Transport {
        if !self.command.is_empty() {
            Transport::Stdio
        } else if !self.url.is_empty() {
            Transport::Http
        } else {
            self.transport
        }
    }
}

/// 键值对清洗：丢弃空键，值原样保留（凭据可能含空格），键去重保首个。
fn normalize_pairs(pairs: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (key, value) in pairs {
        let key = key.trim().to_string();
        if key.is_empty() {
            continue;
        }
        if out.iter().any(|(k, _)| k == &key) {
            continue;
        }
        out.push((key, value));
    }
    out
}

/// 从持久化的 JSON 值解析服务器列表。
///
/// 容错原则：单条解析失败只丢该条，不影响其余（历史脏数据不该让整页打不开）。
/// 旧留桩数据只有 `{id,name,command,args,enabled}`——按「有 command 即 stdio」升级。
pub fn parse_servers(value: &Value) -> Vec<ServerConfig> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items.iter().filter_map(parse_server).collect()
}

fn parse_server(item: &Value) -> Option<ServerConfig> {
    let name = item.get("name")?.as_str()?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let transport = item
        .get("transport")
        .and_then(Value::as_str)
        .and_then(Transport::parse)
        .unwrap_or_else(|| {
            if item.get("command").and_then(Value::as_str).is_some_and(|c| !c.trim().is_empty()) {
                Transport::Stdio
            } else {
                Transport::Http
            }
        });
    let mut config = ServerConfig {
        id: item.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
        name,
        transport,
        command: item.get("command").and_then(Value::as_str).unwrap_or_default().to_string(),
        args: item
            .get("args")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default(),
        env: parse_pairs(item.get("env")),
        cwd: item.get("cwd").and_then(Value::as_str).unwrap_or_default().to_string(),
        url: item.get("url").and_then(Value::as_str).unwrap_or_default().to_string(),
        headers: parse_pairs(item.get("headers")),
        timeout_ms: item.get("timeoutMs").and_then(Value::as_i64).unwrap_or(DEFAULT_TIMEOUT_MS),
        enabled: item.get("enabled").and_then(Value::as_bool).unwrap_or(true),
    };
    // 旧数据可能既没有 transport 也没有 url/command 的可用值：推断后仍不合法就丢弃
    if config.transport.is_remote() && config.url.is_empty() {
        if !config.command.is_empty() {
            config.transport = Transport::Stdio;
        } else {
            return None;
        }
    }
    config.normalize().ok()
}

fn parse_pairs(value: Option<&Value>) -> Vec<(String, String)> {
    let Some(Value::Object(map)) = value else {
        return Vec::new();
    };
    map.iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
        .collect()
}

/// 序列化为持久化 JSON（键值与 shared/types.ts 的 MCPServerView 对齐）。
pub fn server_to_json(config: &ServerConfig) -> Value {
    json!({
        "id": config.id,
        "name": config.name,
        "transport": config.transport.as_str(),
        "command": config.command,
        "args": config.args,
        "env": pairs_to_object(&config.env),
        "cwd": config.cwd,
        "url": config.url,
        "headers": pairs_to_object(&config.headers),
        "timeoutMs": config.timeout_ms,
        "enabled": config.enabled,
    })
}

fn pairs_to_object(pairs: &[(String, String)]) -> Value {
    let mut map = serde_json::Map::new();
    for (k, v) in pairs {
        map.insert(k.clone(), Value::String(v.clone()));
    }
    Value::Object(map)
}

/// 服务器名归一为模型可见工具名的一段：非 `[A-Za-z0-9_-]` → `_`，连续下划线折叠。
///
/// 这是 MCP 生态通行的一段名归一规则，保证同一份 MCP 配置在不同客户端里
/// 产生同一个工具名（用户从别处抄配置过来时不必重新理解命名）。
pub fn sanitize_name_part(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_underscore = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
            last_underscore = false;
        } else if ch == '_' || !ch.is_ascii() {
            // 下划线与任意非 ASCII 字符都归一为单个下划线
            if !last_underscore {
                out.push('_');
                last_underscore = true;
            }
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() { "unknown".to_string() } else { trimmed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_config_requires_command() {
        let mut config = ServerConfig::new("demo".into(), Transport::Stdio);
        config.command = "  ".into();
        let err = config.normalize().unwrap_err();
        assert!(err.contains("启动命令"), "错误文案应指出缺命令：{err}");
    }

    #[test]
    fn remote_config_requires_url() {
        for transport in [Transport::Http, Transport::Sse] {
            let mut config = ServerConfig::new("demo".into(), transport);
            config.url = String::new();
            assert!(config.normalize().is_err(), "{} 缺 url 应被拒绝", transport.as_str());
        }
    }

    #[test]
    fn blank_name_rejected() {
        let mut config = ServerConfig::new("   ".into(), Transport::Stdio);
        config.command = "node".into();
        assert!(config.normalize().is_err());
    }

    #[test]
    fn timeout_falls_back_to_default_and_clamps_to_max() {
        let mut config = ServerConfig::new("demo".into(), Transport::Stdio);
        config.command = "node".into();
        config.timeout_ms = 0;
        assert_eq!(config.normalize().unwrap().timeout_ms, DEFAULT_TIMEOUT_MS);

        let mut config = ServerConfig::new("demo".into(), Transport::Stdio);
        config.command = "node".into();
        config.timeout_ms = MAX_TIMEOUT_MS * 2;
        assert_eq!(config.normalize().unwrap().timeout_ms, MAX_TIMEOUT_MS);
    }

    /// 规格第 4 节：超时一律夹在 [1 秒, 30 分钟]。下限曾被漏掉，
    /// `timeoutMs: 1` 会被原样保存，然后必然「连接超时（1 毫秒）」。
    #[test]
    fn timeout_is_raised_to_the_floor() {
        let mut config = ServerConfig::new("demo".into(), Transport::Stdio);
        config.command = "node".into();
        config.timeout_ms = 1;
        assert_eq!(config.normalize().unwrap().timeout_ms, MIN_TIMEOUT_MS);

        let mut config = ServerConfig::new("demo".into(), Transport::Stdio);
        config.command = "node".into();
        config.timeout_ms = -5;
        assert_eq!(config.normalize().unwrap().timeout_ms, DEFAULT_TIMEOUT_MS, "非正值回落默认值");
    }

    /// 旧留桩数据（决策 D5 时代只存 name/command/args/enabled）必须能读出来，
    /// 否则升级后用户的服务器配置会整片消失。
    #[test]
    fn legacy_stub_rows_upgrade_to_stdio() {
        let legacy = json!([{
            "id": "mcp_0123456789abcdef",
            "name": "filesystem",
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-filesystem"],
            "enabled": true
        }]);
        let servers = parse_servers(&legacy);
        assert_eq!(servers.len(), 1);
        let config = &servers[0];
        assert_eq!(config.transport, Transport::Stdio);
        assert_eq!(config.command, "npx");
        assert_eq!(config.args.len(), 2);
        assert_eq!(config.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert!(config.enabled);
    }

    #[test]
    fn broken_rows_are_dropped_without_affecting_others() {
        let value = json!([
            { "name": "" },
            { "id": "mcp_a", "name": "ok", "command": "node" },
            { "id": "mcp_b", "name": "no-target" }
        ]);
        let servers = parse_servers(&value);
        assert_eq!(servers.len(), 1, "只有配置完整的条目保留：{servers:?}");
        assert_eq!(servers[0].name, "ok");
    }

    #[test]
    fn round_trip_preserves_fields() {
        let mut config = ServerConfig::new("远程 服务".into(), Transport::Http);
        config.url = "https://example.com/mcp".into();
        config.headers = vec![("Authorization".into(), "Bearer abc".into())];
        config.timeout_ms = 12_345;
        let json = server_to_json(&config);
        let parsed = parse_servers(&json!([json]));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "远程 服务");
        assert_eq!(parsed[0].transport, Transport::Http);
        assert_eq!(parsed[0].headers, vec![("Authorization".to_string(), "Bearer abc".to_string())]);
        assert_eq!(parsed[0].timeout_ms, 12_345);
    }

    #[test]
    fn env_pairs_drop_blank_keys_and_keep_first_duplicate() {
        let mut config = ServerConfig::new("demo".into(), Transport::Stdio);
        config.command = "node".into();
        config.env = vec![
            ("  ".into(), "x".into()),
            ("A".into(), "1".into()),
            ("A".into(), "2".into()),
        ];
        let normalized = config.normalize().unwrap();
        assert_eq!(normalized.env, vec![("A".to_string(), "1".to_string())]);
    }

    #[test]
    fn transport_parse_accepts_common_spellings() {
        assert_eq!(Transport::parse("STDIO"), Some(Transport::Stdio));
        assert_eq!(Transport::parse("streamableHttp"), Some(Transport::Http));
        assert_eq!(Transport::parse("streamable-http"), Some(Transport::Http));
        assert_eq!(Transport::parse("sse"), Some(Transport::Sse));
        assert_eq!(Transport::parse("websocket"), None);
    }

    #[test]
    fn sanitize_name_part_matches_zcode_rules() {
        assert_eq!(sanitize_name_part("my-server"), "my-server");
        assert_eq!(sanitize_name_part("my server"), "my_server");
        assert_eq!(sanitize_name_part("a//b"), "a_b");
        assert_eq!(sanitize_name_part("中文名"), "unknown");
        assert_eq!(sanitize_name_part(""), "unknown");
        // 首尾下划线被裁掉，避免产生 `mcp___server` 这种三连下划线
        assert_eq!(sanitize_name_part("_x_"), "x");
    }
}
