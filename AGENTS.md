# AGENTS.md

本文件是本仓库的协作与实现约定，面向在本仓库内写代码的 Agent 与开发者。

`README.md` 面向使用者与贡献者，介绍功能、架构与贡献流程；本文件回答「在这个仓库里应该怎么改、怎么验证、哪些东西不能碰」。

---

## 1. 项目现状

CraftWork 是一个本地优先、可审计的桌面 Agent。**M0–M3 已完成**（骨架、对话、Code 闭环、文档文本级读写）：

- Wails3 v3.0.0 桌面外壳 + 系统 WebView。
- Go 1.26 后端：Agent 循环、工具、策略、审批、SQLite、PTY、密钥。
- React 19 + TypeScript 前端：三栏界面、聊天、面板、浮层。
- 已打通：多提供商（OpenAI 兼容、Anthropic、Gemini）、流式回复、工具与审批、上下文压缩、消息分页、附件、通知、文件树、CodeMirror、差异、补丁、Git、终端、设置、审计、用量、备份、Word / Excel / PDF 文本级读写。

M4（MCP、沙箱、自动更新）目前是留桩，代码中已有占位与界面标注。M3 的文本级限制见第 10 节。

### 1.1 技术栈

| 层 | 选型 |
| --- | --- |
| 桌面外壳 | Wails3 v3.0.0（系统 WebView，不打包 Chromium） |
| 后端 | Go 1.26、SQLite（modernc.org/sqlite）、creack/pty |
| 前端 | React 19、TypeScript 5、Vite 8 |
| 对话界面 | @assistant-ui/react 0.15（headless primitives） |
| 编辑器 / 终端 | CodeMirror 6 / xterm.js 6 |
| 图标 / 样式 | lucide-react 1.66；原型 CSS token 与 class，不使用 Tailwind |

## 2. 环境与工具链

| 工具 | 版本 | 说明 |
| --- | --- | --- |
| Go | 1.26+ | 以 `go.mod` 的 `go` 指令为准 |
| Node.js | 24+ | 以 Vite / `package.json` 要求为准 |
| Wails3 CLI | v3.0.0 | 需在 PATH 中；用 `wails3 doctor` 检查环境 |
| 前端依赖 | — | `cd frontend && npm install` |

规则：

- **不要擅自安装或升级工具**；需要新工具或版本时先向用户提出请求。
- 使用 PATH 中已配置的 Go / Node / Wails3，不通过临时环境变量切换到其他版本或位置。
- `wails3` 不在 PATH 时，先确认用户的环境配置；不要假设固定的安装路径。
- Wails3 未安装时，按官方文档安装 v3.0.0；安装前先征求用户同意。

### 数据目录

- 默认 `~/.craftwork`：`craftwork.db`、`keys.json`、`backups/`。
- `CRAFTWORK_HOME=/tmp/...` 可隔离数据目录，测试与冒烟必须使用隔离目录。
- `CRAFTWORK_DEMO=1` 写入演示工作区与线程；普通启动不写演示数据。

### 不能提交的生成物

`bin/`、`frontend/dist/`、`frontend/node_modules/`、`.task/`、`.DS_Store`。

`frontend/bindings/` 是 Wails3 生成的 TypeScript 绑定，**需要提交**，保证浏览器预览与 CI 类型检查可用；不要手工编辑，通过 `wails3 generate bindings` 重新生成。

## 3. 常用命令

| 目的 | 命令 |
| --- | --- |
| 安装前端依赖 | `cd frontend && npm install` |
| 生成绑定 | `wails3 generate bindings -clean=true -ts -i` |
| 桌面开发模式 | `wails3 dev` |
| 纯前端预览（Mock 后端） | `cd frontend && npm run dev` → `http://127.0.0.1:9245/` |
| 生产构建 | `wails3 build` → `bin/craftwork` |
| macOS 打包 | `wails3 package` → `bin/craftwork.app` |
| Go 测试 | `go test ./...` |
| Go 静态检查 | `go vet ./...` |
| 前端类型检查 | `cd frontend && npm run typecheck` |
| 前端单元测试 | `cd frontend && npm run test` |
| 浏览器端到端测试 | `cd frontend && npm run test:e2e`（首次需 `npx playwright install chromium`） |
| 截图回归 | `cd frontend && npm run test:e2e -- --grep @screenshot`（基线按操作系统保存） |
| 前端构建 | `cd frontend && npm run build` |
| 一键测试 | `wails3 task test` |
| 一键检查 | `wails3 task check` |

> 修改 Go 服务的导出方法签名后，必须重新生成绑定，否则前端类型会与后端不一致。

## 4. 目录与职责

