//! 模型客户端工厂：按引用解析提供商 → 构造协议客户端（统一套重试）→ 密钥校验与回退链。

use std::sync::{Arc, Mutex};

use crate::anthropic::AnthropicClient;
use crate::catalog::Catalog;
use crate::errors::{missing_endpoint_error, ModelError};
use crate::openai::OpenAICompatClient;
use crate::retry::{ClampMaxTokensClient, RetryClient};
use crate::types::{Client, LogHook, ModelInfo};
use crate::errors;

/// 密钥来源抽象（生产为 craft_core::secret::SecretStore 适配，测试注入 stub）。
pub trait KeyProvider: Send + Sync {
    /// 读取提供商密钥：env_names 为目录带入的候选环境变量名；无密钥返回空串。
    fn get_for(&self, provider_id: &str, env_names: &[String]) -> String;
}

/// SecretStore 适配（env_names 为 Vec<String> 与 PersistedProvider.envVars 对齐）。
pub struct SecretKeyProvider {
    pub secrets: Arc<craft_core::secret::SecretStore>,
}

impl KeyProvider for SecretKeyProvider {
    fn get_for(&self, provider_id: &str, env_names: &[String]) -> String {
        let refs: Vec<&str> = env_names.iter().map(String::as_str).collect();
        self.secrets.get_for(provider_id, &refs)
    }
}

/// 失败转移候选的实时解析器（每次读设置，改设置免重启；未设置时无候选）。
type FallbackResolver = Box<dyn Fn() -> String + Send>;

/// 客户端构造替换器（仅测试使用）：给定模型引用直接返回客户端，跳过目录 / 密钥 /
/// 协议适配。存在理由——Runner 的完整链路（流消费、工具执行、事件落库、子代理派发）
/// 必须有可重复的确定性模型输出，否则只能靠真实网络，测不稳也测不快。
type ClientOverride = Box<dyn Fn(&str) -> Option<Box<dyn Client>> + Send + Sync>;

/// 模型客户端工厂：目录解析 → 停用拦截 → 密钥校验 → 按协议构造客户端（统一包重试）。
pub struct ModelFactory {
    pub catalog: Arc<Catalog>,
    keys: Box<dyn KeyProvider>,
    fallback_resolver: Mutex<Option<FallbackResolver>>,
    /// 观测日志回调（重试等提供商层事件；构造客户端时下传）
    log_hook: Mutex<Option<LogHook>>,
    /// 客户端替换（仅测试注入；生产装配从不设置它）
    client_override: Mutex<Option<ClientOverride>>,
}

impl ModelFactory {
    /// 创建工厂（keys 通常为 SecretKeyProvider，测试可注入 stub）。
    pub fn new(catalog: Arc<Catalog>, keys: Box<dyn KeyProvider>) -> Self {
        Self {
            catalog,
            keys,
            fallback_resolver: Mutex::new(None),
            log_hook: Mutex::new(None),
            client_override: Mutex::new(None),
        }
    }

    /// 注入客户端替换（仅测试使用）：返回 Some 时 `client()` 直接用它，不查目录与密钥。
    /// 供 Runner 集成测试构造确定性模型输出（流式片段序列）。
    pub fn set_client_override(&self, f: ClientOverride) {
        *self.client_override.lock().unwrap() = Some(f);
    }

    /// 注册观测日志回调（重试等事件；改注册立即生效）。
    pub fn set_log_hook(&self, hook: LogHook) {
        *self.log_hook.lock().unwrap() = Some(hook);
    }

    /// 解析引用或原始 ID 为模型信息（找不到返回 None）。
    pub fn resolve(&self, ref_or_id: &str) -> Option<ModelInfo> {
        self.catalog.resolve(ref_or_id)
    }

    /// 提供商协议类型（费用估算的缓存口径分支用）。
    pub fn provider_kind(&self, provider_id: &str) -> String {
        self.catalog.find_provider(provider_id).map(|p| p.kind.clone()).unwrap_or_default()
    }

    /// 目录句柄（壳层组装用）。
    pub fn catalog_arc(&self) -> Arc<Catalog> {
        Arc::clone(&self.catalog)
    }

    /// 实时读取备用模型设置（改设置免重启）。
    pub fn set_fallback_resolver(&self, f: FallbackResolver) {
        *self.fallback_resolver.lock().unwrap() = Some(f);
    }

