//! 出站网络配置的唯一解析点（D14）。
//!
//! 职责：把设置里的代理三模式（system / none / custom）与 GitHub 加速前缀，解析成几份
//! 中性结果——reqwest 异步客户端出口（[`ProxyConfig::apply_reqwest`]）、reqwest blocking
//! 客户端出口（[`ProxyConfig::apply_reqwest_blocking`]，壳层探活 / 拉取模型用）与 gix
//! 传输选项出口（[`ProxyConfig::gix_proxy_options`]），保证各出口语义一致。应用内新增
//! 任何出站请求都必须经本模块，不得绕过。
//!
//! 可测试性：环境变量与系统代理的读取都通过 [`ProxyConfig::resolve`] 的注入参数传入
//! （纯函数），生产侧 [`ProxyConfig::from_settings`] 只是薄包装读真实环境——测试并行
//! 跑时不会互相污染进程环境变量。
//!
//! # 出站 UA（[`APP_USER_AGENT`]）
//!
//! 所有 reqwest 出口（[`ProxyConfig::apply_reqwest`] / [`ProxyConfig::apply_reqwest_blocking`]）
//! 在此统一注入 `User-Agent: CraftWork/<版本>`，调用方不再各自设置——统一口径与代理同一理由：
//! 让网关能识别流量来源，且只有一个值可维护。用户为提供商 / MCP 配置的自定义头在请求级最后
//! 附加，仍可覆盖本默认值（reqwest 只在请求头缺失时才补客户端默认头）。
//!
//! gix 出口是已知缺口：gix-transport 把请求级 `User-Agent: git/oxide-<ver>` 写死，reqwest
//! 后端的客户端级默认头覆盖不了请求已带的头，而唯一的请求级钩子会让 gix 拒绝跟随重定向
//! （见 `craft-git::transport` 模块头「UA 注入缺口」）。

/// 出站请求的 `User-Agent`：`CraftWork/<版本>`，版本取 workspace 版本（各 crate 用同一份
/// `version.workspace = true`，与 `src-tauri/tauri.conf.json` 的 `version` 同步维护）。
pub const APP_USER_AGENT: &str = concat!("CraftWork/", env!("CARGO_PKG_VERSION"));

/// 内置本地排除（curl NOPROXY 语法，不可配置）：自定义代理模式下 reqwest 与 gix 两侧
/// 同传。目的：本地模型端点（如 ollama 的 127.0.0.1）永远直连。系统代理模式不注入
/// （尊重系统自身排除与环境变量 NO_PROXY）；无代理模式无此概念。
pub const BUILTIN_NO_PROXY: &str =
    "localhost,127.0.0.0/8,::1,169.254.0.0/16,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16";

/// GitHub 加速改写覆盖的 host 白名单；其余 host（含 api.github.com）一期不改写。
const GITHUB_HOSTS: [&str; 2] = ["github.com", "raw.githubusercontent.com"];

/// 代理三模式（spec 第 1 节）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyMode {
    /// 系统代理（默认）：环境变量 > OS 系统代理 > 直连
    System,
    /// 无代理：显式禁用一切代理（含环境变量）
    None,
    /// 自定义代理：使用设置里的 proxyUrl
    Custom,
}

impl ProxyMode {
    /// 解析设置字符串；未知值回落 system（与仓库「越界值回落默认」惯例一致）。
    pub fn parse(value: &str) -> Self {
        match value {
            "none" => Self::None,
            "custom" => Self::Custom,
            _ => Self::System,
        }
    }
}

/// 解析产物：两个出口（reqwest / gix）统一消费的配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyConfig {
    /// 解析后的模式
    pub mode: ProxyMode,
    /// 实际生效的代理 URL：
    /// - none：恒 None（显式直连）
    /// - custom：设置里归一校验后的 URL
    /// - system：环境变量 > 系统代理读出的 URL；都没有则 None（直连）
    pub effective_url: Option<String>,
}

