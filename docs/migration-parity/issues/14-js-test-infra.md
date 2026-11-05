# 14: JS 测试基建恢复（Vitest）

**labels:** [needs-triage]
**对应对账:** spec.md §二 L8

**What:** 渲染层零测试基建（无 vitest/jsdom、无 *.test.*，lib/*.ts 注释仍称「由 vitest 直测」）。恢复最小 Vitest：纯函数层先行（lib/paste.ts、mergeStreamedMessages、drafts、theme、approval、format-context、model-form 等），不引 jsdom/UI 测试；可选 Playwright 截图回归作为远期。需新增 devDependencies（vitest），走依赖审批。

## 进展

