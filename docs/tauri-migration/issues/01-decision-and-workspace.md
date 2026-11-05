# 01 决策定案与 workspace 脚手架（P0）

labels: [needs-triage]
status: done
depends: 无（本 feature 起点）

## 任务

1. 审阅 `docs/tauri-migration/plan.md`（含已定决策 D1~D3），写 `docs/adr/0002-adopt-tauri2.md`：采用 Tauri 2、移除终端功能、Agent 侧完全自研（参考 codex-rs）、crate 命名无前缀（后被 D3 实测推翻，改 craft- 前缀，见 ADR 0002）；记录放弃项（Node 生态共享、差量更新、Windows 7）。
2. 建 cargo workspace：`src-tauri/`（应用壳占位）+ `crates/craft-{core,store,policy,model,tools,agent,doc}` 空壳（命名依据见 plan D3 实测记录），各 crate 依赖关系按 plan §1.2 声明，`crates/*` 一律不依赖 Tauri（用 CI 或 clippy lint 约束）。
3. 接入 Rust 工具链基线：rust-toolchain.toml 锁 ≥1.90；`cargo check` / `cargo test` / `cargo clippy` 进现有验证流程（package.json scripts 加 Rust 侧入口）。
4. CI 预留：现有三平台矩阵加 Rust 构建步骤（先 macOS 必绿，其余允许失败占位）。

## 验收

- `cargo check && cargo test && cargo clippy` 全绿；
- `pnpm typecheck && pnpm test && pnpm build` 不受影响；
- ADR 0002 合入。

## 进展
