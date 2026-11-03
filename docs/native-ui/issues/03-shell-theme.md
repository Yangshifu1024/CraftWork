# 03. UI 骨架：主题常量与三栏外壳

Status: resolved

## 背景

`frontend/` 之外的新界面地基（[spec.md](../spec.md) 架构映射）。base.css 的 token 体系机械翻译为 Go 主题常量；三栏布局用 `ui.Split` 搭建。

## 改动

- 新建 `internal/ui/`：
  - `theme.go`：色板（bg 8 级/border 3/text 4/状态 5 色/R0–R4/双模式强调色）、字号 7 档派生（15px 基准）、间距/圆角/控件高度档、阴影 4 档、缓动常量；深浅两套 `ui.Theme`，跟随系统实时切换。
  - `icons.go`：go:embed lucide SVG（以 `components/Icon.tsx` 的 87 个名称登记表为清单）+ `Icon(name)` 组件。
  - `shell.go`：三栏外壳（侧栏 268 固定 / 聊天自适应 / 面板 Split 320–760 可拖拽、双击恢复）、顶部 48px 拖拽区（`c.TitleBar()` 避让红绿灯）、面板三形态（停靠/窄窗浮层/最大化）的骨架。
- 窗口背景色按主题取（迁移 `windowBackground` 逻辑）。

## 验收

- 空壳可运行：三栏比例正确、分栏可拖拽、深浅色实时切换、红绿灯避让正确。
- `ui.Tester` 单测覆盖主题派生函数（字号档、语义色映射）。

## 依赖

02。

## Comments

- 2026-11-03 完成于 wt/native-03，已合入 feature/native-ui。`internal/ui`（包名 `ui`，mygo 的 ui 以 `mygo` 别名引入）：`theme.go`（base.css token 全量翻译 + `Apply` 每帧套主题）、`icons.go`（go:embed 登记表 + `Icon`）、`shell.go`（三栏外壳）；`app.go` 占位视图换为 `shell.View`，窗口底色常量收编进 `WindowBackgroundHex`。
- **跟随系统的机制**：mygo 每帧把 `c.Theme()` 重置为跟随系统的默认主题（`runtime.defaultTheme`），view 开头读其 `Dark` 标志选边、`c.SetTheme` 套 CraftWork 变体即可实时跟随；`Tester.SetDark` 下深→浅→深切换用例验证了同一路径。取舍：mygo 默认主题会跟随桌面偏好（系统强调色/高对比/文字缩放），CraftWork 观感以 base.css 为准，accent 与 15px 基准字号固定，不随桌面偏好变化。
- **图标**：登记表逐项对齐 `Icon.tsx`，实测 86 个登记名（票面 87 为笔误；84 个 lucide 文件，`branch`/`list-filter` 为别名），另有 `circle.svg` 作未知名回退。资产取自 unpkg `lucide-static@1.66.0`（与 lucide-react 1.66.0 同版本），解析前把根元素 `stroke-width="2"` 改写为 1.75（前端默认值）。
- **验收**：`go test ./...`（internal/ui 新增 14 个用例）、`go vet`、`golangci-lint run` 0 issues、`go tool mygo build` 产出 .app+DMG。GUI 冒烟（`CRAFTWORK_HOME=/tmp/cw-03-smoke`，临时 X/Y+AlwaysOnTop 补丁便于截图，已还原）：三栏比例正确（侧栏 268/对话自适应/面板 320）、红绿灯避让正确（品牌名与灯位无重叠）、`osascript` 切系统外观深→浅→深各截一张，同一进程内实时换肤，`sips`/像素采样确认各栏底色与 token 逐值一致（BgPanel 顶栏、Border 分隔线、SidebarBg/ChatBg/面板底色）。
- **面板骨架**：停靠形态完整（`ui.Split` 320–760 拖拽钳制、面板左缘 8 DIP 双击恢复 320、窄窗 <1100 自动收起、宽度记忆跨窗口变化保持）；浮层与最大化留桩注释归 08 号票。
- **未尽事项**：真窗口的分栏拖拽未做 GUI 级验证（osascript 无辅助访问权限，无法合成鼠标拖拽），由 `ui.Tester` 的拖拽钳制与双击恢复用例（含几何断言）覆盖；窗口底色首帧前的取值仍按设置 `Theme == "light"` 判定，跟随系统用户首帧用深底，与迁移前行为一致。
