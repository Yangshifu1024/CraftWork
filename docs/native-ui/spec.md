# 原生 UI 迁移（native UI）规格

状态：**已完成**（P0–P3 全部票关闭；P3 收尾见 `issues/10-finish-line.md` 的 Comments。桌面外壳已切换为 mygo 原生 UI，仓库无 WebView / React 残留；现行观感与布局约定以本文为准。决策背景见 [ADR-0014](../adr/0014-native-ui-via-mygo.md)；两轮调研结论摘录在文末。）本文是迁移行为规格：范围、架构映射、约束、分阶段验收。

## 范围

- 把桌面外壳从 Wails3 + WebView + React 切换为 **mygo v0.7.4 原生 UI**（`package ui`），应用内不再有任何 WebView。
- **后端内核不动**：`internal/agent`、`internal/tools`、`internal/policy`、`internal/store`、`internal/secret`、`internal/model`、`internal/editor`、`internal/platform`、`internal/events` 原样保留；`internal/service` 的业务逻辑降级为内部领域包，绑定层语义消失。
- **明确不做**：server 模式、Docker、浏览器验证、WASM 目标、保留 React 前端、CodeMirror/xterm 的全功能对齐。
- 界面功能以 `frontend/` 当前实现为基线（不是 interface.md 里的全部愿景，如文档富预览、逐块 diff 应用不在此迁移范围）。

## 架构映射

| 现状 | 迁移后 |
| --- | --- |
| `application.New` + 10 个绑定 Service | `mygo.App` 单例 + `ui.View`；service 层内部化，view 直调 Go 函数 |
| `cw:events` 批量事件 + 33ms 合并 | bus 订阅回调 + `win.Update(func)`（自动合并）；`internal/events` 类型表保留为契约 |
| `internal/service` Wails 绑定 | 同包普通方法；参数校验与错误转换逻辑保留 |
| 绑定代码生成 + `frontend/bindings/` | 无；类型即 Go 类型 |
| `frontend/src/lib/desktop.ts` 的 `Api` 形状 | `internal/ui` 的 `app` 状态结构持有的后端门面（同方法分组） |
| `threadReducer.ts`（511 行纯函数） | `internal/ui/chat/reducer.go` 逐行移植，Go 单测对齐既有用例 |
| `sidePane.ts`（519 行纯函数） | `internal/ui/panel/tabs.go` 逐行移植 |
| `base.css` token 体系 | `internal/ui/theme.go` 常量（色板/字号档/间距/圆角/阴影），深浅色两套 |
| lucide-react 87 个图标名 | go:embed lucide SVG + `ui.Icon`（`components/Icon.tsx` 登记表即清单） |
| TerminalService（creack/pty + 事件流） | `plugins/terminal`（libghostty 自管 PTY），服务层删除 |
| Wails 单实例/通知/剪贴板/对话框/外链 | `mygo` 对应模块（`RequestSingleInstanceLock`/`NewNotification`/`Clipboard`/`Dialog`/`Shell.OpenExternal`） |
| CSP 中间件、`csp_dev/prod.go` | 无 web 层，全部删除；「不可信内容按文本渲染」边界由 markdown 包默认行为承接 |
| 自动更新留桩（M4） | 接线 mygo 内置 `Updater`（Ed25519 签名 + 增量），作为可选后续项 |

## 约束

- **版本锁定**：`go.mod` 精确锁 mygo v0.7.4，CLI 走 `go tool mygo`；不写包装层，升级时读 mygo 的 `docs/ui/migration.md`。
- **隔离开发**：native UI 开发期用 `CRAFTWORK_HOME=/tmp/...` 隔离数据目录，不碰现网 `~/.craftwork`；`frontend/` 在收尾票之前保持可构建（并行期两套 UI 共存，入口暂不切换）。
- **范式**：立即模式 view 函数；状态集中在 `app` 结构体；后台 goroutine 更新 UI 一律经 `win.Update`，禁止在 view 外直接改 UI 状态。
- **审批边界不变**（ADR-0003/0004 原则）：审批必须显式决定、决定与执行落审计、禁止 Go 侧静默放行；只是「跨 IPC」变为「跨 UI 线程」。
- **文案与行为规则以 Go 为唯一事实源**；原 mock.ts 的近似不复刻。

