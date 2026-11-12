//! doc_read / doc_edit / xlsx_set_cell 内置工具（经 craft-doc，低层 zip + XML 文本级处理）。
//!
//! Work 模式限制（当前留桩）：doc_edit 只支持 Word 同一格式片段内的文本替换；
//! xlsx_set_cell 只写单元格、不计算公式；doc_read 只读文本层（无 OCR、不可编辑 PDF）。
//! 写类工具与 fs_* 一致：写前备份，替换失败时文件保持原样。

use serde_json::json;

use craft_policy::PolicyRisk;

use crate::registry::{Tool, ToolCtx, ToolResult, ToolSpec};

fn err_result(message: impl Into<String>) -> ToolResult {
    ToolResult::err(message)
}

/// 读取 Word（docx）/ Excel（xlsx）/ PDF 的文本内容（R0 只读，文本级，无 OCR）。
pub struct DocRead;

impl Tool for DocRead {
    fn spec(&self) -> ToolSpec {
        DocRead::spec_value()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        if !craft_doc::doc_supported(&ctx.target) {
            return err_result(format!("不支持的文档格式：{}", ctx.display));
        }
        match craft_doc::read_file(&ctx.target) {
            Ok(doc) => {
                let note = if doc.note.is_empty() { String::new() } else { format!("说明：{}\n\n", doc.note) };
                let text = if doc.text.is_empty() { "文档没有可提取的文本".to_string() } else { doc.text };
                ToolResult::ok_with_meta(
                    format!("{note}{text}"),
                    json!({ "path": ctx.target, "kind": doc.kind, "truncated": doc.truncated }),
                )
            }
            Err(err) => err_result(err),
        }
    }
}

impl DocRead {
    pub fn spec_value() -> ToolSpec {
        ToolSpec {
            name: "doc_read".into(),
            description: "读取 Word（docx）/ Excel（xlsx）/ PDF 的文本内容（文本级，不支持 OCR）。".into(),
            schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "文档路径（相对工作区）" } },
                "required": ["path"],
            }),
            risk: PolicyRisk::R0,
            path_arg: Some("path".into()),
            command_arg: None,
            mutates: false,
            work: true,
            title: "读取文档".into(),
            summary_arg: Some("path".into()),
            summary_text: None,
        }
    }
}

/// 替换 Word / Excel 文档中的一处文本（R1 写工作区）。
/// 替换语义由 craft-doc 决定：Word 限同一格式片段内匹配，跨片段不命中会报错。
pub struct DocEdit;

impl Tool for DocEdit {
    fn spec(&self) -> ToolSpec {
        DocEdit::spec_value()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let old_text = ctx.str_arg("old_text");
        if old_text.is_empty() {
            return err_result("old_text 不能为空");
        }
        if !craft_doc::doc_supported(&ctx.target) {
            return err_result(format!("不支持的文档格式：{}", ctx.display));
        }
        let original = match std::fs::read(&ctx.target) {
            Ok(raw) => raw,
            Err(err) => return err_result(format!("读取失败：{err}")),
        };
        // 写前备份（替换失败时文件保持原样）
        if let Err(err) = ctx.env.backup(&ctx.target, &original) {
            return err_result(err);
        }
        let edit = craft_doc::DocEdit {
            kind: "text".into(),
            old: Some(old_text),
            new: Some(ctx.str_arg("new_text")),
            sheet: None,
            cell: None,
            value: None,
        };
        if let Err(err) = craft_doc::replace(&ctx.target, &edit) {
            return err_result(err);
        }
        ToolResult::ok_with_meta(format!("已替换 {} 中的 1 处文本。", ctx.display), json!({ "path": ctx.target }))
    }
}

impl DocEdit {
    pub fn spec_value() -> ToolSpec {
        ToolSpec {
            name: "doc_edit".into(),
            description: "替换 Word / Excel 文档中的一处文本（保留原有结构；写入前自动备份）。".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "文档路径（相对工作区）" },
                    "old_text": { "type": "string", "description": "要替换的原文" },
                    "new_text": { "type": "string", "description": "替换后的文本" },
                },
                "required": ["path", "old_text", "new_text"],
            }),
            risk: PolicyRisk::R1,
            path_arg: Some("path".into()),
            command_arg: None,
            mutates: true,
            work: true,
            title: "修改文档".into(),
            summary_arg: Some("path".into()),
            summary_text: None,
        }
    }
}

/// 更新 Excel 单元格（R1 写工作区）；只写值不计算公式——应用内不重算，由 Excel 打开时求值。
pub struct XlsxSetCell;

