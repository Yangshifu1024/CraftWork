# 05. tab 内容保活（forceMount）

Status: ready-for-agent

## 背景

阶段一切换 tab 用条件渲染，CodeMirror/xterm 实例销毁重建（光标/撤销/搜索状态丢失）。ZCode 的保活基础是所有 tab 内容 `forceMount` + 隐藏不卸载（其 `AnimatedSidePanePanel.tsx:1218-1227`），本票把该机制搬过来（[spec.md](../spec.md) 第 6 节）。

## 改动

- tab 内容渲染改为常挂载：激活 tab 正常显示，非激活 tab `display:none`（面板收起的 `is-closed` 保 DOM 机制不变，二者叠加后任何 tab 任何时刻不重挂载）。
- **relayout 接线（本票核心风险点）**：tab 从隐藏切为显示时主动触发——CodeMirror `requestMeasure()`、xterm `fit()`（FitAddon）、文档预览无需处理；挂在 tab 激活 effect，`requestAnimationFrame` 等一帧再测，双内核（WKWebView / Chromium）各验一遍。
- **file tab 上限 8**：`openFileTab` 超限时关闭最久未激活的 file tab（走关闭流程清槽，**不入**最近关闭栈）；单测覆盖淘汰顺序。
- 内存验证：DevTools 对比改造前后，多 tab 场景堆内存增量可接受；xterm 多实例本就是现状（终端面板内部 term-tabs），确认无重复实例。

## 验收

- 切走再切回：编辑器光标/撤销历史/搜索状态保留、终端输出连续无重放闪烁、文档预览滚动位置保留。
- `display:none` 期间输入焦点不落入隐藏 tab（焦点管理检查）。
- file 第 9 个文件打开时最久未看的被自动关闭。
- 前端四项检查通过；`pnpm run test:e2e` 回归（保活可能改变 e2e 等待时序，重点看截图基线）。

## 依赖

01–04。

## Comments

## Comments

- 已实现于 `3a5815e`（分支 feature/panel-tabs）。验证：前端 lint / tsc / vitest / build 全绿。
