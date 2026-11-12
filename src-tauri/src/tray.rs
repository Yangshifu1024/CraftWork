//! 系统托盘：后台常驻入口 + 轻状态（决策 D20）。
//!
//! 职责边界：
//! - 本模块只做窗口显隐、托盘菜单、状态文本与图标切换；不复制任何会话 / 审批逻辑，
//!   状态计数从 `Runner` 与 `approvals` 表只读推导。
//! - 托盘菜单只在启动时构建一次；状态文本与动态项用 `set_text` 原地更新。
//! - 关闭窗口的最终决策（隐藏还是退出）由 `should_hide_on_close` 给出，真正的拦截在
//!   lib.rs 的 `CloseRequested` 分支里完成。
//!
//! 平台差异：
//! - Windows / Linux：普通图标；待审批 > 0 时切带红点的告警图标。
//! - macOS：始终 template 图标（系统单色着色），不显示托盘红点；待审批由 Dock 角标、
//!   菜单状态与系统通知表达。
//!
//! # 口径与边界（决策 D20）
//!
//! - **状态计数只认主会话**：运行中 = 有未结束 run 的会话（等待审批仍算运行中，子代理
//!   run 归父会话），待审批 = `approvals.decision IS NULL` 的去重 `thread_id`。
//!   **后台命令任务不计入**——它们是 run 内的工具调用产物，不是独立的「正在忙」来源；
//!   计进来只会让托盘数字在 run 空闲时也常亮。
//! - **最小化不进托盘**：最小化保持窗口已有的系统行为，只有「关闭」才按隐藏处理。
//! - **降级**：托盘创建或图标解码失败时不启用「关闭到托盘」（关闭按退出处理），应用照常
//!   启动；此路径不需要额外分支。
//! - **已知待办**：`tauri-plugin-single-instance` 的初始化失败目前没有降级分支，失败即
//!   单实例保护失效（应用仍能启动）。若将来要补，入口在 lib.rs 的插件注册处。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use craft_agent::session_log::LEVEL_ERROR;
use craft_core::contract::Settings;
use craft_store::records::get_setting;

use crate::services::AppState;

const TRAY_ID: &str = "main";
const STATUS_ID: &str = "tray:status";
const TOGGLE_ID: &str = "tray:toggle";
const QUIT_ID: &str = "tray:quit";

#[cfg(not(target_os = "macos"))]
const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");
#[cfg(not(target_os = "macos"))]
const TRAY_ALERT_ICON: &[u8] = include_bytes!("../icons/tray-alert.png");
#[cfg(target_os = "macos")]
const TRAY_TEMPLATE_ICON: &[u8] = include_bytes!("../icons/tray-template.png");

/// 托盘在 Tauri 管理状态里的句柄：菜单项、两套图标与几个进程内标志。
///
/// 菜单只在启动时构建一次；之后只改文本和图标，不重建菜单（Tauri 2.12 的菜单项没有
/// `set_visible`，重建会丢焦点并引入闪烁）。
pub struct TrayState {
    status_item: MenuItem<Wry>,
    toggle_item: MenuItem<Wry>,
    quit_item: MenuItem<Wry>,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    normal_icon: Image<'static>,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    alert_icon: Image<'static>,
    /// 当前托盘图标是否已是告警态（Windows / Linux 切图标用；macOS 恒 false）。
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    alert_shown: AtomicBool,
    /// 动态项当前是否显示「隐藏主窗口」（true）还是「显示主窗口」（false）。
    /// 用原子量去重：窗口拖动 / 缩放会连续触发 Resized，不比较就每帧 set_text。
    toggle_shows_hide: AtomicBool,
    /// 本次进程是否已经提示过「关闭到托盘」；只在第一次隐藏时发通知。
    first_hide_notified: AtomicBool,
    /// 托盘是否成功创建；false 时关闭窗口必须走旧语义（避免隐藏后无入口）。
    available: AtomicBool,
}

pub type TraySignal = Arc<Mutex<Sender<()>>>;

/// 向托盘状态线程投递一次刷新信号（Bus 的 `thread.updated` 批次调用）。
pub fn signal(tx: &TraySignal) {
    if let Ok(tx) = tx.lock() {
        let _ = tx.send(());
    }
}

