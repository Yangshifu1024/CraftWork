# Issue tracker：本地 markdown（docs/<feature>/）

本仓库的 spec 与 issue 以 markdown 文件存放在仓库内，**不使用 GitHub Issues**（remote 虽指向 GitHub，勿用 `gh issue`）。

## 目录约定

- 每个 feature 一个目录：`docs/<feature>/`，目录名用简短 kebab-case 英文（如 `draft-session`）
- `docs/<feature>/spec.md`：该 feature 的行为规格（`/to-spec` 的产物）
- `docs/<feature>/issues/N-<slug>.md`：拆解出的 issue，`N` 为从 1 递增的序号，`slug` 为 kebab-case 短名

## Conventions

- **创建 issue**：在对应 feature 目录下新增 `issues/N-<slug>.md`，文件头部写元信息字段（标题、labels、状态）。
- **标签**：以行内字段记录，如 `labels: [ready-for-agent]`；不用 GitHub 标签系统。
- **读取**：按目录枚举 feature；issue 按文件名序号排序。
- **评论/进展**：以 issue 文件内的 `## 进展` 小节追加记录。

## When a skill says "publish to the issue tracker"

把 spec 写为 `docs/<feature>/spec.md`（`<feature>` 由当次工作确定）。

## When a skill says "fetch the relevant ticket"

读取对应 feature 目录下的 spec.md 或 issue 文件。
