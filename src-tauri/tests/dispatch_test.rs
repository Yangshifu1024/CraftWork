//! dispatch 接线全量对拍：
//! 渲染层 channels.ts 声明的每个方法都必须在 craftwork_invoke 的 dispatch 有对应分支。
//! 背景：documentPreview / catalogImport / workspaceAddProject 等断线正是「实现存在、
//! 缺 dispatch 臂 / 参数形状错」且被现有测试盲区放过（services_test 直调函数绕过分发）。
//! 故意注释掉任一分支本测试即红——防缺臂复发。

use std::path::Path;

/// 声明存在但渲染层未接（死通道）白名单：登记去向，勿静默新增
/// （死代码清理时决定去留或接线）。
const UNWIRED_ALLOWLIST: [&str; 3] = ["catalogLookup", "workspaceSetActive", "workspaceLocateFile"];

/// 对拍规则：channels.ts 声明的方法（两空格缩进签名解析）必须在 lib.rs 出现同名分支字符串；
/// 白名单通道允许缺席，但一旦接线必须移出白名单（防清单失真）。
#[test]
fn channels_all_methods_have_dispatch_branches() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let channels = std::fs::read_to_string(manifest.join("../src/shared/channels.ts")).expect("读 src/shared/channels.ts");
    let lib = std::fs::read_to_string(manifest.join("src/lib.rs")).expect("读 src-tauri/src/lib.rs");

    // 提取 CraftworkApi 方法名：两空格缩进的 `name(` 行（channels.ts 只有这一个接口，
    // 多行签名的方法名同样在首行）
    let mut declared: Vec<String> = Vec::new();
    for line in channels.lines() {
        let Some(rest) = line.strip_prefix("  ") else { continue };
        if rest.starts_with('/') || rest.starts_with('*') {
            continue;
        }
        let Some((name, _)) = rest.split_once('(') else { continue };
        let mut chars = name.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_lowercase()) || !chars.all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        declared.push(name.to_string());
    }
    assert!(declared.len() >= 60, "channels 方法解析异常（{}/应 ≥60）", declared.len());

    let mut missing: Vec<String> = Vec::new();
    let mut stale_allowlist: Vec<String> = Vec::new();
    for name in &declared {
        let arm = format!("\"{name}\"");
        if UNWIRED_ALLOWLIST.contains(&name.as_str()) {
            // 白名单项一旦接线应从白名单移除，保持清单诚实
            if lib.contains(&arm) {
                stale_allowlist.push(name.clone());
            }
            continue;
        }
        if !lib.contains(&arm) {
            missing.push(name.clone());
        }
    }
    assert!(stale_allowlist.is_empty(), "以下白名单项已接线，请从 UNWIRED_ALLOWLIST 移除：{stale_allowlist:?}");
    assert!(missing.is_empty(), "dispatch 缺分支（缺臂运行时报「尚未接线」）：{missing:?}");
}
