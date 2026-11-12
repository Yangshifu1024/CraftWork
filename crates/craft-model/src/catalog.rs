//! 提供商目录：内置 models.dev 快照（候选列表，只读）+ 用户自定义提供商（可调用）。
//! 原则：应用不内置任何可用提供商——内置目录只提供「可添加候选」，调用必须经用户添加。
//! 内置快照经 include_bytes 编译期嵌入（assets/catalog/catalog.json.gz，modelsync 产物同源）。

use std::io::Read;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use craft_store::db::Db;
use craft_store::records::get_setting;
use crate::types::{Capabilities, Cost, ModelInfo, PersistedModel, PersistedProvider, DEFAULT_CAPS};

/// settings 键：用户提供商数组（PersistedProvider JSON，可调用提供商的唯一事实源）。
pub const CUSTOM_PROVIDERS_KEY: &str = "model.custom_providers";

// ---------- 内置目录（models.dev 快照） ----------

/// 内置目录中的模型条目（models.dev 快照原始形状，字段可缺省，经归一化后转持久化形状）。
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub context_k: i64,
    #[serde(default)]
    pub max_tokens: Option<i64>,
    #[serde(default)]
    pub caps: Option<PartialCaps>,
    #[serde(default)]
    pub cost: Option<RawCost>,
    #[serde(default)]
    pub reasoning_options: Vec<RawReasoningOption>,
    #[serde(default, rename = "status")]
    pub status: Option<String>,
    #[serde(default, rename = "releaseDate")]
    pub release_date: Option<String>,
    #[serde(default, rename = "canonicalId")]
    pub canonical_id: Option<String>,
}

/// models.dev reasoning_options 元素：effort 型带 values；budget_tokens 型带 min/max；toggle 无载荷。
#[derive(Debug, Clone, Deserialize)]
pub struct RawReasoningOption {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub values: Option<Vec<String>>,
    #[serde(default)]
    pub min: Option<i64>,
    #[serde(default)]
    pub max: Option<i64>,
}

/// reasoning_options → (kind, levels)：
/// effort 型档位取 values 原样；含 budget_tokens（toggle 组合或单置）归 budget、档位 off/low/high；
/// 纯 toggle / 未知类型不支持（空），UI 不出档位。
pub fn normalize_reasoning(options: &[RawReasoningOption]) -> (String, Vec<String>) {
    if let Some(effort) = options
        .iter()
        .find(|o| o.kind == "effort" && o.values.as_ref().is_some_and(|v| !v.is_empty()))
    {
        return ("effort".into(), effort.values.clone().unwrap_or_default());
    }
    if options.iter().any(|o| o.kind == "budget_tokens") {
        return ("budget".into(), vec!["off".into(), "low".into(), "high".into()]);
    }
    (String::new(), Vec::new())
}

/// 目录能力标记的稀疏形状：未声明字段回退 DEFAULT_CAPS。
#[derive(Debug, Clone, Deserialize)]
pub struct PartialCaps {
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub tools: Option<bool>,
    #[serde(default)]
    pub vision: Option<bool>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub structured: Option<bool>,
}

/// 目录价格原始形状（美元 / 百万 token，字段可缺省）。
#[derive(Debug, Clone, Deserialize)]
pub struct RawCost {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default, rename = "cache_read")]
    pub cache_read: Option<f64>,
    #[serde(default, rename = "cache_write")]
    pub cache_write: Option<f64>,
}

/// 内置目录中的提供商条目：仅是「可添加候选」，未添加前不可调用。
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogProvider {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub doc: Option<String>,
    #[serde(default)]
    pub env: Option<Vec<String>>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub models: Vec<CatalogModel>,
}

/// 目录快照顶层结构（schema 只认 1，见 load_dataset 的校验）。
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogDataset {
    pub schema: i64,
    #[serde(rename = "generatedAt", default)]
    pub generated_at: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub providers: Vec<CatalogProvider>,
}

static BUILTIN: OnceLock<CatalogDataset> = OnceLock::new();

/// 解压并解析内置快照（进程内单例，首次访问时加载）。
/// 损坏 / schema 不支持 / 空目录直接 panic：快照是编译期产物，出错属构建缺陷，
/// 宁可启动失败也不带病运行。
fn load_dataset() -> CatalogDataset {
    static GZ: &[u8] = include_bytes!("../../../assets/catalog/catalog.json.gz");
    let mut decoder = flate2::read::GzDecoder::new(GZ);
    let mut json = String::new();
    decoder
        .read_to_string(&mut json)
        .unwrap_or_else(|e| panic!("目录快照解压失败：{e}"));
    let data: CatalogDataset =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("目录快照解析失败：{e}"));
    if data.schema != 1 {
        panic!("目录快照 schema 不支持：{}", data.schema);
    }
    if data.providers.is_empty() {
        panic!("目录快照为空");
    }
    data
}