    /// 构造可调用客户端：解析引用 → 提供商/模型停用拦截 → 密钥校验 → 按协议构造并包重试。
    /// 不存在、停用、缺密钥等配置问题在此返回可读 ModelError，不发起网络请求。
    /// proxy 为出站代理配置（调用方每次调用前从 settings 即时解析传入——client 每 run
    /// 新建，设置保存即时生效；None = system 模式兜底）。
    pub fn client(
        &self,
        ref_or_id: &str,
        proxy: Option<&craft_core::network::ProxyConfig>,
    ) -> Result<Box<dyn Client>, ModelError> {
        // 测试注入优先：生产装配从不设置替换器
        if let Some(overridden) = self.client_override.lock().unwrap().as_ref().and_then(|f| f(ref_or_id)) {
            return Ok(overridden);
        }
        let info = self
            .catalog
            .resolve(ref_or_id)
            .ok_or_else(|| ModelError::new(errors::code::PROVIDER_UNAVAILABLE, ref_or_id, "模型不存在或已被删除，请重新选择模型", 0))?;
        let provider = self
            .catalog
            .find_provider(&info.provider)
            .ok_or_else(|| ModelError::new(errors::code::PROVIDER_UNAVAILABLE, &info.provider, "提供商不存在，请重新选择模型", 0))?;
        let display_name = if provider.name.is_empty() { provider.id.clone() } else { provider.name.clone() };
        // 启停语义在调用侧收口（spec §9）：停用提供商 / 模型不可调用，但 resolve/findModel
        // 保持不变——历史会话引用仍可解析显示名称，只在真正调用时给出可读停用提示。
        if !provider.enabled {
            return Err(ModelError::new(
                errors::code::PROVIDER_UNAVAILABLE,
                &provider.id,
                format!("{display_name}：该模型已停用，请重新选择模型"),
                0,
            ));
        }
        if provider
            .models
            .iter()
            .find(|m| m.id == info.id)
            .is_some_and(|persisted| !persisted.enabled)
        {
            return Err(ModelError::new(
                errors::code::PROVIDER_UNAVAILABLE,
                &provider.id,
                format!("{display_name}：该模型已停用，请重新选择模型"),
                0,
            ));
        }
        let key = self.keys.get_for(&provider.id, &provider.env_vars);
        if provider.key_required && key.is_empty() {
            return Err(ModelError::new(
                errors::code::AUTH,
                &provider.name,
                format!("{} 尚未配置密钥，请在设置 → 模型 中粘贴 API Key", provider.name),
                0,
            ));
        }
        let inner: Box<dyn Client> = if provider.kind == "anthropic" {
            Box::new(AnthropicClient::new(&provider.endpoint, &key, &display_name, proxy).with_headers(provider.headers.clone()))
        } else {
            // openai 与 openai-compatible 共用 OpenAI 协议适配器
            Box::new(OpenAICompatClient::new(&provider.endpoint, &key, &display_name, proxy).with_headers(provider.headers.clone()))
        };
        // 重试参数：3 次尝试、700ms 基础退避、5s 上限；重试事件经日志回调观测。
        // max_tokens 超限自愈包在协议客户端外、退避重试内：网关 400 换参重发后，
        // 仍交给退避层处理后续的可重试错误。
        let log_hook = self.log_hook.lock().unwrap().clone();
        let healed: Box<dyn Client> =
            Box::new(ClampMaxTokensClient::new(inner, &display_name, log_hook.clone()));
        Ok(Box::new(RetryClient::new(healed, &display_name, 3, 700, 5000, log_hook)))
    }

    /// 失败转移候选：备用引用存在、与当前不同、可解析、未停用、需要密钥时密钥可得，
    /// 才返回候选引用；任一条件不满足返回空串（空串 = 本次不转移）。
    pub fn fallback_model(&self, ref_or_id: &str) -> String {
        let candidate = {
            let guard = self.fallback_resolver.lock().unwrap();
            match guard.as_ref() {
                Some(f) => f(),
                None => return String::new(),
            }
        };
        if candidate.is_empty() {
            return String::new();
        }
        let current = self.catalog.resolve(ref_or_id);
        let fallback = self.catalog.resolve(&candidate);
        let (Some(current), Some(fallback)) = (current, fallback) else {
            return String::new();
        };
        let current_ref = format!("{}/{}", current.provider, current.id);
        let fallback_ref = format!("{}/{}", fallback.provider, fallback.id);
        if current_ref == fallback_ref {
            return String::new();
        }
        let Some(provider) = self.catalog.find_provider(&fallback.provider) else {
            return String::new();
        };
        // 与「密钥不可得返回空」同模式：停用候选不参与失败转移（spec §9）
        if !provider.enabled {
            return String::new();
        }
        if provider.models.iter().any(|m| m.id == fallback.id && !m.enabled) {
            return String::new();
        }
        if provider.key_required && self.keys.get_for(&provider.id, &provider.env_vars).is_empty() {
            return String::new();
        }
        fallback_ref
    }
}

/// 缺失端点错误（供壳层复用）。
pub fn missing_endpoint(provider: &str) -> ModelError {
    missing_endpoint_error(provider)
}
