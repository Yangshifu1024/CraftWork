# 05. 聊天主链路：消息流、流式 Markdown 与 Composer

Status: ready-for-agent

## 背景

核心价值链路（[spec.md](../spec.md) P1）。对齐 `features/chat/` 的消息流与 composer，流式渲染直接用 `markdown.Stream`。

## 改动

- `internal/ui/chat/reducer.go`：移植 `threadReducer.ts`（23 种事件 handler：流式增量、部件 upsert、审批队列、steer、用量累加、终态回填），纯函数 + 单测对齐既有用例。
- 消息流：`ui.List`（`FollowEnd` + 平滑跟底）、80px 阈值回到底部、80 条分页 + seq 游标去重、空态建议句、用户右对齐气泡/助手全宽/系统细条、消息元信息与悬停操作（复制/重新生成）。
- Markdown：`markdown.Stream` + `Reveal`；代码块高亮 + 语言标签 + 复制按钮；`path:line` 引用替换为内部链接（移植 `markdownRef.ts` 预处理），点击打开文件定位行。
- Composer：`ui.TextArea`（2→10 行自适应）、Enter 发送/Shift+Enter 换行（`OnTextKey`）、运行中引导 steer 形态、停止按钮（双击 Esc armed）、附件条可移除。
- 附件：文件 chip + 图片缩略图（后端 data URL 读取）、拖放（`DroppedFiles`）、粘贴图片（`OnPaste` + `Clipboard.ReadImage` → Go 落盘）、12MB 与视觉模型拦截提示。

## 验收

- reducer 单测全绿（对齐 threadReducer 既有用例语义）。
- 真实后端跑通：发送 → 流式回复（中文长文含代码块/表格）→ 停止 → steer → 附件 → 重新生成。
- 链接安全：外链确认框走 `Shell.OpenExternal`，HTML/远程图按文本与占位渲染。

## 依赖

02、04（会话切换）。
