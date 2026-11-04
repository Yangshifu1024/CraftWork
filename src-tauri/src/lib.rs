//! CraftWork 应用壳（Tauri 2）。
//!
//! 分层纪律：本 crate 只做入口、窗口与 IPC 接线；全部领域逻辑在 crates/craft-*（不依赖 Tauri）。
//! P1 阶段壳内尚无领域后端，`craftwork_invoke` 对所有调用返回「尚未迁移」错误——
//! 渲染层界面可完整呈现；P2 起按阶段把领域服务接进 invoke 分发（见 docs/tauri-migration/plan.md）。

use serde_json::{json, Value};

/// 渲染层统一入口：所有 `window.craftwork.<method>(...)` 经此分发。
/// P1 仅支撑渲染层 boot 的最小数据面（默认设置 + 空列表），其余方法返回「尚未迁移」；
/// P2 起替换为按方法名分发的领域调用（与 Electron 版 ipc.ts 的 handle 表等价）。
#[tauri::command]
fn craftwork_invoke(method: String, args: Vec<Value>) -> Result<Value, String> {
    let _ = &args;
    match method.as_str() {
        "settingsGet" => Ok(json!({
            "theme": "system", "mode": "code", "panelWidth": 320,
            "approvalLevel": "graded", "defaultModel": "", "fallbackModel": "",
            "activeWorkspace": "", "language": "zh-CN", "fontSize": 15,
            "codeTheme": "dark", "autoUpdate": true, "notifications": true,
            "updateChannel": "stable", "telemetry": false, "maxIterations": 300
        })),
        "threadList" => Ok(json!([])),
        "workspaceList" => Ok(json!([])),
        _ => Err(format!("方法 {method} 尚未在 Rust 侧实现（迁移进行中）")),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![craftwork_invoke])
        .run(tauri::generate_context!())
        .expect("craftwork 启动失败");
}