/// 构建托盘图标与菜单，并把 `TrayState` 注册进 Tauri 管理状态。
///
/// 失败由调用方记录日志并继续启动：托盘不可用时关闭窗口按旧语义退出。
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let (normal_icon, alert_icon) = load_icons()?;
    let status_item = MenuItem::with_id(app, STATUS_ID, "空闲", false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let initial_toggle_label = toggle_label(app);
    let toggle_item = MenuItem::with_id(app, TOGGLE_ID, &initial_toggle_label, true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, QUIT_ID, "退出 CraftWork", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status_item, &separator, &toggle_item, &quit_item])?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(normal_icon.clone())
        .icon_as_template(cfg!(target_os = "macos"))
        .menu(&menu)
        // 两端一致：左键切换显隐，右键出菜单。macOS 不沿用「左键弹菜单」的系统惯例。
        .show_menu_on_left_click(false)
        .tooltip("CraftWork")
        .on_menu_event(|app, event| {
            if event.id() == TOGGLE_ID {
                toggle_main_window(app);
            } else if event.id() == QUIT_ID {
                app.exit(0);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(TrayState {
        status_item,
        toggle_item,
        quit_item,
        normal_icon,
        alert_icon,
        alert_shown: AtomicBool::new(false),
        toggle_shows_hide: AtomicBool::new(initial_toggle_label == "隐藏主窗口"),
        first_hide_notified: AtomicBool::new(false),
        available: AtomicBool::new(true),
    });
    Ok(())
}

/// 托盘状态刷新线程：接收 Bus 的 `thread.updated` 信号，合并 100ms 内的多次触发后重查一次。
///
/// 不轮询、不逐 token 刷新；流式文本事件不进入这里（Bus 回调只对 `thread.updated` 投递）。
pub fn start_worker(app: AppHandle, rx: Receiver<()>) {
    std::thread::Builder::new()
        .name("craftwork-tray-status".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                std::thread::sleep(Duration::from_millis(100));
                while rx.try_recv().is_ok() {}
                refresh(&app);
            }
        })
        .ok();
}

/// 重查运行中 / 待审批会话数，更新菜单文本、tooltip 与 Windows 告警图标。
pub fn refresh(app: &AppHandle) {
    let Some(tray_state) = app.try_state::<TrayState>() else {
        return;
    };
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };
    let (running, pending) = match counts(state.inner()) {
        Ok(counts) => counts,
        Err(err) => {
            // 保留上一次文本；错误只进日志，不把技术细节塞进菜单。
            state.session_log.write_app(LEVEL_ERROR, "tray_status_failed", json!({ "error": err }));
            return;
        }
    };

    let status = format_status(running, pending);
    let _ = tray_state.status_item.set_text(&status);
    let _ = tray_state.quit_item.set_text(format_quit_label(running, pending));
    sync_toggle_label(app);

    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let tooltip = if status == "空闲" {
        "CraftWork".to_string()
    } else {
        format!("CraftWork · {status}")
    };
    let _ = tray.set_tooltip(Some(tooltip));

    // Windows / Linux：待审批 > 0 时切告警图标；macOS 始终 template，不参与切换。
    #[cfg(not(target_os = "macos"))]
    {
        let want_alert = pending > 0;
        if tray_state.alert_shown.swap(want_alert, Ordering::SeqCst) != want_alert {
            let icon = if want_alert {
                tray_state.alert_icon.clone()
            } else {
                tray_state.normal_icon.clone()
            };
            let _ = tray.set_icon(Some(icon));
        }
    }
}

/// 关闭窗口时是否应该隐藏到托盘：
/// 托盘可用且用户没有开启「关闭窗口时退出应用」。
pub fn should_hide_on_close(app: &AppHandle) -> bool {
    let available = app
        .try_state::<TrayState>()
        .map(|state| state.available.load(Ordering::SeqCst))
        .unwrap_or(false);
    available && !quit_on_close(app)
}

/// 窗口已经隐藏后调用：第一次隐藏发一次系统通知，并同步菜单里的显示 / 隐藏标签。
pub fn on_window_hidden(app: &AppHandle) {
    if let Some(state) = app.try_state::<TrayState>() {
        let first = !state.first_hide_notified.swap(true, Ordering::SeqCst);
        if first && notifications_enabled(app) {
            use tauri_plugin_notification::NotificationExt;
            let _ = app
                .notification()
                .builder()
                .title("CraftWork")
                .body("仍在后台运行，点击托盘图标可恢复")
                .show();
        }
    }
    sync_toggle_label(app);
}

/// 显示并聚焦主窗口：托盘、Dock Reopen、单实例唤起三处共用，顺序固定。
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    sync_toggle_label(app);
}

/// 左键点击托盘：窗口可见且未最小化 → 隐藏；否则显示、还原并聚焦。
pub fn toggle_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    if visible && !minimized {
        let _ = window.hide();
        sync_toggle_label(app);
    } else {
        show_main_window(app);
    }
}