/// gix 传输选项的中性表达：craft-core 不依赖 gix，由 craft-git 将来把本结构转换成
/// `gix::conn::transport::Options` 的对应字段（空串语义必须原样保留）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GixProxyOptions {
    /// 代理 URL；Some("") = 完全禁用代理（连环境变量一并禁用，curl 语义）；None = 不干预
    /// （gix 默认行为，自行读环境变量）
    pub proxy: Option<String>,
    /// NOPROXY 排除串（curl NOPROXY 语法）；None = 不注入
    pub no_proxy: Option<String>,
}

impl ProxyConfig {
    /// 生产入口：读真实进程环境变量与 OS 系统代理。逻辑全部在 [`Self::resolve`]（纯函数）。
    pub fn from_settings(mode: &str, proxy_url: &str) -> Result<Self, String> {
        Self::resolve(mode, proxy_url, &env_lookup_std, &system_proxy)
    }

    /// 纯函数解析（可测试）：环境变量经 `env` 注入、系统代理经 `system` 注入。
    ///
    /// - none：显式直连（不校验 proxyUrl——该模式下此字段无意义）
    /// - custom：proxyUrl 必须非空且协议合法（非法返回 Err，保存与运行时双侧拒绝）
    /// - system：环境变量（https_proxy > http_proxy > all_proxy）> 系统代理 > 直连
    pub fn resolve(
        mode: &str,
        proxy_url: &str,
        env: &dyn Fn(&str) -> Option<String>,
        system: &dyn Fn() -> Option<String>,
    ) -> Result<Self, String> {
        let mode = ProxyMode::parse(mode);
        let effective_url = match mode {
            ProxyMode::None => None,
            ProxyMode::Custom => Some(normalize_proxy_url(proxy_url)?),
            ProxyMode::System => env_proxy_url(env).or_else(system),
        };
        Ok(Self { mode, effective_url })
    }

    /// 出口一：把解析结果应用到 reqwest 客户端构建器（模型调用、提供商探活等）。
    ///
    /// 先按模式装代理，再注入统一的 [`APP_USER_AGENT`]（用户自定义请求头仍可在请求级覆盖）：
    /// - custom：`Proxy::all(归一 URL)` + 内置本地排除
    /// - system：读到代理 URL 则显式 `Proxy::all`（不注入排除，尊重环境 NO_PROXY）；
    ///   没读到则显式 `no_proxy()`（此时环境里本就无代理，行为等同直连，且不受库的
    ///   隐式环境探测影响）
    /// - none：`ClientBuilder::no_proxy()`（清空代理含环境变量）
    ///
    /// 本模块不启用 reqwest 的 system-proxy feature：系统代理由应用读出后显式喂，
    /// 是两个出口语义统一的唯一实现（spec 第 2 节）。
    pub fn apply_reqwest(&self, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        match self.reqwest_proxy() {
            Some(proxy) => builder.proxy(proxy),
            None => builder.no_proxy(),
        }
        .user_agent(APP_USER_AGENT)
    }

    /// 出口三（blocking）：语义与 [`Self::apply_reqwest`] 完全一致，供壳层 blocking 场景
    /// （provider_test、provider_fetch_models）使用。reqwest 的 blocking builder 与
    /// async builder 是两个类型，无法共用签名，这里以同一份解析结果各自应用。
    pub fn apply_reqwest_blocking(
        &self,
        builder: reqwest::blocking::ClientBuilder,
    ) -> reqwest::blocking::ClientBuilder {
        match self.reqwest_proxy() {
            Some(proxy) => builder.proxy(proxy),
            None => builder.no_proxy(),
        }
        .user_agent(APP_USER_AGENT)
    }

    /// 两个 reqwest 出口共用的代理对象构造：Some = 经代理（custom 模式注入内置本地排除），
    /// None = 显式直连（无有效代理 URL，或 URL 已归一校验仍解析失败——禁代理比走错代理安全）。
    fn reqwest_proxy(&self) -> Option<reqwest::Proxy> {
        let url = self.effective_url.as_deref()?;
        let proxy = reqwest::Proxy::all(url).ok()?;
        Some(match self.mode {
            // 仅自定义代理模式注入内置排除；系统模式尊重环境 NO_PROXY。
            // from_string 返回 Option<NoProxy>（None = 不注入），直接传即可。
            ProxyMode::Custom => proxy.no_proxy(reqwest::NoProxy::from_string(BUILTIN_NO_PROXY)),
            _ => proxy,
        })
    }

