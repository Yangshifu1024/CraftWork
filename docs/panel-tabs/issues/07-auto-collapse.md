# 07. 对话列宽度自动收起

Status: ready-for-agent

## 背景

现有窄窗口行为是固定 ≤1179px 切浮层（`AppShell.tsx:70-76`）。ZCode 的策略更细：窗口 resize 结束后实测**对话列**宽度，< 480px 收面板、< 360px 收侧栏（其 `WorkspaceShellLayout.tsx:831-921`）。搬该策略的 CraftWork 版本（[spec.md](../spec.md) 第 8 节）。

## 改动

- `AppShell.tsx` 增补监听：`window.resize` 触发 → 300ms 防抖（等 resize 结束）→ 实测对话列宽度（`getBoundingClientRect`）→ < 480px 且面板开着 → 收起面板。
- 刻意只监听窗口 resize，**不监听对话列 ResizeObserver**（手动拖宽面板/开面板不被策略关回去——ZCode 注释里明说的教训）。
- 与既有 `setNarrow` 浮层逻辑并存：≤1179px 仍走浮层切换；480px 检查只在宽窗口态生效。
- 面板最大化态同样适用（收起时退最大化，复用既有收敛规则）。
- Esc 收敛链的「浮层 → 最大化」次序不受影响。
- 单测：防抖窗口内只判一次；宽度恢复不自动重开（与 `setNarrow` 的自动展开行为对齐取舍，实现时二选一并写注释）。

## 验收

- 1440×900 逐步缩窗：对话列压到 480px 以下时面板自动收起，无中途闪烁（防抖生效）。
- 手动开面板不触发收起；拖拽分隔条过程不触发（指针事件不触发 window resize，天然满足，验证即可）。

## 依赖

02（收敛规则）。

## Comments

## Comments

- 已实现于 `3e88096`（分支 feature/panel-tabs）。验证：前端 lint / tsc / vitest / build 全绿。
