//! HTTP 客户端构建辅助（D14）：协议适配器的 reqwest 客户端
//! 一律经本模块构建，代理配置由 craft-core::network 统一解析应用，不得绕过。

use craft_core::network::ProxyConfig;

/// 构建 reqwest 客户端：
/// - 传入代理配置：按其三模式语义应用（custom 经代理 + 内置本地排除 / none 显式直连 /
///   system 环境变量 > OS 系统代理 > 直连），由调用方从 settings 即时读出传入；
/// - 传 None：按 system 模式读真实环境解析（与改造前 `Client::new()` 的默认行为一致，
///   供测试桩等无设置上下文的调用方兜底）。
///
/// `User-Agent` 由 `apply_reqwest` 统一注入 `CraftWork/<版本>`（口径见
/// `craft_core::network::APP_USER_AGENT`），本处不另设。
///
/// 构建失败在此 panic：URL 已在 craft-core 归一校验且解析失败会兜底直连，失败只剩
/// TLS 后端初始化等进程级异常，与改造前 `Client::new()` 的 panic 语义相同。
pub fn build_http_client(proxy: Option<&ProxyConfig>) -> reqwest::Client {
    let config = proxy.map_or_else(default_proxy_config, |config| config.clone());
    config
        .apply_reqwest(reqwest::Client::builder())
        .build()
        .expect("HTTP 客户端构建失败")
}

/// 默认代理配置：system 模式读真实环境（该模式解析不校验 URL，不会失败）。
fn default_proxy_config() -> ProxyConfig {
    ProxyConfig::from_settings("system", "").expect("system 模式解析不会失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_config_builds_direct_client() {
        let config = ProxyConfig { mode: craft_core::network::ProxyMode::None, effective_url: None };
        let _ = build_http_client(Some(&config));
        // 无配置兜底 system 模式同样可构建（不依赖环境里是否真有代理）
        let _ = build_http_client(None);
    }

    #[test]
    fn socks_proxy_urls_require_socks_feature_and_construct() {
        // craft-model 的 reqwest 必须启用 socks feature：socks5/socks5h 代理地址可被
        // Proxy::all 解析（feature 缺失时该断言失败）；带 socks 配置的客户端可构建
        // （构建不发起连接，代理不可达只在真实请求时暴露，经 errors.rs 归一为可读错误）。
        assert!(reqwest::Proxy::all("socks5://127.0.0.1:1080").is_ok());
        assert!(reqwest::Proxy::all("socks5h://127.0.0.1:1080").is_ok());
        let config = ProxyConfig {
            mode: craft_core::network::ProxyMode::Custom,
            effective_url: Some("socks5://127.0.0.1:1080".into()),
        };
        let _ = build_http_client(Some(&config));
    }
}
