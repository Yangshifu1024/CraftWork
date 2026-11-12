//! dispatch 接线全量对拍：
//! 渲染层 channels.ts 声明的每个方法都必须在 craftwork_invoke 的 dispatch 有对应分支。
//! 背景：documentPreview / catalogImport / workspaceAddProject 等断线正是「实现存在、
//! 缺 dispatch 臂 / 参数形状错」且被现有测试盲区放过（services_test 直调函数绕过分发）。
//! 故意注释掉任一分支本测试即红——防缺臂复发。

use std::path::Path;

/// 声明存在但渲染层未接（死通道）白名单：登记去向，勿静默新增
/// （死代码清理时决定去留或接线）。
const UNWIRED_ALLOWLIST: [&str; 3] = ["catalogLookup", "workspaceSetActive", "workspaceLocateFile"];

/// 从 `channels.ts` 抽出 `CraftworkApi` 接口内声明的方法名。
///
/// 不依赖缩进：早先的实现在这里假定「方法以恰好两空格缩进」，而 `channels.ts` 里
/// `chatBashOutput` / `chatKillShell` / `chatCancelWait` / `workspaceReplaceFolders`
/// 四个方法是顶格写的，于是它们对拍完全隐形——缺臂或参数写错都不会让本测试变红。
/// 现在改为：先框定 `export interface CraftworkApi {` 到其后第一个顶格 `}` 的块，
/// 再取块内每个以 `名字(` 开头的行（含多行签名的首行）。
fn declared_methods(channels: &str) -> Vec<String> {
    let mut declared: Vec<String> = Vec::new();
    let mut inside = false;
    for line in channels.lines() {
        if !inside {
            if line.starts_with("export interface CraftworkApi {") {
                inside = true;
            }
            continue;
        }
        if line.starts_with('}') {
            break;
        }
        let trimmed = line.trim_start();
        // 注释行与块注释体跳过
        if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }
        let Some((name, _)) = trimmed.split_once('(') else { continue };
        let mut chars = name.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_lowercase()) || !chars.all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        declared.push(name.to_string());
    }
    declared
}

/// 从 `lib.rs` 抽取**活着的** dispatch 分支名。
///
/// 早先的实现用 `lib.contains("\"name\"")` 判断，而注释掉整行后该字符串仍留在
/// 文件里（在 `//` 之后），于是「故意注释掉任一分支本测试即红」这句声称从来不成立——
/// 这是实测发现的（注释 `chatCancelWait` 的臂后测试照样绿）。现在逐行判活：
/// 必须出现 `"name" =>`（允许 `=>` 跨到下一行），且该行本身不是注释。
fn live_dispatch_arms(lib: &str) -> Vec<String> {
    let mut arms: Vec<String> = Vec::new();
    for raw in lib.lines() {
        let line = raw.trim_start();
        if line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") {
            continue;
        }
        let Some(start) = line.find('"') else { continue };
        let rest = &line[start + 1..];
        let Some(end) = rest.find('"') else { continue };
        let name = &rest[..end];
        let after = rest[end + 1..].trim_start();
        // 分支形态："name" => 或 "name"（换行后的）=> —— 两种都算活
        if after.starts_with("=>") || after.is_empty() {
            let mut chars = name.chars();
            if chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_alphanumeric()) {
                arms.push(name.to_string());
            }
        }
    }
    arms
}

/// 对拍规则：channels.ts 声明的方法必须在 lib.rs 有**活着的**同名分支；
/// 白名单通道允许缺席，但一旦接线必须移出白名单（防清单失真）。
#[test]
fn channels_all_methods_have_dispatch_branches() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let channels = std::fs::read_to_string(manifest.join("../src/shared/channels.ts")).expect("读 src/shared/channels.ts");
    let lib = std::fs::read_to_string(manifest.join("src/lib.rs")).expect("读 src-tauri/src/lib.rs");

    let declared = declared_methods(&channels);
    // 下界守卫：解析规则退化（比如块识别失败）时必须立刻红，而不是静默「0 个方法全通过」。
    assert!(
        declared.len() >= 117,
        "channels 方法解析异常（{}/应 ≥117）；解析规则可能已失效：{declared:?}",
        declared.len()
    );

    let arms = live_dispatch_arms(&lib);
    let mut missing: Vec<String> = Vec::new();
    let mut stale_allowlist: Vec<String> = Vec::new();
    for name in &declared {
        let wired = arms.iter().any(|a| a == name);
        if UNWIRED_ALLOWLIST.contains(&name.as_str()) {
            // 白名单项一旦接线应从白名单移除，保持清单诚实
            if wired {
                stale_allowlist.push(name.clone());
            }
            continue;
        }
        if !wired {
            missing.push(name.clone());
        }
    }
    assert!(stale_allowlist.is_empty(), "以下白名单项已接线，请从 UNWIRED_ALLOWLIST 移除：{stale_allowlist:?}");
    assert!(missing.is_empty(), "dispatch 缺分支（缺臂运行时报「尚未接线」）：{missing:?}");
}

/// 回归守卫：早先的缩进式解析漏掉了 4 个顶格方法。
/// 这里逐个钉住它们，防止解析规则退回缩进假设。
#[test]
fn top_level_signatures_are_parsed() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let channels = std::fs::read_to_string(manifest.join("../src/shared/channels.ts")).expect("读 src/shared/channels.ts");
    let declared = declared_methods(&channels);
    for name in ["chatBashOutput", "chatKillShell", "chatCancelWait", "workspaceReplaceFolders"] {
        assert!(declared.iter().any(|m| m == name), "{name} 未被解析到（channels.ts 里该签名是顶格的）");
    }
}
