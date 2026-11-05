# 9: 防复发——dispatch 接线全量对拍与契约金样

**labels:** [ready-for-agent]
**对应对账:** spec.md §二 L9；背景：D2~D4 三条断线全部落在现有测试盲区（services_test 直调函数绕过 dispatch）

**What to build:**

1. `src-tauri/tests/dispatch_test.rs`：读 `src/shared/channels.ts` 正则提取 CraftworkApi 全部方法名，逐个断言 `src-tauri/src/lib.rs` 源码含对应 `"methodName"` match 臂（白名单豁免：确属渲染层未接的声明需在测试内显式列出并注释去向，防止默默漂移）。
2. 契约金样最小恢复：`crates/craft-core/src/contract.rs` 序列化形状测试从 Envelope/Batch 扩到全部对外类型（ThreadView/Snapshot/MessageSnapshot/Settings/WorkspaceView/UsageSummary/Backup/AuditEntry/ApprovalCard/DiffResult…），锁 camelCase 与关键字段名，对齐 `src/shared/types.ts`。

## 验收标准

- [ ] 故意注释掉任一 dispatch 臂 → 测试红；恢复 → 绿
- [ ] 契约测试覆盖全部 `#[derive(Serialize)]` 对外结构；人为改驼峰字段名 → 测试红
- [ ] `pnpm test:rust`、`pnpm lint:rust` 通过

## 进展
- 2026-11-05：完成（提交 96cc41b）。dispatch_test.rs：channels 全方法 → dispatch 臂对拍（白名单三项显式登记）；contract 金样：全部对外结构递归断言无 snake_case；补 WorkspaceFolder rename_all
