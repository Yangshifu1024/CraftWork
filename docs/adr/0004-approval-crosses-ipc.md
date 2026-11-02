# ADR-0004：审批必须跨 IPC，禁止 Go 侧静默放行

- 状态：Accepted
- 日期：2026-10

## 背景

原型的审批是进程内阻塞调用（`Approver.Ask` 直接等待用户输入）。桌面应用里 UI 在 WebView 渲染进程，Go 的阻塞等待等不到任何用户输入；而任何「Go 侧自行判断后放行」的实现都会让审批形同虚设。

## 决策

- 审批走 **`approval.requested` 事件 + `ChatService.DecideApproval` 命令**的跨 IPC 往返：Agent 挂起 → 前端审批卡片 → 用户决定 → 命令回传 → 状态机解析 → run 恢复。
- 审批模块内部维护唯一状态机（`waiting → approved / denied / canceled`）、等待表、授权缓存与卡片载荷；服务层只做 IPC 参数校验。
- 授权范围 `once / session / workspace` 由**服务端**校验；`AllowPermanent` 为 false 时禁止 session / workspace。
- Shell 命令按前缀复用授权（取前两个片段，第二段是选项时只取第一段）；含管道、重定向、串联或 `sudo` / `bash -c` 包装的命令只允许精确匹配一次。
- 取消、窗口关闭、应用退出统一按拒绝或取消落审计。
- 审批决定与工具执行都写 `audit_log`。

## 后果

- 没有任何代码路径能在无用户决定的情况下执行 R1 及以上操作（自由档位除外，见 ADR-0003）。
- 审批挂起期间 run 与连接都保持；崩溃恢复时挂起审批按 `canceled` 处理。
- 前端只是决定的转发者；重复决定、越权范围、非法状态迁移都被服务端拒绝。
