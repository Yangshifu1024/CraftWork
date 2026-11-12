//! 系统能力：对话框、外链、剪贴板、窗口控制。
//! 原则：外链唯一出口（仅 http/https，交系统默认浏览器；渲染层不得自行打开任何远程页面）；
//! 路径相关操作只处理应用自身选择/登记过的路径。
//! 各函数由 lib.rs 的 dispatch 接线（openExternal / pickFiles / clipboard* 等）。
//!
//! 自绘标题栏（D22）：Windows 的窗口控制按钮由渲染层 `WindowControls` 组件提供
//! （`src/renderer/src/components/window-controls.tsx`），按面板状态内联迁移——面板关时
//! 在会话头右端，面板开时在面板条右端。关闭按钮走既有 `CloseRequested`
//! → 「关闭到托盘」语义（D20），与原生关闭键一致。

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
    // 不经 shell 解析：直接交给系统的打开入口（macOS `open` / Windows `cmd /c start`，
    // start 的第一个引号参数是窗口标题占位，URL 必须排在第二位；Linux 实机启用后再接）。
    let result = open_with_platform(url);
    match result {
        Ok(_) => Ok(json!(null)),
        Err(e) => err(format!("打开链接失败：{e}")),
    }
}

#[cfg(target_os = "macos")]
fn open_with_platform(url: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("open").arg(url).spawn()
}

#[cfg(target_os = "windows")]
fn open_with_platform(url: &str) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW：cmd 是纯载体，不能替 URL 弹黑窗（同一常量，见 craft_core::platform）
    std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .creation_flags(craft_core::platform::CREATE_NO_WINDOW)
        .spawn()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_with_platform(url: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("xdg-open").arg(url).spawn()
}

/// 在文件管理器中定位文件（macOS reveal / Windows explorer /select）。
pub fn show_in_folder(path: &str) -> SvcResult {
    if !std::path::Path::new(path).exists() {
        return err("路径不存在");
    }
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let result = {
        // explorer /select,：<path> 逗号后不能有空格；反斜杠路径原样传
        std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn()
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let result = {
        let parent = std::path::Path::new(path).parent().unwrap_or(std::path::Path::new(path));
        std::process::Command::new("xdg-open").arg(parent).spawn()
    };
    match result {
        Ok(_) => Ok(json!(null)),
        Err(e) => err(format!("打开位置失败：{e}")),
    }
}

/// 用系统文件管理器打开目录本身（与 show_in_folder 的「在父目录中定位」语义区分）。
/// 仅供服务端已知路径使用（当前为应用数据目录），不接受渲染层传入的任意路径。
pub fn open_folder(path: &str) -> SvcResult {
    if !std::path::Path::new(path).is_dir() {
        return err("目录不存在");
    }
    // 不经 shell 解析：直接以参数传给文件管理器；各平台命令按 cfg 选择
    #[cfg(target_os = "macos")]
    let command = "open";
    #[cfg(target_os = "windows")]
    let command = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let command = "xdg-open";
    let result = std::process::Command::new(command).arg(path).spawn();
    match result {
        Ok(_) => Ok(json!(null)),
        Err(e) => err(format!("打开目录失败：{e}")),
    }
}

/// 写剪贴板（纯文本）。
pub fn clipboard_write(app: &tauri::AppHandle, text: &str) -> SvcResult {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())?;
    Ok(json!(true))
}

/// 读剪贴板（纯文本；剪贴板为非文本内容时返回错误）。
pub fn clipboard_read(app: &tauri::AppHandle) -> SvcResult {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let text = app.clipboard().read_text().map_err(|e| e.to_string())?;
    Ok(json!(text))
}

/// 最小化主窗口。
pub fn minimise_window(app: &tauri::AppHandle) -> SvcResult {
    let Some(window) = app.get_webview_window("main") else {
        return err("主窗口不存在");
    };
    window.minimize().map_err(|e| e.to_string())?;
    Ok(json!(null))
}

/// 主窗口最大化/还原切换。
///
/// 最小化状态下切换直接恢复显示（unminimize）：最小化的最大化窗口 `is_maximized`
/// 仍为 true，走 unmaximize 会从最小化直接落到还原边界——若此时用户尚无正常态
/// 位置记录，窗口会塌缩（实机踩坑）；恢复显示才是最小化状态下点最大化钮的预期。
pub fn toggle_maximise_window(app: &tauri::AppHandle) -> SvcResult {
    let Some(window) = app.get_webview_window("main") else {
        return err("主窗口不存在");
    };
    if window.is_minimized().map_err(|e| e.to_string())? {
        window.unminimize().map_err(|e| e.to_string())?;
    } else if window.is_maximized().map_err(|e| e.to_string())? {
        window.unmaximize().map_err(|e| e.to_string())?;
    } else {
        window.maximize().map_err(|e| e.to_string())?;
    }
    Ok(json!(null))
}

/// 主窗口是否最大化：自绘标题栏据此在「最大化」与「还原」图标之间切换（决策 D22）。
///
/// 只读查询，不改变窗口状态；各平台行为一致（Windows 自绘标题栏是唯一调用方）。
pub fn window_is_maximised(app: &tauri::AppHandle) -> SvcResult {
    let Some(window) = app.get_webview_window("main") else {
        return err("主窗口不存在");
    };
    Ok(json!(window.is_maximized().map_err(|e| e.to_string())?))
}

/// 关闭主窗口：触发 CloseRequested → lib.rs 按「关闭到托盘」设置决定隐藏还是取消在途运行并退出。
pub fn close_window(app: &tauri::AppHandle) -> SvcResult {
    let Some(window) = app.get_webview_window("main") else {
        return err("主窗口不存在");
    };
    window.close().map_err(|e| e.to_string())?;
    Ok(json!(null))
}

/// 应用角标（未读或待审批的会话数，决策 D20）；count ≤ 0 或 null 表示清除。
///
/// 平台范围：本期只做 macOS 停靠栏数字。Windows 任务栏角标需要覆盖图标（`set_overlay_icon`，
/// 要另外准备 .ico 资源），Linux 依赖桌面环境支持，两者暂不实现——显式返回 no-op，
/// 而不是在非目标平台静默报错，避免设置页/调用方把它当成失败。
pub fn set_badge_count(app: &tauri::AppHandle, count: Option<i64>) -> SvcResult {
    let Some(window) = app.get_webview_window("main") else {
        return err("主窗口不存在");
    };
    // 负数没有意义（调用方可能传 -1 表示未知），统一按清除处理
    let value = count.filter(|n| *n > 0);
    #[cfg(target_os = "macos")]
    {
        window.set_badge_count(value).map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&window, value);
    }
    Ok(json!(null))
}

/// 返回 JSON null（当前无调用方，预留占位）。
pub fn null_value() -> Value {
    json!(null)
}
