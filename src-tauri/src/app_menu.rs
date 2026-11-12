//! macOS 应用菜单：在 Tauri 默认菜单上替换「关于」项。
//!
//! 为什么需要这个模块：macOS 上不设 `.menu(...)` 时，Tauri 会自动装 `Menu::default`，
//! 其中 App 子菜单的「关于」是 `PredefinedMenuItem::about`——点击由 muda 直接调
//! `orderFrontStandardAboutPanel` 弹原生面板，**不产生任何菜单事件**，无法拦截。要让
//! 它接到应用内的设置页「关于」分节，只能换成普通 `MenuItem` 自己处理点击。
//!
//! 做法是「打补丁」而不是重写整份菜单：`Menu::default` 的 File / Edit / View / Window /
//! Help 与 `WINDOW_SUBMENU_ID`、`HELP_SUBMENU_ID` 的注册、Services 空菜单的装配都保持
//! 原样（这些是 macOS 系统行为，重写等于重新踩一遍坑），只把 App 子菜单首项换掉。
//!
//! 补丁带守卫：首项不是预定义项（Tauri 改了默认布局）时放弃替换并保留原生 About，
//! 不抛错、不阻塞启动——功能退化而不是启动失败。
//!
//! 渲染层经 `craftwork:menu` 推送通道接收动作（channels.ts 的 PushChannels::Menu），
//! 由 App.tsx 打开设置页对应分节；本模块不碰 IPC invoke，只发事件。

use serde_json::json;
use tauri::menu::{Menu, MenuEvent, MenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Wry};

/// 「关于」菜单项 id。菜单事件靠它识别（渲染层不看文本，文本可随时改）。
pub const ABOUT_MENU_ID: &str = "app:about";

/// 构建应用菜单：Tauri 默认菜单 + App 子菜单首项替换为自建「关于」项。
///
/// 失败时返回 Err 交给 Tauri（启动期菜单构不出属于装配错误，不应静默）；
/// 布局守卫命中的情况不算错误，返回原样菜单。
pub fn build(app: &AppHandle<Wry>) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::default(app)?;
    // macOS 上首个顶层子菜单即 App 菜单（以应用名命名），见 tauri 的 Menu::default。
    let Some(app_submenu) = first_submenu(&menu) else {
        return Ok(menu);
    };

    let items = app_submenu.items()?;
    if !matches!(items.first(), Some(tauri::menu::MenuItemKind::Predefined(_))) {
        // 布局变了：不乱插乱删，保留系统原生 About（退化为旧行为）。
        eprintln!("craftwork: 应用菜单布局与预期不符，跳过多余的「关于」替换");
        return Ok(menu);
    }
    app_submenu.remove_at(0)?;

    let about = MenuItem::with_id(app, ABOUT_MENU_ID, "关于 CraftWork", true, None::<&str>)?;
    app_submenu.insert(&about, 0)?;
    Ok(menu)
}

/// 菜单事件入口：只认「关于」项，其余（剪切复制、最小化、退出等预定义项）交给系统。
pub fn handle_event(app: &AppHandle<Wry>, event: MenuEvent) {
    if event.id() != ABOUT_MENU_ID {
        return;
    }
    // 窗口可能被最小化或藏在其他应用后面：先亮出来再开浮层，否则用户点了菜单看不到变化。
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    // 广播失败只影响这一次入口，记一行日志即可（与 lib.rs 事件出口同规格）。
    if let Err(err) = app.emit(
        "craftwork:menu",
        json!({ "action": "settings", "section": "about" }),
    ) {
        eprintln!("craftwork: 菜单动作广播失败（about）: {err}");
    }
}

/// 取首个顶层子菜单（macOS 的 App 菜单）；非 Submenu 的顶层项按缺失处理。
fn first_submenu(menu: &Menu<Wry>) -> Option<Submenu<Wry>> {
    menu.items().ok()?.into_iter().find_map(|item| match item {
        tauri::menu::MenuItemKind::Submenu(sub) => Some(sub),
        _ => None,
    })
}

