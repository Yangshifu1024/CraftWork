# 8: 跑马灯修复与死 CSS 清理

**labels:** [ready-for-agent]
**对应对账:** spec.md §一 D1、D19、D20

**What to build:**

1. `overflow-title.tsx:50` 轨道 span 补 `overflow-title-track` 基准类——动画选择器 `.overflow-title-track.is-scrolling`（styles.css:488）自迁移后从未匹配，hover 只见两端渐隐不见滚动。
2. 删除死 CSS：`.tool-call-progress-indeterminate`（styles.css:453 起，与 AGENTS 6.4「运行态统一文字扫光」约定冲突且零引用）、`collapsible-up` 孤儿 keyframes（:433 起，未用 radix Collapsible）。
3. `markdown.tsx` 头注释修正为实际行为：streamdown 2.7.1 经 rehype-raw + rehype-sanitize，良性 HTML 渲染、危险内容剥除（非「按文本显示」）。

## 验收标准

- [ ] 超长会话/项目标题 hover：文本去→尾端停顿→返回循环滚动；非 hover 仅右侧渐隐
- [ ] `grep -r "tool-call-progress-indeterminate\|collapsible-up" src/renderer` 无残留且样式无回归
- [ ] `pnpm typecheck`、`pnpm build` 通过

## 进展
- 2026-11-05：完成（提交 a7cee87）。overflow-title-track 基准类补齐（根因：选择器要求的类从未挂载）；死 CSS 两段删除（tool-call-progress-indeterminate + collapsible-up）；markdown 注释改为实际 sanitize 行为；跑马灯滚动实机走查待做