## 分阶段与验收

各阶段拆票见 `issues/`。每票自带 `ui.Tester` 单测或手工走查清单；阶段验收统一跑：

- `go test ./... && go vet ./...`（golangci-lint 安装后加入）
- `go tool mygo build` 产出可运行 .app
- 隔离目录冒烟：添加提供商 → 发消息 → 工具调用 → 审批允许/拒绝 → diff → 终端 → 重启恢复

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| P0 尖刺 | CLI 安装、四件套验证（标题栏拖拽/分栏+中文流式 markdown/编辑器/终端） | spike 工程四视图可运行，CJK 与深浅色正常 |
| P1 MVP | 后端解绑、外壳+主题、侧栏、聊天主链路、审批闭环、设置基础 | 完整对话闭环可用，数据与 Wails 版互通 |
| P2 对齐 | 面板 tab 体系、文件树、编辑器、diff、终端、设置全量、浮层 | 界面表面全覆盖（按裁剪清单） |
| P3 收尾 | 快捷键/动效/通知、测试体系、删 `frontend/`、文档同步、打包签名 | 验收清单全绿，仓库无 webview 残留 |

## 编辑器/终端裁剪清单（P2 验收基线）

- **编辑器**（`ui/editor`）：语法高亮、行号、软换行、查找、撤销、保存前备份与外部修改确认、`path:line` 跳转。不做：多光标、折叠、LSP、搜索替换正则模式。
- **终端**（`plugins/terminal`）：起默认 shell、真彩、scrollback、选区复制粘贴、Ctrl+L、主题跟随应用、标题跟随、退出提示。不做：搜索、鼠标报告应用、缩放、多会话镜像。
- **diff**（`editor.DiffView`）：统一/并排、词级高亮、应用全部（含备份）。不做：逐块应用/跳过、可编辑新侧。
- **文档预览**：维持文本级（与当前实现一致），富预览不在此迁移范围。

## 设置中心全屏覆盖层（12 号票）

设置中心不是居中模态，而是**铺满窗口的不透明覆盖层**（对齐 React 版 e10fc65 的最终形态，推翻 interface.md 的「920×660 浮层」旧规格）：

- **形态**：盖住整个窗口（含三栏外壳与顶栏），不透明 `bg-panel` 底、无圆角 / 描边 / 阴影、无独立遮罩；头部行 48px（设置图标 + 标题（title 字号）+ 分组搜索框 + 关闭 ×），下方左侧导航列 240px、右侧内容列各自滚动。
- **内容列**：宽窗口下居中限宽 880px（`theme.SettingsContentMaxWidth`），窄窗口（内容区不足 880）时自然铺满；目录浏览器与提供商编辑器替换内容列时同在这一列里。
- **导航**：七组设置 + 「数据」小节三个入口（用量 / 审计 / 备份），底色与内容列同为 `bg-panel`；分组搜索只过滤七组名，无匹配时导航变空、内容区给空态（图标 + 「没有匹配的设置分组」+ 关键词提示）。
- **打开 / 关闭**：⌘, / 侧栏底部 / 命令面板打开（幂等，重复触发不关闭，对齐 React e10fc65 只留两条关闭途径）；关闭途径为 **Esc 与头部 ×**——覆盖层铺满后点击全部落在面板上，遮罩点击关闭自然失效。关闭返回原视图：外壳每帧照常构建，只是被盖住，滚动位置与界面状态天然不丢。
- **浮层层级与 Esc 链**：仍走 `DialogBase`，居浮层体系最底层——数据面板可压其上打开（设置保持打开），全局确认 / 输入框再上，通知最上层；模态打开期间窗口级快捷键整体等待，Esc 归本层的 OverlayShortcut，两段 Esc 依次关最上层（先数据面板后设置），窗口级 Esc 退出链只在无模态时参与。

## 观感约定：聊天正文列宽、composer 聚焦、窗口标题栏与材质（14 号票）

本节是 14 号票新增的观感约定（值取自 `internal/ui/theme` token，不写死在视图里）。

