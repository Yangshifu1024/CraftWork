# 13: 消息分页接线（滚动加载更早）

**labels:** [needs-triage]
**对应对账:** spec.md §二 L7

**What:** `threadSnapshotPage(threadId, limit, beforeSeq)` 两端齐备（channels.ts:70 / lib.rs:182 / services.rs:253 / events.rs 快照重建支持 limit+beforeSeq 返回 hasMore/oldestSeq），前端零调用。接线：selectThread 首拉 limit=尾部 N 条；滚动触顶经 `threadSnapshotPage` 前拼；`hasMore/oldestSeq` 驱动是否继续。快照返回的 `pendingApprovals` 字段（events.rs:397）同样被前端丢弃，一并消费。

## 进展