/// 内置目录单例入口（只读候选列表）。
pub fn builtin_catalog() -> &'static CatalogDataset {
    BUILTIN.get_or_init(load_dataset)
}

// ---------- 内置目录 → 持久化形状（从内置目录导入时） ----------

/// 目录模型条目 → 持久化形状（用户从目录添加模型时调用，默认启用）。
pub fn catalog_model_to_persisted(m: &CatalogModel, source: &str) -> PersistedModel {
    let (reasoning_kind, reasoning_levels) = normalize_reasoning(&m.reasoning_options);
    PersistedModel {
        id: m.id.clone(),
        name: m.name.clone(),
        context_k: m.context_k,
        max_tokens: m.max_tokens.unwrap_or(0),
        caps: merge_caps(m.caps.as_ref()),
        cost: m.cost.as_ref().map(|c| Cost {
            input_per_m_tok: c.input.unwrap_or(0.0),
            output_per_m_tok: c.output.unwrap_or(0.0),
            cache_read_per_m_tok: c.cache_read.unwrap_or(0.0),
            cache_write_per_m_tok: c.cache_write.unwrap_or(0.0),
        }),
        status: m.status.clone().unwrap_or_default(),
        canonical_id: m.canonical_id.clone().unwrap_or_default(),
        release_date: m.release_date.clone().unwrap_or_default(),
        source: source.into(),
        enabled: true,
        reasoning_kind,
        reasoning_levels,
    }
}

/// 公开版本（壳层目录视图用）。
pub fn merge_caps_public(partial: Option<&PartialCaps>) -> Capabilities {
    merge_caps(partial)
}

/// 稀疏能力标记与缺省基线合并：未声明的字段取 DEFAULT_CAPS。
fn merge_caps(partial: Option<&PartialCaps>) -> Capabilities {
    let mut caps = DEFAULT_CAPS;
    if let Some(p) = partial {
        if let Some(v) = p.stream {
            caps.stream = v;
        }
        if let Some(v) = p.tools {
            caps.tools = v;
        }
        if let Some(v) = p.vision {
            caps.vision = v;
        }
        if let Some(v) = p.reasoning {
            caps.reasoning = v;
        }
        if let Some(v) = p.structured {
            caps.structured = v;
        }
    }
    caps
}

/// 内置目录提供商的 env 变量名（导入时带入，密钥可从环境变量回退）。
pub fn catalog_provider_env(p: &CatalogProvider) -> Vec<String> {
    p.env.clone().unwrap_or_default()
}

// ---------- 目录查询与解析 ----------

/// 单实例持有 db，custom providers 变化后调 reload。
pub struct Catalog {
    db: Arc<Db>,
    custom: std::sync::Mutex<Vec<PersistedProvider>>,
}

/// 规范化引用：规范 `providerID/modelID` 与模型显示名（序列化给渲染层）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRef {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
}

impl Catalog {
    pub fn new(db: Arc<Db>) -> Catalog {
        let catalog = Catalog { db, custom: std::sync::Mutex::new(Vec::new()) };
        catalog.reload();
        catalog
    }

