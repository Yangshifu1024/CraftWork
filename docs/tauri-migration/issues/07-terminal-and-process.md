# 07 进程执行：shell_exec 完整执行体（P6）

labels: [needs-triage]
status: done
depends: 05（可与 06、08 并行）

注：交互式终端功能已随决策 D1 移除（issue 02），本 issue 只覆盖 Agent 的命令执行工具 `shell_exec`。

## 任务

1. `shell_exec` 完整执行体：tokio::process + `process_group(0)`（Unix 进程组）、环境白名单清洗（`env_clear` + 逐项注入，与现有清洗语义一致）、超时两段式（TERM → KILL）杀进程组、独立进程组隔离、输出截断。
2. Windows 路线预留：command-group（Job Object 树杀）；环境白名单必须保留 `SystemRoot` / `PATH`（ConPTY/Windows 进程启动必需）。
3. 补齐 05 中挂起的执行体接线：Registry `execute` → shell_exec 执行体 → 输出流经 Channel 推送 → 审计落库。
4. 现有 shell_exec 行为语义单测：环境清洗、超时、截断、审计、取消。

## 验收

- 超时后进程树（含孙进程）被杀净、无僵尸进程；
- 环境清洗后仅含白名单变量；
- 现有 shell_exec 语义测试在 Rust 侧全部通过。

## 进展