    /// 出口二：转换成 gix 传输选项（中性结构，空串语义原样保留）。语义见 [`GixProxyOptions`]。
    pub fn gix_proxy_options(&self) -> GixProxyOptions {
        match (&self.mode, &self.effective_url) {
            // 无代理：Some("") 让 gix 连环境变量一并禁用（curl 语义）
            (ProxyMode::None, _) => GixProxyOptions { proxy: Some(String::new()), no_proxy: None },
            // 自定义：同一份 URL + 同一份内置排除
            (ProxyMode::Custom, Some(url)) => {
                GixProxyOptions { proxy: Some(url.clone()), no_proxy: Some(BUILTIN_NO_PROXY.to_string()) }
            }
            // 系统模式读到 URL：显式喂同一份 URL；不注入排除（尊重环境 NO_PROXY）
            (_, Some(url)) => GixProxyOptions { proxy: Some(url.clone()), no_proxy: None },
            // 系统模式没读到：不干预（gix 默认行为，自行读环境变量——环境里本就无代理）
            (_, None) => GixProxyOptions::default(),
        }
    }
}

/// proxyUrl 归一校验（spec 第 1 节）：无协议头补 `http://`；仅接受 http / https /
/// socks5 / socks5h。返回归一后的 URL；空串或协议不在白名单时返回错误。
pub fn normalize_proxy_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("代理地址为空".to_string());
    }
    let candidate =
        if has_scheme(trimmed) { trimmed.to_string() } else { format!("http://{trimmed}") };
    let scheme = candidate.split("://").next().unwrap_or_default().to_ascii_lowercase();
    if matches!(scheme.as_str(), "http" | "https" | "socks5" | "socks5h") {
        Ok(candidate)
    } else {
        Err(format!("不支持的代理协议：{scheme}（仅支持 http / https / socks5 / socks5h）"))
    }
}

/// GitHub 加速前缀归一（spec 第 1 节）：去首尾空白并保证以 `/` 结尾；空串原样返回。
pub fn normalize_accelerator_prefix(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    }
}

/// GitHub 加速改写（spec 第 4 节）：仅 host 为 github.com / raw.githubusercontent.com 的
/// URL 改写为 `前缀 + 原URL`（如 `https://gh-proxy.com/https://github.com/owner/repo.git`）；
/// 前缀自动补尾斜杠；空前缀或其余 host 原样返回。纯函数，git 下载与未来的 release /
/// raw 下载共用。
pub fn rewrite_github_url(url: &str, prefix: &str) -> String {
    if prefix.trim().is_empty() {
        return url.to_string();
    }
    if !GITHUB_HOSTS.contains(&url_host(url).to_ascii_lowercase().as_str()) {
        return url.to_string();
    }
    format!("{}{url}", normalize_accelerator_prefix(prefix))
}

