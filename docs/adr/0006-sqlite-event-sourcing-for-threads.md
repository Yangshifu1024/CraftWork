# ADR-0006：SQLite 事件溯源——events 表是对话事实源

- 状态：Accepted
- 日期：2026-10

## 背景

对话状态（消息、工具调用、审批、错误）需要崩溃恢复、跨端渲染一致与可追溯。选择有二：以「当前消息表」为事实源（更新式），或以「事件流」为事实源（溯源式）。

## 决策

- **`events` 表是对话事实源**：每条事件按线程 `seq` 单调递增，写入在事务内分配序号；会话历史、消息部件、审批状态都从事件表重建。
- 流式增量只推送不逐条落库；文本完成时写入完整部件。
- 事件写入与广播统一走 `internal/record.Recorder`：一次调用完成 seq 分配、落库与推送；不允许手写 `AppendEvent` + `Bus.Publish` 配对。
- run 生命周期状态以 `runs` 表为准，不额外写 run 起止事件。
- SQLite（modernc.org/sqlite 纯 Go）+ WAL，`busy_timeout=5000`；迁移用 `PRAGMA user_version` 逐版执行，迁移失败拒绝启动。
- 应用表（threads / workspaces / approvals / audit_log / usage / settings / backups）只存索引、审计、配置与用量。
- 压缩摘要以 `context_summary` 事件持久化（携带覆盖截止序号），重建历史时复用，同一批早期消息只摘要一次。

## 后果

- 崩溃后重启即可完整恢复会话；`running` 的 run 标记 `interrupted`，挂起审批标记 `canceled`。
- 快照（`lastSeq` + 全量部件）是前端的唯一初始状态来源，切换会话不依赖事件重放。
- 审计天然完整：事件不可变、只追加。
- 存储体积随事件增长，靠软删除 7 天、大文本外置与每日备份控制。
