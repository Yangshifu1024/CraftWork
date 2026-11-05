# 1: 「创建并首发」主进程原子路径与通道接线

**labels:** [ready-for-agent]
**status:** done
**Blocked by:** None（可立即开始）

**What to build:** 对话服务新增「创建并首发」方法：一次调用完成模型校验（未配置或不可用时在创建之前抛出可读中文错误，保证零落库）→ 创建 Thread（标题取首条消息短标题；取到短标题时视为正式命名、自动命名标记置 false，LLM 命名自然跳过；取不到回落「新对话」且自动命名标记置 true，走既有 LLM 命名）→ 经 Recorder 写用户消息事件 → 启动 Run，返回 threadId 与 runId。只在创建前失败时抛错；创建成功后 Run 启动失败视为发送成功（不抛错，错误经 run.failed 事件展示）。通道表 / preload / IPC 三处同步接线；既有普通发送、运行中引导、重新生成的语义不变。

## 验收标准

- [ ] 服务层测试（真实 SQLite 临时目录 + 桩 Runner）：成功时线程落库——标题为首条消息短标题、用户消息事件存在、Run 启动
- [ ] 服务层测试：短标题直用时自动命名标记为 false；回落「新对话」时为 true
- [ ] 服务层测试：模型未配置 / 不可用时抛错，数据库无线程残留
- [ ] 服务层测试：工作区不存在时抛错且零落库
- [ ] 渲染层可经通道表调用该方法，三处接线签名一致
- [ ] 既有发送 / 引导 / 重新生成行为不变（既有测试全绿）

## 进展

已实现（feature/draft-session @ `2f7739f`）。`ChatService.createAndStart(mode, workspaceId, text, attachments, modelRef)`：模型校验前置（空引用抛「尚未选择模型」、不可解析抛「当前模型不可用」）→ 工作区校验 → 建 Thread（shortTitle 直用置 titleAuto=false，取不到回落「新对话」置 true）→ Recorder 写用户消息事件 + emit 广播 → 启动 Run；创建后 Run 失败不抛错返回 `runId: ""`。通道表/preload/ipc 三处接线 `chatCreateAndStart`；ChatService 构造依赖零新增（modelRef 由渲染层传所见引用）。`tests/chat-start.test.ts` 8 用例覆盖全部验收（含空消息、Run 启动失败、纯附件回落）。既有测试无回归；typecheck / test / build / dist:dir 全绿（6195ade 后复验）。
