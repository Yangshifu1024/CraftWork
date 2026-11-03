# 08. 面板：多 tab 体系、文件树与编辑器

Status: resolved

## 背景

对齐 `features/panel/` + `lib/sidePane.ts`（[spec.md](../spec.md) P2；编辑器按裁剪清单用 `ui/editor`）。

## 改动

- `internal/ui/panel/tabs.go`：移植 `sidePane.ts`（结构化幂等 id、singleton、工作区分桶、归属过滤、最近关闭栈 8、file tab 上限 8、中键关闭、右键菜单、末 tab 收起、宽 156→60 收缩），纯函数 + 单测对齐既有 318 行用例。
- tab 条 UI：5 类 tab、激活态、关闭按钮、`Transition` 收缩动画。
- 文件树：懒加载、过滤平铺（≤200）、多源分段 + 「主要」徽标、扩展名着色图标、Git 状态字母、点击开 tab。
- 编辑器：`ui/editor` 接入——打开/保存（备份 + `expectedHash` 冲突确认）、只读切换、按 tab 隔离草稿、`path:line` 跳转居中、面包屑与状态条（裁剪清单见 spec）。

## 验收

- tabs 状态机单测全绿（对齐 sidePane.test.ts 用例）。
- 文件树打开 → 编辑 → 保存 → 备份出现在数据目录；外部修改后保存触发冲突确认。
- 审批卡「查看完整差异」/聊天 `path:line` 能打开正确 tab 并定位行。

## 依赖

03；保存链路依赖 02。

## 完成记录（2026-11-03）

- `internal/ui/panel/tabs.go`：sidePane.ts 逐行移植（结构化幂等 id、singleton 字面量 id、工作区分桶字段与可见性过滤、盖戳不可覆盖、最近关闭栈 8、file tab 上限 8、关 active 激活邻居、prune/reorder/上限淘汰）；TS 的 null（未认领/无激活项）映射为空串。`tabs_test.go` 27 个用例逐条对齐 sidePane.test.ts。
- 编排层与 view 落在 `internal/ui/panelview.go`（工作区记忆桶 LRU 50、激活项按「工作区::线程」记忆、终端 singleton 全局开关、末 tab 收起经 Collapse 回调）+ `panel_files.go`（懒加载、过滤平铺 ≤200、多源分段+「主要」徽标、扩展名着色、Git 字母、行右键菜单：打开/复制路径/文件管理器中显示/移到废纸篓）+ `panel_editor.go`（ui/editor 按 tab 数据槽、只读切换、保存走 WriteFile 备份 + expectedHash 冲突确认、OpenFileAt path:line 居中跳转、面包屑与 Ln/Col 状态条）+ `panel_docs.go`（Word/Excel/PDF 文本级预览）。视图层互相引用（content 分派、行点击入口）无法拆出可编译的中间提交，故状态机独立成提交、视图层为一个可构建提交。
- tab 条：5 类 tab、激活态图标用模式强调色、关闭按钮、中键关闭（HandleInput button==2）、右键菜单（关闭/其他/全部 + 最近关闭重开按上下文过滤）、宽度 156→60 均分收缩（ElementTransition Size/Colors）。
- shell.go 面板槽位换成面板 view；app.go 注入 workspace/document 服务并 BindWindow。Palette 增加 Dark 标记（目录图标深浅分档）。
- 顺带修复 `service.Add` 不落 workspace_folders 记录导致每次 GetWorkspace 合成新 folder ID 的问题（定位后重读会报「源文件夹不存在」）。
- 验收：`go test ./...` 20 包全绿（tabs 27 用例 + 编排 8 用例）、`go vet`、`golangci-lint run` 0 issues、`go tool mygo build` 产出 .app+DMG。GUI 冒烟（CRAFTWORK_HOME=/tmp/cw-08-smoke，临时 X/Y+AlwaysOnTop+脚本驱动补丁已还原）：文件树开两个文件→编辑→保存（备份落在数据目录 backups/、回读刷新回只读）→外部改文件再保存触发「文件已被外部修改」确认框（重新加载/保留草稿，安全侧默认），深浅色各截图确认。环境无辅助访问权限，无法合成真实鼠标点击（与 03 号票一致），交互经面板正式入口（OpenFileAt/数据槽/saveSlot）编程驱动 + ui.Tester 用例覆盖。
- 验收项「审批卡查看完整差异/聊天 path:line 打开正确 tab」：入口 `Pane.OpenFileAt(path, line)` 已就绪（短路径经 LocateFile 唯一匹配），05/06 号票接线时直接调用。
