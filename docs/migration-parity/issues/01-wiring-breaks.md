# 1: 接线断修复——documentPreview / catalogImport / providerSave / 返回类型 / workspace.updated

**labels:** [ready-for-agent]
**对应对账:** spec.md §一 D2~D6

**What to build:** 修复五处接线断（运行时必错或地雷）：

1. `src-tauri/src/lib.rs` dispatch 补 `"documentPreview"` 分支（实现在 `services.rs:430`）；`panel.tsx:166` 调用处补 catch + danger toast（原 unhandled rejection 静默）。
2. `catalogImport`：`lib.rs:241` 改为取 `value_at(0)` 再解对象字段 providerId/all/modelIds，对齐前端 `channels.ts:114` 与 `provider-add.tsx:180` 的单对象实参。
3. `providerSave` 空 ID：`services.rs:623` 空串时服务端生成 `custom_<新id>`（对齐 `provider-add.tsx:239` 注释语义），并补 services 测试。
4. 通道返回类型声明对齐实现（消除断言地雷，`channels.ts`）：provider 系 8 方法（Save/Delete/SetEnabled/Rename/SetModelEnabled/AddModel/UpdateModel/RemoveModel）与 providerSetKey/ClearKey、workspaceAdd/workspaceRename 改为数组或如实类型；chatSteer 改为 `{ queued: boolean; runId?: string }`。
5. `workspace_remove`（`services.rs:331`）经 recorder 发射 `workspace.updated { removed: true, workspaceId }`，激活前端清草稿槽分支（`app.ts:277-292` 死代码转活）。

## 验收标准

- [ ] 面板点击文本文件出预览，点击不支持类型出可读 toast；不再有未捕获拒绝
- [ ] 设置页「从目录导入」导入模型成功
- [ ] 手动新增提供商成功且 ID 形如 `custom_*`；重名/空名有可读错误
- [ ] 删除项目后重启，localStorage `craftwork-drafts` 中无该工作区残留槽
- [ ] `pnpm typecheck`、`pnpm test:rust`、`pnpm lint:rust` 通过

## 进展
- 2026-11-05：完成（提交 16857b4）。面板点击文件出预览（dispatch 分支已补，失败出 toast）；目录导入/手动新增提供商后端语义修正（services 测试覆盖）；返回类型对齐编译验证；workspace.updated 已广播；GUI 走查（删除项目后草稿槽清理）待实机
