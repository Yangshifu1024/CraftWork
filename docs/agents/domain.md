# Domain Docs

工程类技能在探索本仓库时应如何消费领域文档。

## 探索前先读

- 仓库根的 **`GLOSSARY.md`**（单上下文布局，本仓库无 `GLOSSARY-MAP.md`）。
- **`docs/adr/`**：阅读与你即将改动的区域相关的决策记录（ADR）。

如果这些文件不存在，**静默继续**：不要提示缺失，也不要主动建议创建。`/domain-modeling` 技能（经 `/grill-with-docs` 与 `/improve-codebase-architecture` 触达）会在术语或决策真正落定的时候按需创建它们。

## 文件结构

本仓库为单上下文布局：

```
/
├── GLOSSARY.md
├── docs/
│   ├── adr/          架构决策记录，NNNN-短横线标题.md
│   ├── agents/       工程技能的仓库配置（本目录）
│   └── specs/        行为规格：product / features / architecture / interface / prompt-contract
└── frontend/、internal/ 等
```

## 使用术语表的词汇

当你的输出需要指称领域概念（issue 标题、重构提案、假设、测试名）时，使用 `GLOSSARY.md` 中定义的术语。不要漂移到术语表明确避免的同义词。

如果需要的概念还不在术语表里，这是一个信号：要么你在发明项目不使用的语言（请重新斟酌），要么存在真实的空缺（记录下来交给 `/domain-modeling`）。

## 标记与 ADR 的冲突

如果你的输出与某个既有 ADR 矛盾，显式指出，而不是静默推翻：

> _与 ADR-0007（SQLite 事件溯源）矛盾，但值得重开讨论，因为……_

## specs 与 ADR 的分工

- `docs/specs/` 描述**系统当前的行为规格**（是什么）：功能与验收、架构契约、界面规格、提示词契约。
- `docs/adr/` 记录**为什么是这样**：背景、决策、后果、被否决的备选。
- 改变现有行为时，先查对应 spec 与 ADR；推翻既有决策需要新写一份 ADR 并标记旧 ADR 为 superseded，同时更新受影响的 spec。