    /// 重载用户自定义提供商（settings 变更后调用）。
    pub fn reload(&self) {
        let custom = self
            .db
            .with(|conn| get_setting(conn, CUSTOM_PROVIDERS_KEY))
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_value::<Vec<PersistedProvider>>(value).ok())
            .unwrap_or_default(); // 读取失败按空目录：宁可提示重新配置也不崩溃
        *self.custom.lock().unwrap() = custom;
    }

    /// 全部可调用提供商（用户添加的，快照）。
    pub fn providers(&self) -> Vec<PersistedProvider> {
        self.custom.lock().unwrap().clone()
    }

    /// 按 ID 查用户提供商（快照内查找）。
    pub fn find_provider(&self, id: &str) -> Option<PersistedProvider> {
        self.custom.lock().unwrap().iter().find(|p| p.id == id).cloned()
    }

    /// 按提供商 ID + 模型 ID 精确查找（模型引用解析的内层步骤）。
    pub fn find_model(&self, provider_id: &str, model_id: &str) -> Option<ModelInfo> {
        let p = self.find_provider(provider_id)?;
        let m = p.models.iter().find(|x| x.id == model_id)?;
        Some(to_model_info(&p, m))
    }

    /// 按模型引用查找：取第一个斜杠切分（提供商 ID 不含斜杠而模型 ID 可含）。
    pub fn find_model_by_ref(&self, reference: &str) -> Option<ModelInfo> {
        let idx = reference.find('/')?;
        if idx == 0 {
            return None;
        }
        self.find_model(&reference[..idx], &reference[idx + 1..])
    }

    /// 解析引用或原始 ID：先按完整引用（第一个斜杠切分，提供商 ID 无斜杠而模型 ID 可含），
    /// 再按原始模型 ID 全局匹配（兼容旧数据）。找不到返回 None，调用方提示重选，不静默回退。
    pub fn resolve(&self, ref_or_id: &str) -> Option<ModelInfo> {
        if ref_or_id.is_empty() {
            return None;
        }
        if let Some(by_ref) = self.find_model_by_ref(ref_or_id) {
            return Some(by_ref);
        }
        for p in self.custom.lock().unwrap().iter() {
            if let Some(m) = p.models.iter().find(|x| x.id == ref_or_id) {
                return Some(to_model_info(p, m));
            }
        }
        None
    }

    /// 规范化引用：可解析则返回规范 ref 与显示名，否则 None。
    pub fn normalize(&self, ref_or_id: &str) -> Option<NormalizedRef> {
        let info = self.resolve(ref_or_id)?;
        Some(NormalizedRef {
            reference: format!("{}/{}", info.provider, info.id),
            model_name: info.name,
        })
    }

    // ---------- 内置候选目录查询 ----------

    /// 内置目录元信息：(生成时间, 版本, 提供商数, 模型数)。
    pub fn builtin_meta(&self) -> (String, String, usize, usize) {
        let data = builtin_catalog();
        let model_count: usize = data.providers.iter().map(|p| p.models.len()).sum();
        (
            data.generated_at.clone(),
            data.version.clone(),
            data.providers.len(),
            model_count,
        )
    }

    /// 内置目录全部候选提供商（只读）。
    pub fn builtin_providers(&self) -> &[CatalogProvider] {
        &builtin_catalog().providers
    }

    /// 按 ID 查内置候选提供商。
    pub fn builtin_provider(&self, id: &str) -> Option<&CatalogProvider> {
        builtin_catalog().providers.iter().find(|p| p.id == id)
    }

    /// 按 ID / 名称子串过滤候选提供商（空查询返回全部；大小写不敏感）。
    pub fn search_builtin_providers(&self, query: &str) -> Vec<&CatalogProvider> {
        let q = query.trim().to_lowercase();
        let all = self.builtin_providers();
        if q.is_empty() {
            return all.iter().collect();
        }
        all.iter()
            .filter(|p| p.id.to_lowercase().contains(&q) || p.name.to_lowercase().contains(&q))
            .collect()
    }
}

/// 持久化形状 → 解析结果（附带提供商的密钥环境变量与是否必需密钥）。
fn to_model_info(p: &PersistedProvider, m: &PersistedModel) -> ModelInfo {
    ModelInfo {
        id: m.id.clone(),
        name: m.name.clone(),
        provider: p.id.clone(),
        context_k: m.context_k,
        max_tokens: m.max_tokens,
        caps: m.caps.clone(),
        cost: m.cost.clone(),
        status: m.status.clone(),
        canonical_id: m.canonical_id.clone(),
        release_date: m.release_date.clone(),
        source: m.source.clone(),
        env_vars: p.env_vars.clone(),
        key_required: p.key_required,
        reasoning_kind: m.reasoning_kind.clone(),
        reasoning_levels: m.reasoning_levels.clone(),
    }
}

/// 引用规范化的自由函数入口（壳层 store / view 使用，内部走 Catalog::normalize）。
pub fn normalize_model_ref(catalog: &Catalog, ref_or_id: &str) -> Option<NormalizedRef> {
    catalog.normalize(ref_or_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(kind: &str, values: Option<Vec<&str>>) -> RawReasoningOption {
        RawReasoningOption {
            kind: kind.into(),
            values: values.map(|v| v.into_iter().map(String::from).collect()),
            min: None,
            max: None,
        }
    }

    #[test]
    fn reasoning_options_normalization() {
        // effort 型：档位取 values 原样（models.dev 实测形态，gpt/deepseek 系）
        let (kind, levels) = normalize_reasoning(&[
            opt("effort", Some(vec!["minimal", "low", "medium", "high"])),
        ]);
        assert_eq!(kind, "effort");
        assert_eq!(levels, vec!["minimal", "low", "medium", "high"]);

        // toggle + budget_tokens 组合：归 budget、固定 off/low/high（anthropic 系）
        let (kind, levels) =
            normalize_reasoning(&[opt("toggle", None), opt("budget_tokens", None)]);
        assert_eq!(kind, "budget");
        assert_eq!(levels, vec!["off", "low", "high"]);

        // 纯 toggle（GLM/Qwen 等兼容网关开关）：不支持档位选择
        let (kind, levels) = normalize_reasoning(&[opt("toggle", None)]);
        assert_eq!(kind, "");
        assert!(levels.is_empty());

        // 空元数据：不支持
        let (kind, levels) = normalize_reasoning(&[]);
        assert_eq!(kind, "");
        assert!(levels.is_empty());
    }
}
