# 04. 编辑器升一等 tab

Status: ready-for-agent

## 背景

现状编辑器是 FilesPanel 的子状态（`selected` 非空时整棵树被 `<Editor />` 替换，`FilesPanel.tsx:72-74`），单文件打开、树与编辑器互斥。多 tab 模型下文件树是**导航器**、编辑器是**内容 tab**，二者解耦（[spec.md](../spec.md) 第 1 节、proposal 4.3 第 5 条）。

## 改动

- `frontend/src/features/panel/FilesPanel.tsx`：移除 `selected` 非空整树替换逻辑；文件树常驻为面板内容的一部分。
- 编辑器渲染迁移到 tab 内容层：`file` tab 激活时渲染 `<Editor />`，多文件 = 多 `file` tab 并存；编辑器 tab 关闭即露出文件树。
- `stores/workspace.ts` 的 `selected` / `content` / `draft` / `targetLine` 从单值槽位改为**按 file tab id 分槽**（每打开文件一份草稿与滚动状态）；`openFileTab` 负责建槽，关闭 tab 清槽。`draft` 写回 / `expectedHash` 校验 / 备份流程逐文件独立，语义不变。
- 编辑器内部功能随 tab 迁移保持：只读/编辑切换、CM 搜索、复制、`targetLine` 跳转（`openFileAt` 打开时定位）、二进制/截断占位。
- 单测：多文件草稿互不覆盖；关闭 tab 清槽后重开内容一致；`openFileAt` 定位到正确 tab 与行。

## 验收

- 文件树可见 + 多文件 tab 并存；A/B 文件来回切换草稿与光标各自保留。
- 审批卡片「查看完整差异 → 在编辑器中打开」、聊天正文文件引用均落到正确 tab 与行。
- 前端四项检查通过；1440×900 与 1100×800 走查布局不破。
- 文档同步：`docs/specs/interface.md`（面板结构变化）、`docs/specs/features.md`（编辑器入口行为）。

## 依赖

01、02、03。

## Comments

## Comments

- 已实现于 `e6722a9`（分支 feature/panel-tabs）。验证：前端 lint / tsc / vitest / build 全绿。