```text
main.go                  入口：embed frontend/dist，交给 internal/app
internal/app/            窗口、单实例、CSP、服务装配、关闭钩子
internal/service/        Wails 绑定层（App / Settings / Thread / Chat /
                         Workspace / Provider / Audit / Terminal / Document）
internal/agent/          Run 循环、审批 Broker、授权缓存
internal/tools/          fs_* / shell_exec / git_status / todo_write
internal/policy/         风险分级、审批档位、路径解析、命令分类
internal/model/          统一 Client、提供商目录、Mock、OpenAI 兼容 SSE
internal/store/          SQLite、迁移、事件、审计、用量、设置、种子数据
internal/secret/         keys.json 读写与权限
internal/editor/         行级 diff
internal/events/         事件类型与批量总线
frontend/src/app/        AppShell、全局快捷键
frontend/src/features/   sidebar / chat / panel / overlays / statusbar
frontend/src/components/ Icon、BrandMark、Popover、Toast、Dialog
frontend/src/lib/        API、桌面绑定、浏览器 Mock、类型、工具
frontend/src/stores/     app / threads / panel / workspace / terminal / ui
frontend/src/styles/     原型 CSS 分区 + highlight + app-extra + desktop
docs/                    需求、界面、技术方案、原型
```

新增能力时优先落到对应领域包，不要在 `internal/service` 里堆业务逻辑；服务层只做参数校验、编排与错误转换。

## 5. 改动流程

1. **先读文档**：`docs/产品需求文档.md`、`docs/技术设计方案.md`、`docs/界面设计方案.md`。
2. **改 Go 接口**：修改服务方法或 `RegisterEvent` 后重新生成绑定。
3. **改前端**：保持原型 class 名与 token；类型检查必须通过。
4. **补测试**：核心逻辑（策略、路径、事件、存储、Agent 循环、diff）必须有 Go 单测；前端关键 reducer 在 M1 接入 Vitest。
5. **跑验证**：见第 8 节。
6. **同步文档**：见第 9 节。

### 新增一个绑定服务

1. 在 `internal/service/` 新建具体类型与构造函数。
2. 在 `internal/app/app.go` 的 `application.Options.Services` 注册。
3. `wails3 generate bindings`。
4. 在 `frontend/src/lib/api-types.ts`、`desktop.ts`、`mock.ts` 同步实现。
5. 补 Go 单测与前端类型。

### 新增一个工具

1. 在 `internal/tools/` 实现 `Tool` 接口（`Name/Description/Schema/Execute`）。
2. 在 `NewRegistry` 注册。
3. 在 `policy.baseRisk` 增加风险等级；需要路径/命令判断时补 `Assess`。
4. 确认输出截断、错误结构化、写前备份。
5. 补策略与工具测试。

### 新增一个前端面板或浮层

1. 面板放 `features/panel/`，用 `lazy()` 按需加载（编辑器、终端、文档较重）。
2. 浮层放 `features/overlays/`，通过 `useUiStore` 控制开关，并用 Portal 挂到 `document.body`。
3. 复用 `styles/` 里的 class 与 token；新增样式放 `app-extra.css` 或对应分区文件。
4. 图标只通过 `Icon` 组件使用，在 `components/Icon.tsx` 登记新名称。

## 6. 实现约束（必须遵守）

### 6.1 IPC

- 命令走绑定方法；事件只走 `cw:events` 批量通道。
- 文本/推理增量在 Go 侧按 33ms 合并；不要逐 token 推送。
- 非增量事件（工具、审批、生命周期）立即推送，保持顺序。
- 大对象（文件内容、差异、终端历史）用命令按需拉取，不塞进事件。
- 事件类型与载荷在 `internal/events/events.go` 与 `frontend/src/lib/types.ts` 两处保持一致。

### 6.2 审批

- 审批必须跨 IPC：`approval.requested` 事件 + `ChatService.DecideApproval` 命令，不允许在 Go 侧静默放行。
- `once / session / workspace` 三种范围由服务端校验；`AllowPermanent` 为 false 时禁止 session/workspace。
- 受限工作区：R3 直接拒绝、不提供永久授权、已有授权不生效。
- 取消、窗口关闭、应用退出统一按拒绝或取消落审计。
- 审批决定与工具执行必须写 `audit_log`。

### 6.3 安全

- 窗口只加载 `/`，**禁止把绝对 URL 传给窗口**；外链只走 `AppService.OpenExternal`（http/https）。
- 修改资源响应头时保留 CSP；生产构建不允许内联脚本与远程资源。
- Markdown 不渲染原始 HTML；图片不加载远程地址。
- 所有路径校验在 Go 侧完成：`policy.Resolve` 先词法校验，再解析符号链接；不信任前端路径。
- 密钥只存在 Go 进程与 `~/.craftwork/keys.json`；不进入日志、数据库、事件、导出包。
- `shell_exec` 必须清洗敏感环境变量、杀进程组、限制输出。

### 6.4 前端

