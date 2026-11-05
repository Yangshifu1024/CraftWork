//! 文档读写：Word（docx）/ Excel（xlsx）/ PDF 的文本级处理。
//! 限制：docx 只读正文；xlsx 不计算公式（保留公式原文）；pdf 只读文本层、不支持 OCR。
//! 编辑语义：XML 转义后的字节级替换（恰好一次）；xlsx 用定点替换保留原结构与样式。
//! 路线：低层 zip + XML 处理，保真度可控。

pub mod docx;
pub mod pdf;
pub mod xlsx;
pub mod xml;
pub mod zipio;

/// 原始文件体积上限：超过直接报错，不做解析（50MB）。
pub const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;
/// 提取文本的输出上限：超过按 UTF-8 字符边界截断并置 `truncated`（2MB）。
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

/// 文档读取结果：`kind` 文档类型（docx/xlsx/pdf）、`text` 提取的正文、
/// `note` 降级提示（缺正文、疑似扫描件等，为空表示正常）、`truncated` 文本是否被截断。
#[derive(Debug, Clone)]
pub struct DocResult {
    pub kind: String,
    pub text: String,
    pub note: String,
    pub truncated: bool,
}

/// 编辑请求：`kind = "text"` 为 docx 文本替换（用 old/new），
/// `kind = "cell"` 为 xlsx 单元格更新（用 sheet/cell/value）。
#[derive(Debug, Clone)]
pub struct DocEdit {
    /// 编辑类型："text"（docx 文本替换）或 "cell"（xlsx 单元格更新）
    pub kind: String,
    /// 替换原文（docx）：必须在文档中唯一命中
    pub old: Option<String>,
    /// 替换后的新文本（docx）
    pub new: Option<String>,
    /// 目标工作表（xlsx）：名称或从 1 起的序号，缺省取第一个
    pub sheet: Option<String>,
    /// 目标单元格引用，如 "B2"（xlsx）
    pub cell: Option<String>,
    /// 写入单元格的值（xlsx）：=开头按公式原文写入，不计算
    pub value: Option<String>,
}

/// 该格式不支持编辑（PDF 只读文本层，不支持编辑）。
#[derive(Debug)]
pub struct ErrReadOnly;

impl std::fmt::Display for ErrReadOnly {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "该格式不支持编辑")
    }
}

impl std::error::Error for ErrReadOnly {}

/// 按扩展名判别文档类型；不识别返回空串。
pub fn doc_kind(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".docx") {
        "docx"
    } else if lower.ends_with(".xlsx") {
        "xlsx"
    } else if lower.ends_with(".pdf") {
        "pdf"
    } else {
        ""
    }
}

/// 扩展名是否属于可处理的文档类型。
pub fn doc_supported(path: &str) -> bool {
    !doc_kind(path).is_empty()
}

/// 读取文档并提取文本：先检查大小上限，再按类型分发到对应模块；错误为可读中文消息。
pub fn read_file(path: &str) -> Result<DocResult, String> {
    let kind = doc_kind(path);
    if kind.is_empty() {
        return Err("不支持的文档格式".into());
    }
    let raw = std::fs::read(path).map_err(|e| format!("读取失败：{e}"))?;
    if raw.len() > MAX_FILE_BYTES {
        return Err("文档过大（超过 50MB 上限）".into());
    }
    match kind {
        "docx" => docx::read(&raw),
        "xlsx" => xlsx::read(&raw),
        "pdf" => pdf::read(&raw),
        _ => Err("不支持的文档格式".into()),
    }
}

/// 应用编辑：仅 docx（text）与 xlsx（cell）支持；PDF 只读，按 ErrReadOnly 拒绝。
pub fn replace(path: &str, edit: &DocEdit) -> Result<(), String> {
    match doc_kind(path) {
        "docx" => docx::replace(path, edit),
        "xlsx" => xlsx::replace(path, edit),
        _ => Err(ErrReadOnly.to_string()),
    }
}
