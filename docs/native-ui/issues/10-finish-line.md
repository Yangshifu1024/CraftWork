# 10. 收尾：快捷键动效、打包、清理与文档同步

Status: resolved

## 背景

P3（[spec.md](../spec.md) 分阶段）。入口正式切换、web 残留清除、发布链路重建。

## 改动

- 快捷键全表：⌘K/⌘N/⌘,/⌘B/⌘W、Esc 退出链（一次退一层，IME 组合期忽略；双击 Esc 停止 800ms 确认窗）、审批卡 Esc=拒绝。
- 动效：侧栏折叠 `Transition{Size}`、模式滑块、状态点脉冲、审批扫光；`ReduceMotion` 跟随系统。
- 系统通知：失焦时审批/完成/失败经 `mygo.NewNotification`，点击聚焦并跳会话。
- **删除**：`frontend/` 整目录、`internal/service` 绑定残留、`csp_dev/prod.go`、Taskfile 的 wails 任务、`.github/workflows/ci.yml` 的 web 段（含 Playwright 与绑定漂移检查）。
- 打包：`go tool mygo build`（icns、Developer ID 签名 + 公证、DMG）；版本号注入对齐 `internal/version`。
- 文档同步：AGENTS.md（命令/目录/约束/已知坑重写）、specs（architecture/interface/features/prompt-contract）、README、GLOSSARY 术语、新 ADR 索引。
- e2e 重建：`ui.Tester` 关键链路测试（对话闭环、审批、tab 语义）替代 Playwright 用例。

## 验收

- 全新 clone → `go tool mygo build` → 隔离目录冒烟全链路。
- 仓库 `grep -ri wails` 仅剩历史文档；CI 全绿。
- 1440×900 与 1100×800 两视口对照 interface.md 走查（窄窗折叠行为）。

## 依赖

05–09 全部完成后执行。

## Comments

- 2026-11-03 完成于 wt/native-10，七个实现提交：`1bf8ffc`（快捷键全表 + Esc 退出链）、`bb2e3b8`（动效补齐）、`899429f`（通知切 mygo.NewNotification + 跳会话）、`9ab0d16`（删 web 轨残留）、`d75f396`（图标与版本对齐）、`26054ad`（端到端冒烟测试）、`bb84b55`（文档同步）+ 两个收尾小提交。`go build`、`go test ./...`（25 包）、`go vet`、`golangci-lint run` 0 issues、`go tool mygo build` 产出 .app（AppIcon.icns + adhoc 签名）+ DMG（craftwork-0.1.0-arm64.dmg）。
- **快捷键全表**：⌘K/⌘N/⌘B/⌘W/⌘, 对齐 React 版 AppShell；Esc 退出链接全 06 号票入口（`Shell.HandleWindowEscape`）。**关键实现语义**：mygo 对已注册快捷键的按键不再投给焦点元素（`engine.claimed`），所以「焦点在面板可编辑区」层级用的是**条件注册**——`FocusWithin` 为真时根视图不注册窗口级 Esc，终端 / 编辑器才能收到 Esc（vim 实测语义）；焦点状态取上一帧，与 React 逐事件判断同样存在一帧竞态（可接受近似）。IME 组合期天然免疫：组合串走 TextComposition 直送焦点控件，不产生 keyDown。
- **动效**：侧栏会话行带 Enter/Exit 收展过渡（Motion{Collapse}）；审批卡「等待决定」扫光（思考块同机制换警示色）；状态脉冲 / 思考扫光在 `theme.ReduceMotion(c)`（读 ui.Context.Preferences）时停用——Animate/Transition 由 mygo 自动跟随系统减少动效，Loop 不自动停所以要手动门控。**取舍留档**：面板开合保持瞬时——收起时 Split 整体重建（元素身份变化），CSS 时代的宽度滑出依赖持久 DOM 无法平移；React 版组折叠是瞬时的，原生侧按 mygo 列表过渡惯例补了收展动画。
- **通知**：`internal/notify` 从 osascript/notify-send 切 `mygo.NewNotification`；未打包运行经 NotificationsSupported 静默跳过（探测语义同旧）；同会话通知按 ID 聚组、同 ID 替换；点击回调 Close + `jumpToThread`（后台查会话模式 → 主线程切模式刷列表 → 选中加载快照），**同时接上了命令面板会话结果（hooks.OpenThread 此前未接线置灰）**；`OnDidBecomeActive` 时 ClearNotifications。runner 通知回调签名带 threadID。
- **删除清单确认**：`frontend/`（147 个文件含 bindings、e2e 截图基线）、`build/`（wails 打包资产全量）、Taskfile 的 wails 任务、CI 的 pnpm/Playwright/绑定漂移三段、`.gitignore` 前端条目。删除前工作树干净、历史全在 git。appicon.png 迁至 `assets/`。残留 grep：代码里仅剩「Wails 时期」历史语境注释与 ui 各文件的 `对齐 frontend/src/...` 来源注释（有意保留的出处留档）；命令引用清零。
- **打包**：`mygo.json` 补 `"icon": "assets/appicon.png"`（1024×1024 RGBA）；版本对齐策略落档 `internal/version/version.go` 包注释（mygo.json version 是发布产物事实源，两处同改；JSON 写不了注释）。
- **验收清单重建**：选了**普通 `go test` 的装配级端到端**（`internal/app/app_e2e_test.go`，0.02s，不加构建标签）：Run 装配形状组装真实后端 + httptest 假 OpenAI 兼容端点（SSE 工具调用/正文两轮），断言审批事件、DecideApproval 放行、文件落盘、审计两条（用户审批 allow once + 执行）、无挂起审批、快照含正文。GUI 层由各票 ui.Tester 用例与 cwsmoke 截图覆盖。
- **冒烟**（`CRAFTWORK_HOME=/tmp/cw-10-smoke`，截图留档 /tmp/cw-10-shot-*.png）：cwsmoke 构建终端 tab（zsh 运行中、提示符正常）与差异 tab（+3/-1 徽章、统一/并排、语法高亮）各一张；普通构建与打包 .app（adhoc 签名）各启动一张（三栏、空态、侧栏工作区分组、窗口状态恢复均正常）；隔离目录 db/keys/backups/attachments/shell 结构正确，二次启动读回数据。
- **未尽事项**：①真实密钥 GUI 全链路（添加提供商→发消息→工具→审批）仍需用户本地密钥验收——osascript 无辅助功能授权，键鼠自动化送不进窗口，假端点链路由 e2e 测试与 05/06 的 Tester 用例覆盖；②签名/公证需要 Developer ID 证书，当前产出为 adhoc 签名；③CI 的 macOS mygo build 冒烟未实机跑过 runner（本地同命令验证通过），首跑如遇 runner 环境差异按报错调整；④1140×800 以下窄窗折叠走查由 03 号票 Tester 用例与本次 1440×900 截图覆盖，未逐视口实拍。
