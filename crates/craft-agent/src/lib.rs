//! Agent：Run 循环、审批状态机、授权缓存、历史重建、上下文压缩、自动命名、
//! 系统提示词组装、附件加载、环境上下文。
//!
//! 对应 Electron 主进程 `src/main/agent/`，自 P5（docs/tauri-migration/issues/06）起实现。
//! 循环结构参考 openai/codex 的 protocol/state/rollout 拆分（仅参考设计，不复制代码）。
