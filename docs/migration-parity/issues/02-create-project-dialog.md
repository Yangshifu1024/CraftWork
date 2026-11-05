# 2: 创建项目对话框与 AddProject 语义补齐

**labels:** [ready-for-agent]
**对应对账:** spec.md §二 L1、§一 D5(workspaceAddProject)

**What to build:** 恢复 ZCode 式创建项目交互（旧实现参考 `git show 9ca40f6:frontend/src/features/overlays/CreateProjectDialog.tsx`、后端语义参考 `git show adcebf5~1:internal/service/workspace.go` AddProject）。

**后端** `services.rs workspace_add_project`：
- name 空 → 从 `paths[0]` 叶子名推导；与既有项目重名 → 自动加序号
- 目录去重：请求内去重；目录已是任何项目的主/附加目录 → 可读错误
- 内置目录守卫：拒绝数据目录与内置临时会话目录（`data_dir/workspace/*`）作为项目目录
- trust 白名单 `trusted|restricted`；返回单 WorkspaceView（对齐 channels 声明）
- cargo test：空名推导、重名序号、重复目录拒绝、内置目录拒绝

**前端**：
- `store/overlays.ts` 加 `createProject: boolean` + open/close
- 新建 `overlays/create-project.tsx`：radix Dialog 两视图——表单（名称 input，placeholder「新项目」，未编辑提交空串；设备「此电脑」静态行；源文件夹空态虚线大按钮 → `api.pickFolder()`，列表行「主要」徽标 + 移除 + 添加行；取消/创建）→ 信任确认视图（「信任此文件夹吗？/信任这些文件夹吗？」+ 全路径列表 + 风险说明 + 取消/「信任文件夹并创建」；焦点落取消；Esc 回表单——`onEscapeKeyDown` 分视图；IME 守卫复用 `lib/ime.ts`）
- 提交 `workspaceAddProject(nameEdited?name.trim():"", folders, "trusted")` → close → `refreshWorkspaces()` → `newDraft(ws.id)` → ok toast；失败回表单保留输入 + danger toast
- `App.tsx` 挂载；`sidebar.tsx createProject` 改为 `openCreateProject()`，按钮 title 改「新建项目」；`shortcuts.ts` dialogOpen 纳入 createProject

## 验收标准

- [ ] 空名创建：项目名 = 主要目录叶子名；同目录再建被拒；同名项目自动「name 2」
- [ ] 多文件夹：首个带「主要」徽标，可增删；信任确认两视图、Esc 行为正确
- [ ] 创建成功后侧栏出现项目分组并进入其草稿态；选数据目录/临时目录被拒且有 toast
- [ ] cargo test 四例通过；`pnpm typecheck` 通过；深浅主题走查

## 进展

