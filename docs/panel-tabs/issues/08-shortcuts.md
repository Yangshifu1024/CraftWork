# 08. 面板快捷键

Status: ready-for-agent

## 背景

面板目前无快捷键（只能按钮/命令面板）。对齐 [spec.md](../spec.md) 第 9 节；`Cmd/Ctrl+W` 的桌面限定是硬约束——server 模式下它是浏览器自身关标签，**绝不拦截**。

## 改动

- `Cmd/Ctrl+B`：开合右侧面板（等价 `toggle()`）。桌面与 server 模式都注册；命令面板同步暴露「开合右侧面板」。
- `Cmd/Ctrl+W`：关闭当前激活 tab（走 `closeTab`）；无可见 tab 时不拦截、不弹 toast。**仅桌面注册**——注册判断沿用 `lib/bus.ts` 的 `isDesktop`（server 模式直接不挂监听，而不是挂了再 preventDefault）。
- `Esc` 退出链在**最前**插入一级：激活 tab 可关时先关 tab；随后才是既有顺序（浮层 → 最大化 → 运行中会话双击确认 → 待审批，`AppShell.tsx:118-163`）。注意与输入框焦点场景的兼容（编辑器/终端聚焦时 Esc 语义优先局部组件，维持现状，不劫持）。
- 实现位置遵循现有全局快捷键惯例（AppShell 全局监听或 app store 快捷键层，看仓库现状对齐）。
- e2e：桌面链路 Cmd+W 关 tab、无 tab 时透传；server 模式确认未注册。

## 验收

- 桌面：Cmd+B 开合；Cmd+W 逐个关 tab 至面板收起，再按不产生任何效果；Esc 链次序正确。
- server 模式：Cmd+W 由浏览器处理（关浏览器标签），页面无拦截行为。
- 命令面板两条命令可用。

## 依赖

02、03（tab 关闭语义就绪）。

## Comments

## Comments

- 已实现于 `d19e1b3`（分支 feature/panel-tabs）。验证：前端 lint / tsc / vitest / build 全绿。
- 实现说明：Cmd/Ctrl+W 的注册范围判断用新增的 isDesktopWindow（wails: 协议 / wails.localhost），而非 isDesktop——server 模式下 isDesktop 也为 true，不能用于此判断。Esc 关 tab 带焦点守卫：焦点在面板可编辑区域内（终端隐藏输入、编辑器 contenteditable）时整级跳过。
