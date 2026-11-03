# 12. 设置中心全屏化（对齐 React 版最终形态）

Status: resolved

## 背景

React 版在 2026-11-02 14:24 提交 `e10fc65 feat: 设置中心重构为全屏覆盖层`，把设置中心从 920×660 浮层重构为**全屏覆盖层**，推翻了 `docs/specs/interface.md:181` 的「920×660 浮层」旧规格。原生 UI 迁移按 interface.md 旧规格实现成 `DialogBase` 模态（`internal/ui/overlays/settings.go:192`），复刻用了过时参照。2026-11-03 用户走查确认差异。

## 改动

- 设置从 920×660 模态改为全屏覆盖层：替换主内容区呈现，关闭后返回原位。
- 布局细节对照 `git show e10fc65` 的 React 实现核对（头部结构、七组导航、搜索、返回交互）。
- 浮层语义适配：全屏覆盖层在浮层体系中的层级、窗口级 Esc 退出链中的位置、⌘, 再按切换/关闭的行为，实现时按覆盖层惯例定并留档。
- 同步文档：`docs/native-ui/spec.md`（观感约定）补设置全屏形态；`docs/specs/interface.md` 已标注为历史参照，在其设置条目加一行「后由 e10fc65 改为全屏，原生 UI 按 e10fc65 复刻」即可，不整篇重写。

## 验收

- ⌘, 打开设置为全屏覆盖，Esc / 关闭返回原视图，原视图滚动位置不丢。
- 七组设置、提供商编辑器、目录浏览器、数据面板功能不回退；深浅主题各走查一遍。
- `ui.Tester` 覆盖开合与返回。

## 依赖

07（已 resolved，设置内容与立即生效机制都在）。

## Comments

- 2026-11-03 开票留档：`e10fc65` 提交时间 2026-11-02 14:24:35 +0800，含设置页截图更新（settings-desktop-1440-darwin.png 等），是迁移窗口期内的 React 侧演进，迁移分支未跟上。
- 2026-11-03 实现留档（wt/ui-12）。**形态**：`drawSettings` 仍是 `DialogBase`，面板从 920×660 圆角阴影模态改为 `Fill()` 铺满 + 不透明 `bg-panel` 底、无圆角 / 描边 / 阴影；遮罩底色同步 `bg-panel`（面板盖住后不可见，只防边缘闪色）。导航列 208 → `theme.SettingsNavWidth`（240）且底色 `SidebarBg` → `BgPanel`（对齐 `.settings-nav`，浅色主题下两者不同）；内容列加 `settings-content-inner` 等价物：`FillWidth + MaxWidth(theme.SettingsContentMaxWidth=880) + AlignSelf(Center)`，目录浏览器与提供商编辑器同在这一列；头部标题字号 XL → `Fonts.Title()`（base.css 的 `--text-ui-title` 注释即「仅设置中心页标题」，07 号票漏对齐）。搜索无匹配的空态从导航内一行文本移到内容区 `panel-empty` 形态（图标 + 主句 + 关键词提示，React 是叠在当前分组内容上方，同语义）。底部栏 07 号票本就没迁，无可删项。
- **实现决策（浮层语义，按覆盖层惯例定并留档）**：① 层级不变——设置仍居浮层体系最底层，数据面板压其上打开（设置保持打开），确认 / 输入框再上、通知最上层；② Esc——模态打开期间窗口级快捷键整体等待（mygo `insideModal` 语义），Esc 归 DialogBase 的 OverlayShortcut，两段 Esc 依次关最上层（先数据面板后设置），窗口退出链（`Shell.HandleWindowEscape`）只在无模态时参与，无需改动；③ 遮罩点击——面板铺满后按压全落在面板上，`back.Clicked()` 自然失效，关闭途径只剩 Esc 与头部 ×（对齐 e10fc65 提交说明「关闭途径为 Esc 与标题栏」）；④ ⌘, 保持幂等打开不做成切换——React e10fc65 里 ⌘, 就是幂等 `openSettings()`，且模态等待期间窗口级 ⌘, 本就不触发，做成切换需要给面板额外注册 OverlayShortcut、还要处理数据面板压着时 ⌘, 落到下层 overlay 的边角语义，不值当。观感新增常量落 `internal/ui/theme`（`SettingsNavWidth` / `SettingsContentMaxWidth`），未写死值。
- **框架坑**：`visibleGroups`（原 settingsNav 内联过滤）用 `groupsAlign[:0]` 原地过滤复用了包级切片的底层数组，过滤结果会永久写回共享定义、污染后续所有帧的分组清单——单测各自跑全绿、按序列跑才炸（被新增的层级用例暴露）。已改为每次分配新切片并留注释。ui.Tester 的 `Find` 返回「第一个」匹配文本的元素矩形：导航项与内容区标题同名（如「通用」「高级」）时探针要挑只在导航出现的字符串；几何断言阈值按实测偏移留足余量（全屏后导航文本 x≈102、关闭按钮右缘 ≈1266）。
- **验收对照**：`ui.Tester` 新增六条——全屏几何（标题贴顶 / 关闭贴右缘 / 导航贴左缘）、内容列居中限宽与窄窗（900×700）不限宽、搜索空态、数据面板层级与两段 Esc 链、关闭返回原视图（覆盖层吃按压、关闭后背景控件恢复可交互）与重开回上次分组；「原视图滚动位置不丢」是结构保证（覆盖层不卸载外壳，外壳每帧照常构建），未做滚动断言。深浅主题：外观分组的即时切换用例（07 号票既有）保持通过，全屏面板底色随 `theme.Apply` 实时走，未引入第二套色值。
- **未尽事项**：真机双主题全屏走查截图未做（无头环境，建议下次 GUI 冒烟时按 `CRAFTWORK_HOME=/tmp/cw-12-smoke` 对照 1440×900 与 1100×800 各看一眼全屏设置与目录浏览器在 880 列内的观感）；e2e 截图基线是 React 时代的产物，原生 UI 无对应物，未更新。验证：`go test ./...`、`go vet ./...`、`golangci-lint run`（0 issues）、`go tool mygo build`（.app + DMG）全绿。