/// 只刷新动态菜单项的文案（窗口显隐 / 最小化后会变化）。
pub fn sync_toggle_label(app: &AppHandle) {
    let Some(state) = app.try_state::<TrayState>() else {
        return;
    };
    let want_hide = window_shows_hide(app);
    if state.toggle_shows_hide.swap(want_hide, Ordering::SeqCst) != want_hide {
        let _ = state.toggle_item.set_text(if want_hide {
            "隐藏主窗口"
        } else {
            "显示主窗口"
        });
    }
}

/// 主窗口当前是否「可见且未最小化」——是则托盘菜单显示「隐藏主窗口」。
fn window_shows_hide(app: &AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    visible && !minimized
}

fn toggle_label(app: &AppHandle) -> String {
    if window_shows_hide(app) {
        "隐藏主窗口".into()
    } else {
        "显示主窗口".into()
    }
}

/// 运行中 / 待审批计数：按主会话去重。
///
/// 运行中来自 Runner 的取消令牌集合（包含等待审批的 run 与后台子代理 run），子代理
/// 通过 `parent_thread_id` 归到父会话；待审批直接查 `approvals.decision IS NULL` 的
/// distinct thread_id（子代理审批的 thread_id 已是父会话，见 D15）。
fn counts(state: &AppState) -> Result<(usize, usize), String> {
    let mut running = HashSet::new();
    for thread_id in state.runner.running_threads() {
        let Ok(row) = state.db.with(|conn| craft_store::threads::get_thread(conn, &thread_id)) else {
            continue;
        };
        let parent = if row.parent_thread_id.is_empty() {
            row.id
        } else {
            row.parent_thread_id
        };
        running.insert(parent);
    }
    let pending = state
        .db
        .with(craft_store::runs::pending_approval_threads)
        .map_err(|e| e.to_string())?;
    Ok((running.len(), pending.len()))
}

fn format_status(running: usize, pending: usize) -> String {
    match (running, pending) {
        (0, 0) => "空闲".into(),
        (running, 0) => format!("{running} 个运行中"),
        (0, pending) => format!("{pending} 个待审批"),
        (running, pending) => format!("{running} 个运行中 · {pending} 个待审批"),
    }
}

fn format_quit_label(running: usize, pending: usize) -> String {
    match (running, pending) {
        (0, 0) => "退出 CraftWork".into(),
        (running, 0) => format!("退出 CraftWork（{running} 个运行中）"),
        (0, pending) => format!("退出 CraftWork（{pending} 个待审批）"),
        (running, pending) => format!("退出 CraftWork（{running} 个运行中 · {pending} 个待审批）"),
    }
}

fn load_icons() -> tauri::Result<(Image<'static>, Image<'static>)> {
    #[cfg(target_os = "macos")]
    {
        let icon = Image::from_bytes(TRAY_TEMPLATE_ICON)?;
        Ok((icon.clone(), icon))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok((
            Image::from_bytes(TRAY_ICON)?,
            Image::from_bytes(TRAY_ALERT_ICON)?,
        ))
    }
}

fn quit_on_close(app: &AppHandle) -> bool {
    settings_snapshot(app).map(|settings| settings.quit_on_close).unwrap_or(false)
}

fn notifications_enabled(app: &AppHandle) -> bool {
    settings_snapshot(app).map(|settings| settings.notifications).unwrap_or(true)
}

fn settings_snapshot(app: &AppHandle) -> Option<Settings> {
    let state = app.try_state::<Arc<AppState>>()?;
    state
        .db
        .with(|conn| get_setting(conn, "app"))
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_value::<Settings>(value).ok())
}

#[cfg(test)]
mod tests {
    use super::{format_quit_label, format_status};

    #[test]
    fn status_text_is_idle_when_no_activity() {
        assert_eq!(format_status(0, 0), "空闲");
    }

    #[test]
    fn status_text_combines_running_and_pending() {
        assert_eq!(format_status(2, 1), "2 个运行中 · 1 个待审批");
        assert_eq!(format_status(3, 0), "3 个运行中");
        assert_eq!(format_status(0, 4), "4 个待审批");
    }

    #[test]
    fn quit_label_carries_activity_counts() {
        assert_eq!(format_quit_label(0, 0), "退出 CraftWork");
        assert_eq!(format_quit_label(2, 0), "退出 CraftWork（2 个运行中）");
        assert_eq!(format_quit_label(0, 1), "退出 CraftWork（1 个待审批）");
        assert_eq!(
            format_quit_label(2, 1),
            "退出 CraftWork（2 个运行中 · 1 个待审批）"
        );
    }
}
