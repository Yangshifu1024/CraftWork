# 02. 后端解绑：service 内部化与事件投递改造

Status: resolved

## 背景

native UI 的 view 直接调 Go 函数，绑定层与批量事件通道失去存在意义（[spec.md](../spec.md) 架构映射）。本票把后端从 Wails 形态解绑出来，是所有 UI 票的地基。

## 改动

- `internal/app/app.go` 重写装配：去掉 `application.New`/`Services`/`AssetOptions`/`SingleInstance`/CSP 中间件；改为 `mygo.App.WhenReady` + `NewWindow(WindowOptions{...})`（TitleBarHidden、最小 960×640、`StateKey`）+ `RequestSingleInstanceLock`/`OnSecondInstance` 唤起窗口 + `OnWillQuit` 关闭 bus。
- `internal/service`：方法签名保持不变，成为内部领域包；删除对 `application.Get()` 的调用（`service/app.go` 的 OpenExternal/剪贴板/对话框/窗口操作改走 `mygo.Shell`/`Clipboard`/`Dialog`/窗口句柄，`workspace.go` 的 PickFolder 同理）。
- 事件投递：`emitter` 改为「bus 订阅 → `win.Update` 派发到 UI 状态」；删除 `internal/record` 的 33ms 合并（或改为直接透传）；`internal/events` 类型表保留。
- `TerminalService` 停用（终端由 `plugins/terminal` 自管 PTY，见 09 票），相关事件类型暂时保留。
- main.go 去掉 embed，改为普通装配入口。

## 验收

- `go test ./...` 全绿（service 层既有测试不改断言）。
- 临时 main 能起原生空窗口并在第二次启动时唤起。
- 数据目录行为不变：`CRAFTWORK_HOME` 隔离可用，既有 SQLite/keys.json 直接打开。

## 依赖

01。

## Comments

- 2026-11-03 完成于 wt/native-02 分支的两个提交：`9d3b31a`（events 透传化）与 `cab1374`（app 装配切 mygo、service 去 wails），已合入 feature/native-ui。
- 验收摘要：17 包测试/vet/lint 全绿，`go tool mygo build` 产出 .app+DMG，隔离目录 GUI 冒烟通过（含二次启动唤起与窗口状态恢复）。
- 实现者发现的坑（后续 UI 票注意）：①native UI 窗口画出首帧才显示，空 view 必须铺主题背景，否则启动瞬间白屏；②mygo `App.SetPath` 不自动创建目录，window-state 写入前需先 mkdir。
- 未尽事项：CI 绑定漂移检查在本分支会红（预期，10 号票改造）；OpenExternal/剪贴板/对话框待 03+ 走查覆盖。
