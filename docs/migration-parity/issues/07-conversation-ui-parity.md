# 7: 对话区高优 UI 恢复——附件可见、图片预览、粘贴上传、已工作秒数、Esc 链、steer 标记

**labels:** [ready-for-agent]
**对应对账:** spec.md §二 L4/L10/L12、§一 D16/D17

**What to build:**

1. **附件可见**：用户消息 `payload.attachments` 渲染——图片附件经 `readAttachment`（通道已通，lib.rs:253）取 base64 渲染缩略图（懒加载、失败降级文件 chip）；非图片渲染文件名 chip。消息行与时间线两处（composer 附件条、气泡内）。
2. **图片放大浮层**：新建 `overlays/image-preview.tsx`——fixed 全屏 z-[60] 遮罩 + 居中原图 + 文件名 + 点遮罩/Esc 关闭；遵循 prefers-reduced-motion。
3. **粘贴上传**：composer textarea `onPaste` → 新建 `lib/paste.ts` `collectPastedFiles`（clipboardData.files → {name, mime, base64}，总量 12MB 上限、图片 MIME→扩展名、逻辑纯净无 DOM 依赖）→ 既有 `savePastedAttachment` → 追加到草稿附件。粘贴图片在附件条出缩略图。
4. **已工作秒数实时**：`store/app.ts:311-323` run 终态分支把事件 `durationMs` 并入对应消息 `workDurationMs`（快照路径已好，事件流补齐）。
5. **全局 Esc 退出链**：`hooks/shortcuts.ts` Esc 分层——任何浮层/对话框打开时让位（radix 自管，维持现状 return）→ picker 开着关 picker → 会话运行中：第一次 Esc 置 armed（800ms）并联动停止按钮文案「再按一次确认停止」，第二次 Esc 触发停止；修正停止按钮 title 的「停止（Esc）」误导（改为如实描述双击语义）。
6. **steer 待送达标记**：后端 `services.rs chat_steer` 容量预检通过后广播 user.message 带 `via:"steer"`；前端 `app.ts` user.message 分支按 via 置待送达标记，`run.started` 时清除；`conversation.tsx` 用户消息旁渲染「待送达」小徽标。

## 验收标准

- [ ] 发送带图片消息：气泡内见缩略图，点击放大、Esc 关闭；非图片显示文件 chip
- [ ] 粘贴截图进 composer：出缩略图，发送成功且入模型（vision 模型）
- [ ] 运行完成后折叠块立即显示「持续 X 秒」（无需重启）
- [ ] 运行中 Esc 两击停止；浮层打开时 Esc 归浮层；停止按钮文案与实际一致
- [ ] 运行中发消息立即带「待送达」，回复开始即消失
- [ ] `pnpm typecheck` 通过；深浅主题走查

## 进展

