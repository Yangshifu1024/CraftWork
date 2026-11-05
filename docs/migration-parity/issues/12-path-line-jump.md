# 12: path:line 行内引用跳转

**labels:** [needs-triage]
**对应对账:** spec.md §二 L6；依赖票据 10（编辑器 tab 作为跳转目标）

**What:** 恢复 markdownRef 逻辑（旧 1e53511 `markdownRef.ts`/mygo `mdref.go`）：markdown 输出中的 `path:line` 三形态（裸文本/空链接/反引号）识别为可点击引用，点击打开编辑器 tab 并定位行。系统提示词已在引导模型使用该格式（craft-agent/src/prompt.rs:20），只差消费端。

## 进展

