# 06. 面板偏好持久化

Status: ready-for-agent

## 背景

面板宽度、开合目前零持久化（全前端仅主题 `cw-theme` 落盘）。对齐 ZCode 的持久化哲学：**布局偏好进 localStorage，tab 列表只进内存**（避免恢复失效路径，见 [spec.md](../spec.md) 第 7 节）。

## 改动

- localStorage 两个键：`cw-panel-width`（像素宽度）、`cw-panel-open`（开合布尔）。读写封装到 `stores/panel.ts`（或 `lib/storage.ts` 既有工具，看仓库现状）；启动时读取，非法值回退默认（宽度夹取 `[320, 760]`）。
- 写入时机：`setWidth` / `setOpen` / `toggle` / 收敛规则触发时同步写；不做防抖（低频操作）。
- **tab 列表不持久化**：重启后各工作区桶为空；`cw-panel-open` 为 true 时显示默认空态。
- 窄窗口自动收起（`setNarrow`）不改写持久化的开合值（临时态，宽窗口恢复时读回用户偏好）——实现时注意与 02 的收敛规则区分。
- Mock（`mock.ts`）无需改动（纯前端状态，不涉 API）。

## 验收

- 刷新后宽度与开合恢复；篡改 localStorage 为非法值后回退默认。
- 窄窗口进出后开合偏好不被临时收起覆盖。
- 桌面与 server 模式行为一致。

## 依赖

02（store 改造后接持久化）。

## Comments

## Comments

- 已实现于 `4cfbd09`（分支 feature/panel-tabs）。验证：前端 lint / tsc / vitest / build 全绿。