impl Tool for XlsxSetCell {
    fn spec(&self) -> ToolSpec {
        XlsxSetCell::spec_value()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let cell = ctx.str_arg("cell").trim().to_string();
        if cell.is_empty() {
            return err_result("cell 不能为空");
        }
        if !craft_doc::doc_supported(&ctx.target) {
            return err_result(format!("不支持的文档格式：{}", ctx.display));
        }
        let original = match std::fs::read(&ctx.target) {
            Ok(raw) => raw,
            Err(err) => return err_result(format!("读取失败：{err}")),
        };
        // 写前备份
        if let Err(err) = ctx.env.backup(&ctx.target, &original) {
            return err_result(err);
        }
        let edit = craft_doc::DocEdit {
            kind: "cell".into(),
            old: None,
            new: None,
            sheet: Some(ctx.str_arg("sheet")),
            cell: Some(cell.clone()),
            value: Some(ctx.str_arg("value")),
        };
        if let Err(err) = craft_doc::replace(&ctx.target, &edit) {
            return err_result(err);
        }
        ToolResult::ok_with_meta(
            format!("已更新 {} 的单元格 {}。", ctx.display, cell.to_uppercase()),
            json!({ "path": ctx.target }),
        )
    }
}

impl XlsxSetCell {
    pub fn spec_value() -> ToolSpec {
        ToolSpec {
            name: "xlsx_set_cell".into(),
            description: "更新 Excel 单元格（如 B8）。value 以 = 开头写入公式（不计算）；数字写为数值；其余写为文本。".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "xlsx 路径（相对工作区）" },
                    "sheet": { "type": "string", "description": "工作表名称或从 1 起的序号，默认第一个" },
                    "cell": { "type": "string", "description": "单元格引用，如 B8" },
                    "value": { "type": "string", "description": "要写入的值" },
                },
                "required": ["path", "cell", "value"],
            }),
            risk: PolicyRisk::R1,
            path_arg: Some("path".into()),
            command_arg: None,
            mutates: true,
            work: true,
            title: "修改表格".into(),
            summary_arg: Some("path".into()),
            summary_text: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolEnv;
    use serde_json::Value;

    fn make_ctx(args: Value, target: String, display: String, roots: Vec<String>) -> (std::path::PathBuf, ToolCtx) {
        let dir = crate::test_util::unique_temp_dir("doctool");
        std::fs::create_dir_all(&dir).unwrap();
        let db = std::sync::Arc::new(craft_store::db::Db::open(&dir.join("t.db")).unwrap());
        let ctx = ToolCtx {
            name: "doc".into(),
            args,
            target,
            display,
            workspace: roots.first().cloned().unwrap_or_default(),
            roots,
            target_root: String::new(),
            err: None,
            plan_active: false,
            thread_id: String::new(),
            env: std::sync::Arc::new(ToolEnv { backup_dir: dir.join("backups").to_string_lossy().to_string(), data_dir: String::new(), db, shell_tasks: std::sync::Arc::new(crate::shell_tasks::ShellTasks::new()), proxy_config: crate::registry::direct_proxy_config() }),
        };
        (dir, ctx)
    }

    #[test]
    fn xlsx_cell_tool_update_and_backup() {
        let sheet_xml = r#"<?xml version="1.0"?><worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#;
        let buf = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(buf);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("xl/workbook.xml", options).unwrap();
        std::io::Write::write_all(&mut writer, br#"<workbook><sheets><sheet name="S1" r:id="rId1"/></sheets></workbook>"#).unwrap();
        writer.start_file("xl/_rels/workbook.xml.rels", options).unwrap();
        std::io::Write::write_all(&mut writer, br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#).unwrap();
        writer.start_file("xl/worksheets/sheet1.xml", options).unwrap();
        std::io::Write::write_all(&mut writer, sheet_xml.as_bytes()).unwrap();
        let xlsx_bytes = writer.finish().unwrap().into_inner();

        let dir = crate::test_util::unique_temp_dir("doctool-xlsx");
        std::fs::create_dir_all(&dir).unwrap();
        let xlsx_path = dir.join("book.xlsx");
        std::fs::write(&xlsx_path, &xlsx_bytes).unwrap();
        let target = xlsx_path.to_string_lossy().to_string();

        let (tool_dir, ctx) = make_ctx(json!({"cell": "b2", "value": "新值"}), target.clone(), "book.xlsx".into(), vec![dir.to_string_lossy().to_string()]);
        let result = XlsxSetCell.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("B2"));

        // 备份已登记（backups 目录有文件）
        let files: Vec<_> = std::fs::read_dir(tool_dir.join("backups")).unwrap().flatten().collect();
        assert!(!files.is_empty(), "备份目录应有备份文件");
        std::fs::remove_dir_all(&dir).ok();
    }

    // zip 依赖供测试使用（匿名导入确保链接且不报 unused）
    use zip as _;
}