/// 判断字符串是否带 `scheme://` 头（scheme 非空且只含 RFC 3986 允许字符）。
/// `127.0.0.1:7890` 这类纯 host:port 不算带协议头，交给调用方补 `http://`。
fn has_scheme(url: &str) -> bool {
    match url.split_once("://") {
        Some((scheme, _)) => {
            !scheme.is_empty()
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        None => false,
    }
}

/// 提取 URL 的 host（去掉 scheme、凭据与端口）；解析失败返回空串。
/// 仅用于加速 host 白名单匹配，不追求完整 URL 语法。
fn url_host(url: &str) -> &str {
    let Some((_, rest)) = url.split_once("://") else { return "" };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    // host:port 取冒号前段（白名单里没有 IPv6 地址，无需处理方括号）
    authority.split(':').next().unwrap_or_default()
}

/// 环境变量代理查找顺序：https_proxy > http_proxy > all_proxy（每个名字先小写后大写，
/// 与 curl / Go 的常见约定一致）；值为空白视为未设置，继续往下回落。
fn env_proxy_url(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    const NAMES: [&str; 6] =
        ["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY", "all_proxy", "ALL_PROXY"];
    NAMES.iter().find_map(|name| {
        env(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    })
}

/// 生产侧薄包装：读真实进程环境变量。
fn env_lookup_std(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// 读 OS 系统代理：macOS 经 SCDynamicStore（与 reqwest system-proxy 同款底层），优先
/// HTTPS 代理、回退 HTTP；任何一步失败（会话建立失败、键缺失、未启用、值类型不符）都
/// 返回 None（调用方回落直连）。Windows 等其他平台直接返回 None（注册表读取随 D7 跨
/// 平台节奏补，一期 macOS 实测）。
#[cfg(target_os = "macos")]
pub fn system_proxy() -> Option<String> {
    use system_configuration::core_foundation::base::CFType;
    use system_configuration::core_foundation::dictionary::CFDictionary;
    use system_configuration::core_foundation::number::CFNumber;
    use system_configuration::core_foundation::string::CFString;
    use system_configuration::dynamic_store::SCDynamicStoreBuilder;

    // 从 SCDynamicStoreCopyProxiesDictionary 读一个协议块（enabled / host / port），
    // 拼出 reqwest 与 gix 都能用的 `http://host:port` 形式。
    fn block_url(
        proxies: &CFDictionary<CFString, CFType>,
        enable_key: &str,
        host_key: &str,
        port_key: &str,
    ) -> Option<String> {
        let enabled = proxies.find(CFString::new(enable_key))?.downcast::<CFNumber>()?.to_i32()?;
        if enabled == 0 {
            return None;
        }
        let host = proxies.find(CFString::new(host_key))?.downcast::<CFString>()?.to_string();
        if host.is_empty() {
            return None;
        }
        let port = proxies.find(CFString::new(port_key))?.downcast::<CFNumber>()?.to_i32()?;
        Some(format!("http://{host}:{port}"))
    }

    let store = SCDynamicStoreBuilder::new("craftwork-network").build()?;
    let proxies = store.get_proxies()?;
    block_url(&proxies, "HTTPSEnable", "HTTPSProxy", "HTTPSPort")
        .or_else(|| block_url(&proxies, "HTTPEnable", "HTTPProxy", "HTTPPort"))
}

#[cfg(not(target_os = "macos"))]
pub fn system_proxy() -> Option<String> {
    None
}

// ---------- 测试 ----------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// 构造环境变量注入闭包（BTreeMap 快照，测试并行互不污染）
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |key: &str| map.get(key).cloned()
    }

    fn no_env() -> impl Fn(&str) -> Option<String> {
        env_of(&[])
    }

    fn no_system() -> Option<String> {
        None
    }

    fn resolve_ok(mode: &str, url: &str, env: &dyn Fn(&str) -> Option<String>) -> ProxyConfig {
        ProxyConfig::resolve(mode, url, env, &no_system).expect("解析应成功")
    }

    // ---------- URL 归一 ----------

    #[test]
    fn proxy_url_without_scheme_gets_http_prefix() {
        assert_eq!(normalize_proxy_url("127.0.0.1:7890").unwrap(), "http://127.0.0.1:7890");
        assert_eq!(normalize_proxy_url(" 127.0.0.1:7890 ").unwrap(), "http://127.0.0.1:7890");
    }

    #[test]
    fn proxy_url_accepts_whitelisted_schemes() {
        for raw in
            ["http://p:1", "https://p:1", "socks5://127.0.0.1:1080", "socks5h://127.0.0.1:1080"]
        {
            assert_eq!(normalize_proxy_url(raw).unwrap(), raw);
        }
    }

    #[test]
    fn proxy_url_rejects_empty_and_unknown_scheme() {
        assert!(normalize_proxy_url("").is_err());
        assert!(normalize_proxy_url("   ").is_err());
        assert!(normalize_proxy_url("ftp://127.0.0.1:21").is_err());
        assert!(normalize_proxy_url("file:///tmp/x").is_err());
    }

    #[test]
    fn accelerator_prefix_gets_trailing_slash() {
        assert_eq!(normalize_accelerator_prefix("https://gh-proxy.com"), "https://gh-proxy.com/");
        assert_eq!(normalize_accelerator_prefix("https://gh-proxy.com/"), "https://gh-proxy.com/");
        assert_eq!(normalize_accelerator_prefix(""), "");
    }

    // ---------- 三模式 × 环境变量矩阵 ----------

    #[test]
    fn custom_mode_uses_normalized_url_and_ignores_env() {
        let env = env_of(&[("https_proxy", "http://env:1")]);
        let cfg = resolve_ok("custom", "127.0.0.1:7890", &env);
        assert_eq!(cfg.mode, ProxyMode::Custom);
        assert_eq!(cfg.effective_url.as_deref(), Some("http://127.0.0.1:7890"));
    }

    #[test]
    fn custom_mode_rejects_empty_and_invalid_url() {
        assert!(ProxyConfig::resolve("custom", "", &no_env(), &no_system).is_err());
        assert!(ProxyConfig::resolve("custom", "ftp://x", &no_env(), &no_system).is_err());
    }

    #[test]
    fn none_mode_forces_direct_even_with_env_proxy() {
        let env = env_of(&[("https_proxy", "http://env:1"), ("all_proxy", "socks5://env:2")]);
        let cfg = resolve_ok("none", "whatever", &env);
        assert_eq!(cfg.mode, ProxyMode::None);
        assert_eq!(cfg.effective_url, None);
    }

    #[test]
    fn system_mode_env_https_takes_precedence() {
        let env = env_of(&[("https_proxy", "http://https:1"), ("http_proxy", "http://http:2")]);
        let cfg = resolve_ok("system", "", &env);
        assert_eq!(cfg.effective_url.as_deref(), Some("http://https:1"));
    }

    #[test]
    fn system_mode_env_http_then_all_fallback() {
        let env = env_of(&[("http_proxy", "http://http:2"), ("all_proxy", "socks5://all:3")]);
        assert_eq!(resolve_ok("system", "", &env).effective_url.as_deref(), Some("http://http:2"));
        let env_all = env_of(&[("all_proxy", "socks5://all:3")]);
        assert_eq!(resolve_ok("system", "", &env_all).effective_url.as_deref(), Some("socks5://all:3"));
    }

    #[test]
    fn system_mode_reads_uppercase_env_names() {
        let env = env_of(&[("HTTPS_PROXY", "http://upper:1")]);
        assert_eq!(resolve_ok("system", "", &env).effective_url.as_deref(), Some("http://upper:1"));
    }

    #[test]
    fn system_mode_env_empty_value_falls_through() {
        let env = env_of(&[("https_proxy", "  "), ("http_proxy", "http://http:2")]);
        assert_eq!(resolve_ok("system", "", &env).effective_url.as_deref(), Some("http://http:2"));
    }

    #[test]
    fn system_mode_falls_back_to_system_proxy_then_direct() {
        // 环境变量无代理 → 用注入的系统代理
        let cfg = ProxyConfig::resolve("system", "", &no_env(), &|| {
            Some("http://sys:8080".to_string())
        })
        .unwrap();
        assert_eq!(cfg.effective_url.as_deref(), Some("http://sys:8080"));
        // 都没有 → 直连
        let cfg = resolve_ok("system", "", &no_env());
        assert_eq!(cfg.effective_url, None);
    }

    #[test]
    fn unknown_mode_falls_back_to_system() {
        let env = env_of(&[("http_proxy", "http://http:2")]);
        let cfg = resolve_ok("bogus", "", &env);
        assert_eq!(cfg.mode, ProxyMode::System);
        assert_eq!(cfg.effective_url.as_deref(), Some("http://http:2"));
    }

    // ---------- 两个出口 ----------

    #[test]
    fn gix_options_none_mode_disables_everything() {
        let cfg = resolve_ok("none", "", &env_of(&[("https_proxy", "http://env:1")]));
        let opts = cfg.gix_proxy_options();
        assert_eq!(opts.proxy.as_deref(), Some(""));
        assert_eq!(opts.no_proxy, None);
    }

    #[test]
    fn gix_options_custom_mode_passes_url_and_builtin_exclusions() {
        let cfg = resolve_ok("custom", "http://127.0.0.1:7890", &no_env());
        let opts = cfg.gix_proxy_options();
        assert_eq!(opts.proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(opts.no_proxy.as_deref(), Some(BUILTIN_NO_PROXY));
    }

    #[test]
    fn gix_options_system_mode_matches_resolution() {
        // 读到代理：显式同一份 URL，不注入排除（尊重环境 NO_PROXY）
        let env = env_of(&[("https_proxy", "http://env:1")]);
        let opts = resolve_ok("system", "", &env).gix_proxy_options();
        assert_eq!(opts, GixProxyOptions { proxy: Some("http://env:1".into()), no_proxy: None });
        // 没读到：不干预（gix 默认行为）
        let opts = resolve_ok("system", "", &no_env()).gix_proxy_options();
        assert_eq!(opts, GixProxyOptions::default());
    }

    #[test]
    fn apply_reqwest_builds_client_for_all_modes() {
        let env = env_of(&[("https_proxy", "http://env:1")]);
        for (mode, url) in
            [("none", ""), ("system", ""), ("system", "irrelevant"), ("custom", "http://127.0.0.1:7890")]
        {
            let cfg = resolve_ok(mode, url, &env);
            cfg.apply_reqwest(reqwest::Client::builder()).build().expect("客户端应可构建");
        }
    }

    #[test]
    fn socks_urls_construct_reqwest_proxy() {
        // 依赖 craft-core 的 reqwest socks feature：socks5/socks5h 地址能被 Proxy::all 解析
        assert!(reqwest::Proxy::all("socks5://127.0.0.1:1080").is_ok());
        assert!(reqwest::Proxy::all("socks5h://127.0.0.1:1080").is_ok());
    }

    // ---------- GitHub 加速改写 ----------

    #[test]
    fn rewrite_prefixes_github_hosts() {
        let prefix = "https://gh-proxy.com/";
        assert_eq!(
            rewrite_github_url("https://github.com/owner/repo.git", prefix),
            "https://gh-proxy.com/https://github.com/owner/repo.git"
        );
        assert_eq!(
            rewrite_github_url("https://raw.githubusercontent.com/owner/repo/main/README.md", prefix),
            "https://gh-proxy.com/https://raw.githubusercontent.com/owner/repo/main/README.md"
        );
    }

    #[test]
    fn rewrite_normalizes_prefix_and_host_case() {
        assert_eq!(
            rewrite_github_url("https://github.com/o/r", "https://gh-proxy.com"),
            "https://gh-proxy.com/https://github.com/o/r"
        );
        assert_eq!(
            rewrite_github_url("https://GitHub.com/o/r", "https://gh-proxy.com/"),
            "https://gh-proxy.com/https://GitHub.com/o/r"
        );
    }

    #[test]
    fn rewrite_leaves_other_hosts_untouched() {
        let prefix = "https://gh-proxy.com/";
        assert_eq!(rewrite_github_url("https://api.github.com/repos", prefix), "https://api.github.com/repos");
        assert_eq!(rewrite_github_url("https://example.com/x", prefix), "https://example.com/x");
        // 非法形态（无 scheme）也原样返回
        assert_eq!(rewrite_github_url("github.com/o/r", prefix), "github.com/o/r");
    }

    #[test]
    fn rewrite_with_empty_prefix_returns_original() {
        assert_eq!(rewrite_github_url("https://github.com/o/r", ""), "https://github.com/o/r");
        assert_eq!(rewrite_github_url("https://github.com/o/r", "  "), "https://github.com/o/r");
    }

    // ---------- 内置排除常量 ----------

    #[test]
    fn builtin_no_proxy_covers_local_and_private_ranges() {
        let parts: Vec<&str> = BUILTIN_NO_PROXY.split(',').collect();
        assert_eq!(
            parts,
            vec![
                "localhost",
                "127.0.0.0/8",
                "::1",
                "169.254.0.0/16",
                "10.0.0.0/8",
                "172.16.0.0/12",
                "192.168.0.0/16"
            ]
        );
    }

    #[test]
    fn builtin_no_proxy_parses_as_reqwest_no_proxy() {
        // 常量必须是 reqwest（NoProxy）与 curl NOPROXY 都能接受的语法
        assert!(reqwest::NoProxy::from_string(BUILTIN_NO_PROXY).is_some());
    }
}
