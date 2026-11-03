# HandOff：wails3 → mygo 原生 UI 迁移（2026-11-03）

> 交接文档：写给接手本仓库的下一个 Agent/开发者。目标读者已具备仓库基础认知（先读根目录 `AGENTS.md`）。本文只记「不在其他工件里的会话级上下文」，不重复 spec/票/提交已有内容。

## 一句话现状

桌面外壳已从 Wails3 + WebView + React 完整切换为 mygo v0.7.4 原生 UI：迁移分支 `feature/native-ui` 上 10 张实现票（01–10）全部完成合入，code-review 两轴（Standards/Spec）已完成，**review 修复轮正在进行/刚完成**（见下文「接手时先确认」）。分支尚未推送、尚未开 PR。

## 决策与规格（读这些，不要问）

- 决策：`docs/adr/0014-native-ui-via-mygo.md`（取代 ADR-0001/0009；放弃 server 模式与浏览器验证是用户明确拍板）
- 权威规格：`docs/native-ui/spec.md`（架构映射、约束、编辑器/终端/diff 裁剪清单）
- 任务分解与实现记录：`docs/native-ui/issues/01`–`10`——**每张票的 `## Comments` 是实现决策与 mygo 框架坑的第一手来源**，改 `internal/ui` 前先读相关票

## 接手时先确认（本文写作时的在途状态）

1. **review 修复轮**：一个后台实现者正在集成分支修复 review 发现的问题——theme 外写死 hex 收编、`panel_desktop.go` 三处 TODO(07) 换浮层、`sidebar/view.go` 重命名换 `overlays.Prompt`、`chat/client.go` 失败加 toast、`WriteClipboard` 契约修复、时钟/剪贴板/参数结构体等 smell 收敛。**`settings.go`（1191 行九 tab）明确不拆**，只留档注释。
2. 修复轮的验收基线：`go build ./...`、`go test ./...`（25 包）、`go vet`、`golangci-lint run` 0 issues、`go tool mygo build`。
3. 待办：推送分支 → 开 PR（→ main）→ 跟进 CI。

## 本机环境事实（GUI 验证相关，踩过的坑）

- **osascript 无辅助功能权限**：无法合成键鼠/窗口断言（历票反复确认）。交互验证走 `ui.Tester`（headless）+ 真实服务链；截图用「窗口固定 X/Y + `AlwaysOnTop` + `screencapture -x -R`」。**屏幕锁定时截屏拍到的是锁屏画面**（06 票踩过）。
- 双显示器（主屏 1512×982@2x + 副屏负坐标）；全屏截图会截到无关应用，务必区域截图。
- 本机 GOPATH 是 `/Users/yzb/Developer/go-workspace`（**不是 `~/go`**）；`mygo`/`golangci-lint` 在 `$(go env GOPATH)/bin`。
- mygo CLI v0.7.4 已装，但项目内统一用 `go tool mygo`（go.mod tool 指令锁版本）；升级 mygo 是显式决策（读其 `docs/ui/migration.md` 先）。
- libghostty dylib 在 `~/Library/Caches/mygo/natives/`（裸 `go run` 首次下载；`mygo build` 产物内置，运行时不下载）。
- 冒烟一律 `CRAFTWORK_HOME=/tmp/...` 隔离；验证桩（OpenAI 兼容 SSE 假端点、cwsmoke 标签种子）模式见 05/06/09 票 Comments。

## 流程惯例（本会话的实际工作方式，接手可复用）

- implement-spec 流程：每票一个 git worktree（`/Users/yzb/Code/Personal/craftwork-wt-NN`，分支 `wt/native-NN`）+ 后台实现者子代理 + 合并者子代理合入集成分支；槽位纪律（shell.go 各票只动自己的列）+ 冲突「他人槽位采用对方版本并报告」。
- 提交规范：中文信息、一票多提交按逻辑粒度、全部 `git commit -s`（DCO）。
- 每票完成后：实现者把「实现决策 + 框架坑 + 未尽事项」写回票 Comments——**这是本会话最有价值的沉淀习惯**。

## mygo 框架坑速查（详情在各票 Comments）

- view 帧内直调 `Clipboard`/`Dialog` 会死锁（onMain 等主循环而主循环跑帧中）→ 一律 `win.Update`/goroutine（04 票）。
- 空 view 窗口不显示：首帧必须画内容（铺主题背景）（02 票）。
- `App.SetPath` 不自动建目录（02 票）；`ui.Split` 宽度指针初值必须非零（01 票）；`StateKey` 与显式 X/Y 二选一（01 票）。
- 已注册快捷键不再投给焦点元素（engine.claimed）→ 焦点守卫用条件注册（10 票）。
- `Scroll` 无界约束下 Fill/Grow 高度归零 → 集合用 `ui.List` + 显式高度（07 票）。
- `ReduceMotion`：Animate/Transition 自动跟随系统，Loop 需 `theme.ReduceMotion(c)` 手动门控（10 票）。
- P0 报过的「终端 PTY 网格协商失效」已在 09 票**证伪**（早期 80×24 兜底暂态被误读），风险关闭。

## 已知未尽事项（合并 PR 后仍有效的）

- 真实密钥 GUI 全链路走查（键鼠送不进窗口，假端点链路已由 e2e/Tester 覆盖）——AGENTS.md 第 10 节保留该验收项。
- Developer ID 签名与公证：当前 adhoc 签名，需要用户证书。
- 新 CI 的 macOS `go tool mygo build` 段未实机首跑（PR CI 即首跑）。
- `internal/ui/overlays/settings.go` 九 tab 同文件（1191 行）拆分留档未做（review 判定风险>收益）。
- 文档预览仍为文本级、面板最大化留桩——均有注释标注归属。
- 等待审批中 Esc 的审计口径是 `canceled`（非 deny），与 React 版同语义，验收方需知悉（Spec 轴 review 记录）。

## Suggested skills（下一个 Agent 按需调用）

- 开 PR / 跟 CI：`pr`
- 继续票流程或新增收尾票：`implement-spec`（issue tracker 在 `docs/<feature>/issues/`）
- 再次全量审查：`code-review`
- GUI 交互走查（若辅助功能权限已授予）：`computer-use:computer-use`

## 环境清理提醒

合并与推送后：删除 9 个 worktree（`git worktree remove craftwork-wt-NN` + `git branch -d wt/native-NN`）与磁盘残留未跟踪的 `frontend/` 构建产物目录（dist/node_modules/test-results——gitignore 已移除前端条目所以它们显示为未跟踪）。
