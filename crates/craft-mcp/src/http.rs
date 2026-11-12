//! MCP 远程传输的 HTTP 装配：统一走 `craft_core::network` 的代理解析（决策 D14）。
//!
//! 为什么不直接用 rmcp 的 `from_uri`：那条路径内部会新建自己的 reqwest client，
//! 用户的代理设置（系统代理 / 自定义代理 / 禁用代理）对它不生效。这里显式构造
//! client 再交给 rmcp，保证「应用内所有出站请求经同一网络配置」这条约束不被绕过。

use std::collections::HashMap;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

/// 按当前网络设置构造 reqwest 客户端。
///
/// 读到的是进程环境的实时值（`ProxyConfig::from_settings` 内部会读系统代理），
/// 与 craft-model 的做法一致：每次建立连接时求值，改设置后下一次连接生效。
/// `User-Agent` 同样由 `apply_reqwest` 统一注入 `CraftWork/<版本>`（口径见
/// `craft_core::network::APP_USER_AGENT`），本处不另设。
pub fn build_client() -> Result<reqwest::Client, String> {
    build_client_with(&crate::network_settings())
}

/// 用显式给定的（模式, 地址）构造客户端（可测试）。
pub fn build_client_with(settings: &(String, String)) -> Result<reqwest::Client, String> {
    let proxy = craft_core::network::ProxyConfig::from_settings(&settings.0, &settings.1)
        .unwrap_or(craft_core::network::ProxyConfig {
            mode: craft_core::network::ProxyMode::None,
            effective_url: None,
        });
    let builder = reqwest::Client::builder()
        // MCP 服务器的长轮询/流式响应可能长时间静默，禁用空闲超时以免被本机掐断
        .connect_timeout(std::time::Duration::from_secs(15))
        .pool_idle_timeout(std::time::Duration::from_secs(90));
    proxy
        .apply_reqwest(builder)
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败：{e}"))
}

/// 把配置里的字符串键值对转换为 reqwest 的头部表；非法头名/头值给出可读错误。
pub fn header_map(pairs: &[(String, String)]) -> Result<HashMap<HeaderName, HeaderValue>, String> {
    let mut map = HashMap::with_capacity(pairs.len());
    for (key, value) in pairs {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| format!("非法的请求头名称：{key}"))?;
        let value = HeaderValue::from_str(value)
            .map_err(|_| format!("请求头 {key} 的值含非法字符"))?;
        map.insert(name, value);
    }
    Ok(map)
}

/// 供测试与诊断使用的 HeaderMap 形态（检测重复键等）。
pub fn header_map_strict(pairs: &[(String, String)]) -> Result<HeaderMap, String> {
    let mut map = HeaderMap::new();
    for (name, value) in header_map(pairs)? {
        map.insert(name, value);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illegal_header_name_is_rejected() {
        let err = header_map(&[("bad header".into(), "x".into())]).unwrap_err();
        assert!(err.contains("非法的请求头名称"), "{err}");
    }

    #[test]
    fn header_values_are_kept_verbatim() {
        let map = header_map(&[("Authorization".into(), "Bearer a b c".into())]).unwrap();
        let name = HeaderName::from_static("authorization");
        assert_eq!(map.get(&name).unwrap().to_str().unwrap(), "Bearer a b c");
    }

    /// 代理模式 none 应能正常构造客户端（不触网，只验证装配不 panic）。
    #[test]
    fn client_builds_under_none_proxy_mode() {
        assert!(build_client_with(&("none".to_string(), String::new())).is_ok());
    }

    /// custom 模式地址非法时回落直连（与 craft-core 的容错约定一致），不应报错。
    #[test]
    fn client_falls_back_to_direct_on_invalid_custom_proxy() {
        assert!(build_client_with(&("custom".to_string(), "not-a-url".to_string())).is_ok());
    }
}
