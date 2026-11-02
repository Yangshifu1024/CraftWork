# Triage 标签

本仓库使用默认的五个分诊角色，标签字符串与角色名一致：

| 角色 | 标签字符串 | 含义 |
| --- | --- | --- |
| 待分诊 | `needs-triage` | 尚未判断有效性、优先级与归属 |
| 缺信息 | `needs-info` | 缺少复现步骤或决策输入，需要补充信息 |
| 可交给代理 | `ready-for-agent` | 信息完整，Agent 可以直接开工 |
| 需要人做 | `ready-for-human` | 需要人来执行或决策，Agent 不应接手 |
| 不修 | `wontfix` | 明确不处理 |

Issue tracker 为仓库内 markdown（见 `issue-tracker.md`），状态写在 issue 文件的 `Status:` 行，值为上表中的标签字符串。不创建任何外部标签。
