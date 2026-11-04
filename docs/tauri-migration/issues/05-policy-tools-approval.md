# 05 策略、工具框架与审批（P4）

labels: [needs-triage]
status: pending
depends: 03（可与 04 并行）

## 任务

1. `craft-policy` crate：风险分级（R0~R4 与参数提升规则）、审批档位（graded/strict/readonly/free）、路径安全解析（词法校验 + symlink realpath）、命令分类、敏感附件名——现有 policy 逻辑逐条直译。
2. `craft-tools` crate：Registry（spec 声明：风险等级、路径参数、命令参数、Work 模式可用性、标题/摘要）+ `assess` / `execute` 接缝；输出截断与写前备份（ctx.backup）由 Registry 统一处理；内置工具先平移 `fs_*`、`apply_patch`、`git_*`（经 shell）；`shell_exec` 本期仅注册 spec（执行体在 07）。工具抽象参考 codex-rs 的 tools/execpolicy 拆分方式（只参考设计）。
3. 审批状态机：请求 → 卡片事件 → 用户决定 → 执行/跳过；授权缓存（once/session/workspace，服务端校验；`allowPermanent=false` 时一律按 once）；受限工作区（R3 拒绝、R1/R2 逐次、已有授权不生效）；取消/关窗/退出按拒绝或取消落审计。
4. 审批决定与工具执行写入 audit_log（经 Recorder）。
5. tests/policy 与 tools 测试逐条翻译；黄金文件对拍测试建立（plan §3）。

## 验收

- 现有全部 policy 风险分级/路径解析用例在 Rust 侧通过；
- 审批全流程（含三种授权范围、受限工作区、取消路径）单测覆盖；
- 主进程侧无任何静默放行路径。

## 进展
