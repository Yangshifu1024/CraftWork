# 计划模式（plan-mode）行为规格

本文是计划模式的行为规格与边界裁决来源。2026-11 重构对齐 ZCode 的 EnterPlanMode / ExitPlanMode 设计，`docs/arch.md` 第 7 节 D8 决策条目同步修订（原「纯提示词约束」方案已被替代）。

## 术语

- **计划态（plan_active）**：run 级布尔状态，决定提示词注入与只读闸门。随消息勾选初始化，运行中由协议工具翻转。
- **计划 run**：以 planMode=true 消息触发的 run（用户勾选路径）；或运行中模型调 enter_plan 进入计划态的 run（模型主动路径）。
- **计划行（plans 表）**：计划 run 启动即建（approved=0、无文件）；exit_plan 提交时回填文件路径，批准时置 approved=1。

## 参照系（ZCode 对应关系）

| 本仓库 | ZCode | 差异 |
| --- | --- | --- |
| `enter_plan` 工具 | `EnterPlanMode` | snake_case 命名遵循仓库惯例 |
| `exit_plan` 工具 | `ExitPlanMode` | 无 allowedPrompts 权限预申请（远期可选） |
| plan.entered 事件 | Plan 标记定向开启 | 纯广播不入库 |
| 计划态只读闸门（R2+ 拒绝） | mode.plan.nonReadOnly 拒绝 | 一致：只拦不放，放行只来自原模式 |
| exit_plan 无条件审批 | ExitPlanMode 始终需确认 | 一致：自由档也弹卡 |
| 审批三路径 | 批准继续 / 反馈升级用户消息 / 无反馈终止 turn | turn≈本仓库 run 迭代 |
| 计划文件 `~/.craftwork/plans/` | `${workspaceRoot}/.zcode/plans/` | 本仓库放数据目录（planOpen 白名单校验），不放工作区 |
| compact 后计划 reminder | plan_file_reference model-only reminder | 一致：排摘要之前 |
| 跨 run 注入（提示条兜底路径） | —（ZCode 无此路径） | 本仓库保留的兜底 |

明确不做：`/plan` 斜杠命令（无 slash 体系）、goal 命令互斥（无 goal 体系）。

## 状态与生命周期

```text
用户勾选 planMode 发送 ──┐
                        ├─> plan_active=true，计划 run 启动（create_plan 建行）
模型调 enter_plan ───────┘        │
                                  │ 只读调研（R2+ 被闸门拒绝）
                                  v
                        模型调 exit_plan（plan 全文）
                                  │
                    ┌─────────────┼─────────────┐
                    v             v             v
                  批准        拒绝带反馈      拒绝无反馈
                    │             │             │
              退出计划态      保持计划态      保持计划态
              落盘 approved=1 落盘未批准     落盘未批准
              同 run 继续实施  反馈→用户消息   run 正常完成
                              修订后重新提交  （提示条数据就绪）
```

- plan_active 初始值 = trigger 消息的 planMode；`enter_plan` 置 true（幂等，广播 plan.entered）；`exit_plan` 批准置 false。
- 计划态中途翻转后，下一迭代重建系统提示词（只动 dynamic 块）。
- `exit_plan` 在非计划态调用 → 普通拒绝（模型可自行纠正），不弹卡。
- 参数校验失败（空 / 超 20000 字符）→ 普通拒绝，不弹卡。

## exit_plan 审批三路径（核心合同）

对齐 ZCode `plan-mode-approval-boundary`：

1. **批准**：退出计划态；计划原文（模型入参，不 trim 不改写）原子写入 `~/.craftwork/plans/plan_<id>.md`；plans 行回填路径并置 approved=1；广播 plan.created；工具结果「已批准，开始实施」；**同 run 继续迭代直接实施**（后续写操作恢复档位判定）。
2. **拒绝带反馈**：计划落盘（未批准）；反馈经 user_message 事件落库（via=plan_feedback、planMode=true）并注入本 run 历史（assistant tool_use → tool_result 桥接说明 → user 反馈的 provider 可见顺序）；工具结果只给桥接说明；run 继续迭代，模型修订后重新提交。
3. **拒绝无反馈**（含关闭卡片 / 审批表单直接拒绝）：计划落盘（未批准）；工具结果为拒绝；run 按正常完成收尾（status=done，不是 canceled/error）——会话保持计划态，等用户输入修改意见、重新勾选执行或忽略。

审批卡动作：批准并开始实施 / 提出修改（展开输入框，反馈必填非空）/ 忽略（无反馈拒绝）。

- 反馈的持久化本体是用户消息；approvals 表不加列，resolved 事件与审计只带长度摘要/前缀。
- 审批取消（run 取消、窗口关闭、应用退出）沿用既有取消链：按取消落审计，run canceled。
- 防自循环：拒绝不把失败喂回模型自行重试（ZCode 2026-10 修复的自循环问题）；无反馈拒绝终止轮次，带反馈必须经真实用户消息。

## 计划态只读闸门（D8 修订）

- 判定链位置：模式可用性之后、档位判定之前。
- 规则：plan_active 且风险 ≥ R2（执行/网络越界/破坏性）→ 直接拒绝，**自由档也不例外**；R0/R1 照走原档位判定（分级档该弹卡弹卡、自由档放行）。
- 闸门只拦不放：任何放行都必须能追溯到原档位判定。
- 协议工具（enter_plan / exit_plan）不进闸门（在更早的特判分支处理）。

## 计划文件与压缩续读

- 写入：exit_plan 审批决定时刻（批准与拒绝都落盘），内容为计划原文；原子写、0600。
- 文件名：`plan_<plans.id 剥前缀>.md`；finalize 兜底路径与显式路径一致。
- compact 注入：压缩完成后，若会话最新计划行 approved=1 且文件可读，把计划全文（8000 字符截断保头部）拼在保留窗**最前**、摘要之前。
- 跨 run 注入：计划 run 结束后经提示条「批准并执行」发起的新 run（历史里只有固定确认文本），run 启动把计划全文注入历史头部。计划模式主路径（批准同 run 继续）不注入。
- 提示条启发式落盘（run 最后一段非空 text）保留：模型未调 exit_plan 的计划 run，行为与旧版一致。

## 前端合同

- 勾选态：会话级（threadComposer 分桶）；plan.entered 定向开启，exit_plan 批准结果定向关闭，拒绝/反馈不动；重启不回放（plan.entered 不入库）。
- 计划审阅卡：以 `tool === "exit_plan"` 识别；三动作见上。
- plan_feedback 用户消息渲染为普通用户气泡（无「待送达」标记——via 只匹配 "steer"）。
- 提示条（兜底）：「计划已生成」条件不变（file_path 非空、未批准、未忽略、run 不在跑）；按钮为查看计划 / 提出修改（置勾选 + 聚焦输入框）/ 忽略 / 批准并执行（发 planMode=false 固定确认消息，走跨 run 注入路径）。

## 测试要点与现状

- 工具层（craft-tools）：spec 元数据、进入引导、原文保留、缺参/空白/超长拒绝、Registry 接缝。
- 提示词（craft-agent/prompt.rs）：计划段注入 exit_plan 引导、直接拒绝文案、非计划态不注入。
- 存储层（craft-store/plans.rs）：计划行生命周期、精确 id 查询。
- 待补（后续提交）：run 层三路径集成测试、闸门自由档拒绝测试、compact 注入集成测试。
