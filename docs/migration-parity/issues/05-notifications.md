# 5: 系统通知接线（tauri-plugin-notification）

**labels:** [ready-for-agent]
**对应对账:** spec.md §一 D10、§二 L13

**What to build:** 接通 Runner 既有 notifier 钩子（`runner.rs:94/:117-119/:921-925`）：

- `src-tauri/Cargo.toml` 加 `tauri-plugin-notification`；`capabilities/default.json` 加通知权限；`lib.rs` 注册插件
- `build_app_state` 装配 `runner.set_notifier`：审批待决 / run 完成 / run 失败三类
- **仅窗口失焦时**发（主窗口 focused 状态判断，对齐 Gen1 useNotifications 行为）；受 settings「系统通知」开关门控（开关变化即时生效）
- 点击通知默认聚焦应用（跳转会话依赖插件点击载荷，另开票据 17）

## 验收标准

- [ ] 窗口失焦时：审批待决/完成/失败各收一条系统通知；聚焦时零通知
- [ ] 设置关闭「系统通知」后立即静默（无需重启）
- [ ] `pnpm test:rust`、`pnpm lint:rust` 通过

## 进展
- 2026-11-05：完成（提交 b983121）。tauri-plugin-notification 依赖 + capabilities 授权 + 插件注册；notifier 装配：失焦才发、开关实时读设置；实机通知收发待走查（macOS 通知权限首次弹窗需用户确认）
