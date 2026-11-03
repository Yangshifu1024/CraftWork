# 10. 收尾：快捷键动效、打包、清理与文档同步

Status: ready-for-agent

## 背景

P3（[spec.md](../spec.md) 分阶段）。入口正式切换、web 残留清除、发布链路重建。

## 改动

- 快捷键全表：⌘K/⌘N/⌘,/⌘B/⌘W、Esc 退出链（一次退一层，IME 组合期忽略；双击 Esc 停止 800ms 确认窗）、审批卡 Esc=拒绝。
- 动效：侧栏折叠 `Transition{Size}`、模式滑块、状态点脉冲、审批扫光；`ReduceMotion` 跟随系统。
- 系统通知：失焦时审批/完成/失败经 `mygo.NewNotification`，点击聚焦并跳会话。
- **删除**：`frontend/` 整目录、`internal/service` 绑定残留、`csp_dev/prod.go`、Taskfile 的 wails 任务、`.github/workflows/ci.yml` 的 web 段（含 Playwright 与绑定漂移检查）。
- 打包：`go tool mygo build`（icns、Developer ID 签名 + 公证、DMG）；版本号注入对齐 `internal/version`。
- 文档同步：AGENTS.md（命令/目录/约束/已知坑重写）、specs（architecture/interface/features/prompt-contract）、README、GLOSSARY 术语、新 ADR 索引。
- e2e 重建：`ui.Tester` 关键链路测试（对话闭环、审批、tab 语义）替代 Playwright 用例。

## 验收

- 全新 clone → `go tool mygo build` → 隔离目录冒烟全链路。
- 仓库 `grep -ri wails` 仅剩历史文档；CI 全绿。
- 1440×900 与 1100×800 两视口对照 interface.md 走查（窄窗折叠行为）。

## 依赖

05–09 全部完成后执行。
