// Windows release 构建下隐藏随应用启动的额外控制台窗口，勿删！
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 二进制入口：全部装配在 `craftwork_lib::run`（逻辑放 lib 目标，供集成测试直接复用）。
fn main() {
    craftwork_lib::run()
}
