# 11: 上下文占用圆环、输出速度与 usage.updated 实时消费

**labels:** [needs-triage]
**对应对账:** spec.md §二 L5

**What:** 恢复 Gen1 ContextRing（composer 角落：SVG 三档色环 + tok/s + 百分比，点击出详情 popover：已用/累计/缓存 token/实测速度）。数据源 `usage.updated` 事件每轮广播（runner.rs:958，载荷含 inputTokens/outputTokens/costMicros/contextTokens/cacheTokens/durationMs/tokensPerSecond）但前端 default 分支丢弃（app.ts:448）——先在 store 接住（按会话累积），再接 UI。`context.compressed` 的水位同样可用于环显示。

## 进展