### 聊天正文列居中限宽

- 消息流与 composer 外框共用同一条「正文列」：宽度为所在列的 **80%**（两侧 margin 各 10%，用户给定参数），在列内水平居中；token 为 `theme.ChatColumnMarginPercent`。空态引导保持自身的窄条居中形态（MaxWidth 380），不在本约定内。
- 正文列有最小宽度保底 `theme.ChatColumnMinWidth`：mygo 的 Markdown 在收缩布局里会塌成零宽（AGENTS.md 第 7 节），正文列必须以显式宽度（`WidthPercent` + `MinWidth`）构建，不允许用纯 `MaxWidth` 上限的收缩布局。
- 用户气泡的 `max-width: 78%` 相对**正文列**而非窗口宽；助手消息铺满正文列。
- composer 外框与正文列同宽（80% + 保底），聚焦态与失焦态只有描边色差异。

### composer 聚焦态

- 聚焦高亮套在 composer **整个外框**（含附件条、输入行与工具条），不是仅内部输入区：外框描边从 `BorderStrong` 切到当前模式强调色（`Palette.ModeAccent` 的 `Accent`），失焦恢复。
- 判定用外框的 `FocusWithin`（焦点在内部任意可编辑区即算），帧内写入聊天列状态便于 `ui.Tester` 断言。

### 窗口标题栏形态（复刻 ZCode）

- **无独立标题条**：外壳不再画顶部 48px 标题栏（原 topbar 已删），内容直接从窗口顶开始，红绿灯叠加在内容之上。
- 窗口选项：`TitleBarStyle: hidden`（不是 hiddenInset，内容真正顶到窗口顶）+ `TrafficLightPosition {x: 22, y: 23}`（ZCode `MACOS_TRAFFIC_LIGHT_BASE_POSITION`）；窗口位置与尺寸仍只按 `StateKey` 记忆（与显式 X/Y 二选一，不得并存）。
- 顶部拖拽区（`DragWindow`，双击按系统设置缩放）分两段：
  - 侧栏顶部 48px 空白行：外壳画的侧栏标题行，红绿灯叠在其上（y 23 + 按钮高 < 48，不遮下方内容；ZCode 的左上工具组 CraftWork 没有，96px 左避让参数不引入，机制目的是「红绿灯不遮内容」）。
  - 会话头部行（chatHead）：整行可拖拽；行内无交互控件，无需 no-drag 划分。
- 浮层打开时拖拽让位语义不变（模态由 mygo 的 DialogBase 接管，窗口级快捷键等待）。

### 窗口材质（毛玻璃）

- 窗口启用 `Transparent: true` + `Vibrancy: under-window`（ZCode 同名材质 `vibrancy: "under-window"`）；`BackgroundColor` 不再设置（Transparent 下无效）。
- 视图底色在材质上分派（`c.Vibrancy()` 为真时）：
  - 窗口根不铺底（`c.Root().Background(ui.Transparent)`，主题底色让位给材质）；
  - **侧栏透材质**：侧栏底色改用半透明值（token `theme` 材质派生，对照 ZCode 侧栏不画底），外壳以改写后的色板传给侧栏视图；
  - **对话区与面板保持实底**（对照 ZCode workspace 主区 `bg-background`）。
- 无材质环境（`ui.Tester`、不支持的平台）自动回退全部实底，两态共用一套视图代码。
- `SetActiveAppearance`（ZCode `visualEffectState: "active"`）暂不启用：under-window 材质失焦态由系统绘制，待实机对照后再定。

## 调研结论摘录（2026-11-03）

- mygo：egoist 出品，v0.7.4，MIT，纯 Go 零 cgo（purego），1260 个 ui 测试、零 TODO、文档 ~1.27 万行、21 个 showcase 应用；`markdown.Stream` 追加 16B token 整帧 0.8ms，List 百万行可见行 14.5µs。
- CraftWork 前端 25,887 行（styles 7.8k / stores 4.2k / chat 3.0k / overlays 3.1k / panel 1.9k）；四个 hard 件全部由内置件覆盖；估算全量对齐 10k–15k 行 Go UI + 后端解绑 ~1k。