- 使用 assistant-ui **headless primitives**，不要引入 `@assistant-ui/react-ui` 或 Tailwind。
- 自定义消息部件使用 `data-cw-*`，在 `MessageView.tsx` 的 `partComponents.data.by_name` 注册。
- 用户消息不能带 `status` 字段（assistant-ui 约束）；只有 assistant 消息带状态。
- 图标只用 `lucide-react` 并通过 `Icon` 包装；不手写 SVG path（品牌 logo 除外）。
- 颜色、间距、圆角来自 `styles/base.css` 的 CSS 变量；不新增第二套 token。
- 设置项必须立即生效；未接线的控件要禁用并标注里程碑，不要做“看起来能改但无效果”的控件。
- 面板、终端、编辑器、设置等重组件用 `lazy()` 拆分，控制首屏体积。

### 6.5 数据

- SQLite 是事实源；事件表按线程 `seq` 单调递增，写入在事务内完成。
- 修改表结构时增加迁移版本，不要直接改已有迁移。
- 写入文件前备份；外部修改时用 `expectedHash` 阻止覆盖。
- 软删除、审计、用量等保留策略与需求文档一致。

### 6.6 错误处理

- Go 侧返回可读错误；不要 panic 到用户界面。
- 模型调用错误统一使用 `internal/model` 的 `Error` 与错误码（`auth`、`rate_limit`、`timeout`、`context_overflow` 等）；新增提供商必须映射到既有错误码，不要向前端抛原始 HTTP 错误。
- 前端调用失败要 `toast` 提示，不能只 `console.error`。
- 后台 goroutine 失败要写日志并推送 `error` 事件，不能静默退出。

## 7. 已知坑

- Wails3 **没有默认 CSP，也不拦截窗口导航**；安全靠 Go 中间件与统一外链出口。
- Wails3 `SingleInstance` 只通知，不自动唤起窗口；回调里要 `Restore/Show/Focus`。
- macOS 图标使用 `icons.icns`；不要重新引入 `Assets.car` 或 `CFBundleIconName`，否则会覆盖新图标。
- `wails3 build` 需要 `frontend/dist`；正常流程会自动构建前端。
- Vite HMR 与 Zustand 同时存在时，模式切换可能出现旧 store 实例；验证时整页重载。
- 浏览器预览与桌面端的桥接通过 `isDesktop` 判断；新增 API 时三个实现（`desktop.ts`、`mock.ts`、`api-types.ts`）都要改。
- `frontend/bindings` 不要手改；重新生成即可。

## 8. 验证清单

提交前至少执行：

```bash
go test ./...
go vet ./...
cd frontend && npx tsc --noEmit && npm run build
wails3 build
```

涉及 UI 时额外：

- `cd frontend && npm run dev`，在 1440×900 与 1100×800 两个视口对照 `docs/prototype/index.html`。
- 检查三栏布局、48px 顶行、27px 状态栏、窄窗口折叠、深浅主题。
- 走一遍：发送消息 → 工具调用 → 审批允许/拒绝 → 差异面板 → 审计记录。
- 桌面端启动冒烟（建议隔离目录）：

```bash
CRAFTWORK_HOME=/tmp/cw-smoke CRAFTWORK_DEMO=1 ./bin/craftwork
```

CI（`.github/workflows/ci.yml`）在 Ubuntu 上执行 `go test`、`go vet`、`govulncheck`、`npm ci`、`npm run typecheck`、`npm run build`、`npm audit`，并重新生成绑定检查 `frontend/bindings` 是否漂移。本地提交前仍以本节命令为准。

## 9. 文档同步

| 改动 | 需要同步 |
| --- | --- |
| 技术栈、分层、IPC、安全策略 | `docs/技术设计方案.md` |
| 产品定位、约束、功能验收、指标 | `docs/产品需求文档.md` |
| 布局、组件、交互、图标、视觉 | `docs/界面设计方案.md` |
| 原型行为或模块映射 | `docs/prototype/README.md` |
| 使用方式、功能、架构、贡献流程 | `README.md`、`CONTRIBUTING.md` |
| 开发约定、命令、陷阱 | 本文件 |

版本号规则：需求文档 v4.x、技术方案 v2.x、界面方案 v1.x；修改后在文档头同步日期。

## 10. 当前留桩

- M4：沙箱、自动更新、签名与公证、Streamable HTTP MCP、MCP 信任列表。
- M3 限制：Word 只支持同一格式片段内的文本替换；Excel 不计算公式；PDF 只读文本层，不支持 OCR 与编辑。
- 真实密钥验收：OpenAI / Anthropic / Gemini 需要本地配置密钥后各跑一次工具调用闭环。
- 跨平台：Windows 系统通知、Linux 打包与 CI 兼容性需要实机验证。

## 11. 提交规范

- 提交信息使用中文，一次提交只做一件逻辑上的事。
- 提交前运行第 8 节的检查；不要提交 `bin/`、`frontend/dist/`、`node_modules/`。
- 贡献使用 DCO：`git commit -s`。
- 不修改 `docs/prototype/` 的视觉与交互，除非设计变更明确要求；修改后同步界面方案与原型说明。
