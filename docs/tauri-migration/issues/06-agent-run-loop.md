# 06 Run 循环与 Agent 服务（P5）

labels: [needs-triage]
status: pending
depends: 03、04、05

## 任务

1. Run 循环：发送消息 → 模型流式请求（SSE）→ 解析文本/推理/工具调用 Part → 审批（经 05 状态机）→ 执行工具 → 结果回填 → 续跑直至完成/失败/取消；同一会话单 run 约束；steer（运行中引导）排队并入下一批。循环结构参考 codex-rs（`codex-rs` 的 protocol / state / rollout 拆分与《Unrolling the Codex agent loop》），只参考设计不复制代码。
2. 历史重建（事件表 → 请求消息序列）、系统提示词组装（含 EnvContext：日期/平台/Shell+Git 摘要/AGENTS.md 两级，任一步失败降级省略）、附件加载、自动命名——逐模块直译。
3. 上下文压缩：自研实现（决策 D2），设计参考 codex-rs 的 memories/context 管理思路；压缩触发阈值与现有行为对齐。
4. 事件流出口：低频状态 emit + 高频 token/工具输出走 `tauri::ipc::Channel`（Rust 侧 50~100ms 攒批，测有序性与延迟体感）。
5. 取消语义：窗口关闭/应用退出/用户取消统一终止 run 并按取消落库。
6. agent/title/context 相关 vitest 用例翻译。

## 验收

- 端到端闭环（隔离目录）：添加提供商 → 发消息 → 流式渲染 → 工具调用 → 审批允许/拒绝 → 结果落库 → 重启后快照重建一致；
- Channel 高频推送无乱序、体感延迟可接受；
- 自由档位切换的风险确认、受限工作区拦截在端到端走查中复现。

## 进展
