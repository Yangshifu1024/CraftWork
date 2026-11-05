# 10: 面板 IDE 化恢复（多标签页、编辑器、差异视图）

**labels:** [needs-triage]
**对应对账:** spec.md §二 L2/L3

**What:** 面板从「文件树 + 死桩文档预览」恢复 Gen1 的 IDE 形态：files/file/diff/document 多类 tab 并存（`workspaceReadFile/WriteFile/ComputeDiff` 通道齐备但零调用）、编辑器一等 tab（读写 + expectedHash 外部修改确认，后端防线已在）、diff 视图（审批「查看完整差异」入口同步恢复）、tab 保活与偏好持久化、面板分隔条（宽度可拖/持久化，替换 App.tsx:53 纯装饰假把手）、面板快捷键（⌘W 关 tab 等）、文件树多文件夹分段与「主要」徽标、文档预览死桩（DocView setText 从未被调用）接活或移除。旧实现参考 `git show 9ca40f6:frontend/src/features/panel/`。决策点：编辑器选型（CodeMirror 6 回归 vs 保持轻量只读）需先定。

## 进展

