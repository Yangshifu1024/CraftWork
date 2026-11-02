# Issue tracker：仓库内 markdown

本仓库的 issue 与 spec 以 markdown 文件形式存放在 `docs/<feature>/` 下，不使用 GitHub Issues 或其他外部跟踪系统。

## 约定

- 一个特性一个目录：`docs/<feature-slug>/`，slug 用小写连字符（例如 `docs/mcp-http/`）。
- 该特性的规格是 `docs/<feature-slug>/spec.md`。
- 实现 issue 一票一个文件：`docs/<feature-slug>/issues/<NN>-<slug>.md`，编号从 `01` 起，不要合并成一个总票文件。
- 分诊状态写在每个 issue 文件顶部附近的 `Status:` 行（角色字符串见 `triage-labels.md`）。
- 评论与讨论历史追加到文件底部 `## Comments` 标题之下。

## 当技能说「发布到 issue tracker」

在 `docs/<feature-slug>/` 下新建文件（目录不存在则先创建）。

## 当技能说「读取相关票」

读取被引用路径的文件。用户通常会直接给出路径或编号。

## Wayfinder 操作

供 `/wayfinder` 使用。**map** 是一个文件，每张子票一个**子文件**。

- **Map**：`docs/<effort>/map.md`（Notes / Decisions-so-far / Fog 正文）。
- **子票**：`docs/<effort>/issues/NN-<slug>.md`，编号从 `01` 起，正文写问题。`Type:` 行记录票型（`research`/`prototype`/`grilling`/`task`）；`Status:` 行记录 `claimed`/`resolved`。
- **阻塞**：文件顶部 `Blocked by: NN, NN` 行。所列文件全部 `resolved` 时该票解除阻塞。
- **前沿**：扫描 `docs/<effort>/issues/` 中开放、未阻塞、未认领的文件；编号最小者优先。
- **认领**：动手前先置 `Status: claimed` 并保存。
- **解决**：在 `## Answer` 标题下追加答案，置 `Status: resolved`，并在 map 的 Decisions-so-far 追加上下文指针（摘要 + 链接）。
