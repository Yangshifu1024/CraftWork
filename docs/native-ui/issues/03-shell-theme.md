# 03. UI 骨架：主题常量与三栏外壳

Status: ready-for-agent

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
