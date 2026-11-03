# 05. 聊天主链路：消息流、流式 Markdown 与 Composer

Status: resolved

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

## Comments

- 2026-11-03 完成于 wt/native-05，四个实现提交 + 一个合并提交（08 号票面板合入后槽位共存）。`go test ./...`、`go vet`、`golangci-lint run` 0 issues、`go tool mygo build` 产出 .app+DMG。
- **reducer 对齐**：`internal/ui/chat/reducer.go` 逐行移植 threadReducer.ts 的 23 种事件 handler；单测 14 例对齐 TS 测试全部 10 个用例语义（增量拼接、审批队列去重、用量累加与 sidecar 水位、跨会话刷新标记、steer 去重与三路清除、未知事件忽略），另补终态工时回填、部件 upsert 位置保持、整条会话合并、纯函数不可变 4 例 Go 侧用例。与 React 版的结构差异：threads 双模式列表改为按 Id 的 map（侧栏列表归 04 号票，这里的表只服务聊天列自身），`*Message` 指针共享 + 写时复制，Part.Data 统一归一化为 map（同进程事件是强类型结构体、快照是 JSON map，两种形状都要吃）。
- **实现者留档（对兄弟票关键）**：①markdown.Stream/View 的元素在收缩布局（Row 里不带 FillWidth 的 Column）里会塌成零宽，助手正文列必须显式 `FillWidth`；②`ui.Tester` 里 markdown.Stream 的词级 reveal 按引擎时间推进，harness/单测要 `tt.Advance` 让 reveal 追上，否则截图/断言全空（真窗口走墙钟无此问题）；③普通发送后端不再广播 user.message，界面必须乐观插入（steer 仍以 user.message 事件为唯一来源，不要双插）。
- **冒烟**：真实链路（store → runner → OpenAI 兼容适配器 → SSE → reducer → view）跑在 ui.Tester 里，模型指向 /tmp 假端点（35ms 分段 SSE，中文长文含表格与 go 代码块）；截图 8 张在 /tmp/cw-05-shots：空态建议句、流式中（生成中标签 + 光标 + 停止按钮）、steer 气泡、完成态（表格/代码块高亮/Copy/path:line 链接）、停止（incomplete 保留半截）、附件 chip、深浅色。消息序列断言 reply → steer → followup → canceled 全对。
- **未尽事项**：真窗口的剪贴板复制 / 对话框拾取 / 外链确认只过了 Tester 与代码走查，未做实机 GUI 验证（osascript 无辅助权限，与 03 号票同样限制）；粘贴图片走 OnPaste 消费 + goroutine 读剪贴板（官方文档示例是帧内直调，按 04 号票的 onMain 坑规避），实机待 07 号票走查时覆盖。附件图片缩略图（后端 data URL 读取）未做，只画文件 chip，归 06 号票附件卡一起补。
