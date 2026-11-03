# 07. 设置中心与浮层体系

Status: resolved

## 背景

对齐 `features/overlays/`（3,065 行）与 Promise 式对话框体系（[spec.md](../spec.md) P1 基础组、P2 全量）。

## 改动

- 通用浮层基建：模态（`ui.Modal`）、Popover 锚定翻转、Toast（`c.Toast`/`ToastAction`）、确认/输入对话框（移植 askConfirm/askPrompt/askFreeConfirm 的 Promise 语义为回调语义）。
- 设置中心（920×660 模态，7 组导航 + 搜索）：通用、模式、模型与提供商（提供商卡片/密钥显隐/能力表/测试连接/删除确认）、工具与权限（4 档 + 自由确认门 + R0–R4 表）、MCP、外观（3 主题卡 + 界面/终端字号）、高级。
- 提供商编辑器：表单（`ui.Form`）、密钥掩码与留空保留、模型多行表单、目录补全。
- 模型目录浏览器：双栏、提供商搜索 + logo（沿用 `CatalogLogo` data URL 通道）、模型过滤/勾选/导入。
- 数据面板：审计表（`ui.Table` 排序/过滤）、用量指标卡与图表（`ui/chart`）、备份列表 + 恢复确认。
- 命令面板：⌘K、命令 + 会话/文件过滤、↑↓ 循环、回车执行。
- 创建项目对话框：两视图（表单→信任确认）、文件夹多选走 `mygo.Dialog`。

## 验收

- 设置项改动立即生效（主题/字号/档位无重启）。
- 目录导入 → 提供商出现在设置 → 选为默认模型 → 发消息走新提供商。
- `ui.Tester` 覆盖确认对话框语义（Esc/回车/焦点默认项）。

## 依赖

03；目录浏览器依赖 02 的 service 内部化。

## Comments

- 2026-11-03 实现者留档：浮层基建落在 `internal/ui/overlays`（包级单例 Store + Backend 服务门面），确认 / 输入 / 自由档位三对话框的 Promise 语义移植为回调（`Confirm(c, opts, done)`），同类请求同时最多一个、新请求覆盖旧请求（旧回调按取消收尾，React 版是悬挂）。Esc / 回车 / 默认焦点语义对齐 React 版并有 `ui.Tester` 单测（确认框焦点在确认按钮、输入框空内容不提交、自由档勾选门 + 默认焦点在取消）。
- 主题 token 与图标下沉为 `internal/ui/theme` / `internal/ui/icons` 叶子包（`package ui` 留类型别名转出口），否则设置中心 / 浮层与侧栏 / 聊天 / 面板平级视图包引用 token 会与外壳成 import 环；合并时注意并行票对 `Palette` 的新增字段（04/08 加过 `Dark`）要落到叶子包而不是 shim。
- 设置立即生效的落点：`theme.SetAppearance`（显式主题覆盖系统偏好 + 字号档全档派生 + 重建两套 mygo 主题）在保存路径的草稿先行阶段调用（保存失败保留本地观感，对齐 React applyTheme）；审批档位 / 往返上限 / 失败转移模型走服务既有实时 resolver。
- 对兄弟票关键的坑：mygo `Scroll` 内的布局在真实窗口与 `ui.Tester` 不一致——`Fill`/`Grow` 在无界约束下高度归零或内容被定位到面板外（tester 测量一切正常），集合列表务必用 `ui.List`（虚拟化）并给显式高度，勿在 Scroll 内逐个构建 ButtonBase 行；`CapturePage` 内容捕获不含浮层，冒烟截图用 `screencapture -x -R`；并行票冒烟会抢单例锁与屏幕，冒烟构建用独立应用名 + 固定 X/Y + AlwaysOnTop（临时补丁，已还原）。
- 2026-11-03 合并者补录：对兄弟票关键的坑——模态面板的垂直偏移不能靠 `AlignSelf`，它只覆盖交叉轴（水平）对齐；要放到非居中高度（命令面板 9%、确认框 16–18%）得改 backdrop 主轴 `Justify(mygo.Start)` 再用上边距落位（`overlays.placePanel` 已封装，fraction 为 0 时保留 DialogBase 默认垂直居中）。
- 提供商目录 logo 是 data URL，走 `mygo.DecodeBitmap` 解码（SVG 不支持，回退两位字符文字头像）；216 个 logo 异步抓取完成各自 `win.Update` 会触发重绘风暴，缓存条目带 loading 标记防重复发起。
- 验收走查（`CRAFTWORK_HOME=/tmp/cw-07-smoke`，17 张截图留档 /tmp/cw-07-shots）：⌘, 开设置翻七组、深浅主题各一张即时切换、目录浏览器 216 提供商双栏、命令面板 ⌘K 十二命令与待接线标注、确认框 / 三语气通知 / 数据面板三分区 / 创建项目两视图；`go test ./...`、`go vet`、`golangci-lint run` 0 issues、`go tool mygo build` 产出 .app+DMG。04 票侧栏底部 ⌘K / ⌘, / 用量 / 审计入口已直开浮层体系，删除确认换成应用内危险确认框（04 的 TODO(07) 完成）。
