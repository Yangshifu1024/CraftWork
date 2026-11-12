// 构建脚本：Tauri 资源生成 + 构建期元数据注入。
//
// 注入两项「关于」页需要的构建期事实（编译后不可变，故用 rustc-env 固化为常量）：
//   CRAFTWORK_GIT_HASH   —— `git rev-parse --short HEAD`，工作区有未提交改动时附 `-dirty`；
//                           取不到（源码 tarball／未装 git）时为 `unknown`，不影响构建。
//   CRAFTWORK_BUILD_TIME —— 构建时刻（本地时区，秒级）。
//
// 为什么放在构建脚本而不是运行时执行 git：运行时探测要依赖用户机器上的 git 与仓库目录
//（打包后的 .app 里二者都不存在），只有构建期取值才能真实反映「这份二进制来自哪次提交」。
use std::process::Command;

fn main() {
    // 元数据的重跑条件：自身、Git 引用文件。目录不存在时 Cargo 每次都会重跑（无副作用，
    // 只是构建脚本本身耗时极短）。
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs");
    println!("cargo:rerun-if-changed=../.git/packed-refs");

    println!("cargo:rustc-env=CRAFTWORK_GIT_HASH={}", git_hash().unwrap_or_else(|| "unknown".into()));
    println!(
        "cargo:rustc-env=CRAFTWORK_BUILD_TIME={}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );

    tauri_build::build()
}

/// Git 短哈希；工作区有未提交改动时追加 `-dirty`。git 不可用／非仓库返回 None。
fn git_hash() -> Option<String> {
    let hash = git(&["rev-parse", "--short", "HEAD"])?;
    // status 失败按「干净」处理：宁可少标 dirty，也不要把哈希变成 unknown 整段丢失
    let dirty = git(&["status", "--porcelain"]).is_some_and(|out| !out.is_empty());
    Some(if dirty { format!("{hash}-dirty") } else { hash })
}

/// 执行 git 子命令并返回裁剪后的标准输出；非零退出返回 None。
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}
