# 09. 分隔条增强

Status: ready-for-agent

## 背景

拖拽调宽已有（pointer 事件 + `--panel-w`），缺三样：双击重置、拖拽视觉态（`.panel-resize.is-dragging` 样式在 `panel.css:24` 已备但组件未接线）、键盘可达性（[spec.md](../spec.md) 第 8 节）。

## 改动

- 双击分隔条：宽度重置为默认 `PANEL_MIN_WIDTH`（320px），带既有宽度过渡动画。
- 拖拽期间给分隔条加 `is-dragging` class（现 `startResize` 只改了 body cursor，未加 class）。
- 键盘可达：分隔条 `role="separator"` + `aria-orientation="vertical"` + `aria-valuenow/min/max`；获得焦点后左右方向键调宽（步长 16px，Shift 加速 48px）；`tabindex="0"`。
- 视觉走查：拖拽态、焦点态在深浅主题下对照 `--mode-accent` 与既有 hover token。
- 文档同步：`docs/specs/interface.md` 补分隔条交互。

## 验收

- 双击重置；拖拽有视觉反馈；纯键盘可完成调宽（Tab 聚焦 → 方向键 → 数值 aria 属性跟随）。

## 依赖

无（可与 01–05 并行）。

## Comments

## Comments

- 已实现于 `4405e43`（分支 feature/panel-tabs）。验证：前端 lint / tsc / vitest / build 全绿。
