//! 文档读写：docx / xlsx / pdf 文本级处理。
//!
//! 对应 Electron 主进程 `src/main/doc/`，自 P7（docs/tauri-migration/issues/08）起实现。
//! 路线：docx 走 zip + quick-xml 直译；xlsx 先试 umya-spreadsheet（保真度不达标退低层）；
//! pdf 用 pdf-extract 只读文本层。
