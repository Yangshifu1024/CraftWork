# 3: 审批闭环修复——resolved 落库、僵尸卡、取消审计、cancels 回收、自动命名

**labels:** [ready-for-agent]
**对应对账:** spec.md §一 D11~D15

**What to build:**

1. **resolved/canceled 落库**：`crates/craft-agent/src/approvals.rs:195-206` 由仅 `recorder.emit` 改为落库+广播（append 事件 kind 使快照回载 `craft-store/src/events.rs:245-264` 能重建终态），重启后已决定审批卡不再回显「等待决定」。
2. **启动清理补事件**：`craft-store/src/runs.rs interrupt_stale_runs` 取消等待中审批时补写审批 resolved(canceled) 事件。
3. **决策失败反馈**：`conversation.tsx:393-399` ApprovalCard 决策失败从 console.warn 改 pushToast。
4. **审计合规（AGENTS 6.2）**：`runner.rs:773-776` 审批取消路径补 add_audit；`approvals.decide` 写决定审计行（allow/deny/scope）；`lib.rs` 窗口关闭/应用退出挂钩——取消在途 run 并按取消落审计。
5. **cancels 回收**：`runner.rs:250-252` run 收尾总是移除 cancels 条目（不只 cancelled），修复 steer 回退分支不可达与 chat_cancel 误报。
6. **自动命名写回**：消除 `services.rs:500-515`（title=derive_title 却 title_auto=true）与 `craft-store/src/threads.rs:151-159`（写回要求 title='新对话'）的冲突——threadRename 置 title_auto=0，写回条件收敛为 title_auto=1；对照旧 `internal/agent/title.go` 语义，模型命名恢复生效且不再每轮空发 sidecar 调用。

## 验收标准

- [ ] 审批允许后重启：卡片显示「已允许（范围）」而非回显等待
- [ ] 运行中强退应用再启动：中断 run 报可重试错误、其等待审批卡显示「已取消」，且均有审计行
- [ ] 决策失败（对已处理审批再点）出 toast 而非静默
- [ ] 首轮完成后再次运行中发消息：steer 正常入队（回归 D14）；运行完成边界发消息能触发新 run（回退可达）
- [ ] 发长首消息：模型命名最终覆盖占位标题；手动重命名的会话不被覆盖；自动命名 sidecar 只发一次
- [ ] `pnpm test:rust`、`pnpm lint:rust` 通过（含新增审批落库/审计/命名测试）

## 进展
- 2026-11-05：完成（提交 92eb85c）。resolved/canceled 落库+广播（approval_resolved 部件事件映射），快照回填测试通过；interrupt_stale_runs 补审批取消终态事件（测试覆盖）；审计三缺口补齐（取消路径/decide/退出挂钩 cancel_all_runs）；cancels 回收（仅移除仍归属本 run 的令牌）；auto_title 写回条件修复并翻转 title_auto（测试覆盖）；GUI 走查（重启卡片状态、Esc/退出审计）待实机
