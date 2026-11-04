//! 文档读写：Word（docx）/ Excel（xlsx）/ PDF 的文本级处理。
//! 限制：docx 只读正文；xlsx 不计算公式（保留公式原文）；pdf 只读文本层、不支持 OCR。
//! 编辑语义：XML 转义后的字节级替换（恰好一次）；xlsx 用定点替换保留原结构与样式。
//! 路线：与 Electron 版一致的低层 zip + XML 处理（直译 src/main/doc/index.ts，保真度可控）。
//!
//! 对应 Electron 主进程 `src/main/doc/`。

pub mod docx;
pub mod pdf;
pub mod xlsx;
pub mod xml;
pub mod zipio;

pub const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct DocResult {
    pub kind: String,
    pub text: String,
    pub note: String,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct DocEdit {
    /// text | cell
    pub kind: String,
    pub old: Option<String>,
    pub new: Option<String>,
    pub sheet: Option<String>,
    pub cell: Option<String>,
    pub value: Option<String>,
}

/// 该格式不支持编辑。
#[derive(Debug)]
pub struct ErrReadOnly;

impl std::fmt::Display for ErrReadOnly {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "该格式不支持编辑")
    }
}

impl std::error::Error for ErrReadOnly {}

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

pub fn doc_supported(path: &str) -> bool {
    !doc_kind(path).is_empty()
}

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

pub fn replace(path: &str, edit: &DocEdit) -> Result<(), String> {
    match doc_kind(path) {
        "docx" => docx::replace(path, edit),
        "xlsx" => xlsx::replace(path, edit),
        _ => Err(ErrReadOnly.to_string()),
    }
}
