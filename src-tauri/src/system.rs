//! 系统能力：对话框、外链、剪贴板、窗口控制。
//! 原则：外链唯一出口（仅 http/https，交系统默认浏览器）；
//! 路径相关操作只处理应用自身选择/登记过的路径。

use serde_json::{json, Value};
use tauri::Manager;

use crate::services::SvcResult;

fn err<T>(msg: impl Into<String>) -> Result<T, String> {
    Err(msg.into())
}

/// 选择文件夹（单选）。
pub fn pick_folder(app: &tauri::AppHandle) -> SvcResult {
    let Some(window) = app.get_webview_window("main") else {
        return err("主窗口不存在");
    };
    let picked = tauri_plugin_dialog::DialogExt::dialog(&window)
        .file()
        .blocking_pick_folder();
    match picked {
        Some(path) => {
            let path_str = path.to_string();
            Ok(json!(path_str))
        }
        None => Ok(json!("")),
    }
}

/// 选择文件（多选）。
pub fn pick_files(app: &tauri::AppHandle) -> SvcResult {
    let Some(window) = app.get_webview_window("main") else {
        return err("主窗口不存在");
    };
    let picked = tauri_plugin_dialog::DialogExt::dialog(&window).file().blocking_pick_files();
    let paths: Vec<String> = picked
        .map(|list| list.iter().map(|p| p.to_string()).collect())
        .unwrap_or_default();
    Ok(json!(paths))
}

/// 外链唯一出口：仅 http/https，交系统默认浏览器。
pub fn open_external(url: &str) -> SvcResult {
    let ok = url.starts_with("http://") || url.starts_with("https://");
    if !ok {
        return err("仅允许打开 http/https 链接");
    }
    // 不经 shell 解析：直接 open（macOS）；Windows/Linux 在对应平台实机启用
    let result = std::process::Command::new("open").arg(url).spawn();
    match result {
        Ok(_) => Ok(json!(null)),
        Err(e) => err(format!("打开链接失败：{e}")),
    }
}

/// 在访达中显示（macOS reveal）。
pub fn show_in_folder(path: &str) -> SvcResult {
    if !std::path::Path::new(path).exists() {
        return err("路径不存在");
    }
    let result = std::process::Command::new("open").arg("-R").arg(path).spawn();
    match result {
        Ok(_) => Ok(json!(null)),
        Err(e) => err(format!("打开位置失败：{e}")),
    }
}

pub fn clipboard_write(app: &tauri::AppHandle, text: &str) -> SvcResult {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())?;
    Ok(json!(true))
}

pub fn clipboard_read(app: &tauri::AppHandle) -> SvcResult {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let text = app.clipboard().read_text().map_err(|e| e.to_string())?;
    Ok(json!(text))
}

pub fn minimise_window(app: &tauri::AppHandle) -> SvcResult {
    let Some(window) = app.get_webview_window("main") else {
        return err("主窗口不存在");
    };
    window.minimize().map_err(|e| e.to_string())?;
    Ok(json!(null))
}

pub fn toggle_maximise_window(app: &tauri::AppHandle) -> SvcResult {
    let Some(window) = app.get_webview_window("main") else {
        return err("主窗口不存在");
    };
    if window.is_maximized().map_err(|e| e.to_string())? {
        window.unmaximize().map_err(|e| e.to_string())?;
    } else {
        window.maximize().map_err(|e| e.to_string())?;
    }
    Ok(json!(null))
}

pub fn close_window(app: &tauri::AppHandle) -> SvcResult {
    let Some(window) = app.get_webview_window("main") else {
        return err("主窗口不存在");
    };
    window.close().map_err(|e| e.to_string())?;
    Ok(json!(null))
}

pub fn null_value() -> Value {
    json!(null)
}
