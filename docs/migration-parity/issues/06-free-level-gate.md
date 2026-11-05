# 6: 自由档确认门复原（能力清单 + 勾选确认）

**labels:** [ready-for-agent]
**对应对账:** spec.md §二 L11；旧实现 `git show e9880ae` （FreeLevelDialog）

**What to build:**

- `store/overlays.ts` 加 `freeLevel: { onOk(): void } | null` + open/close（仿 confirm 单例：新请求覆盖旧请求）
- `overlays/dialogs.tsx` 新增 FreeLevelDialog：警示标题「要启用完全访问吗？」+ 三类能力清单卡片（lucide Folder/Terminal/Globe：文件和文件夹 / 终端命令 / 网络和应用）+ 风险提示 + 「我已了解风险」勾选门——未勾选：确认禁用、Enter 不生效；勾选后确认按钮用填充式危险色；Esc 关闭
- `lib/approval.ts` `confirmThenSave` 与 `cycleApprovalLevel`（⌘⇧M）改走 freeLevel 门
- `hooks/shortcuts.ts` dialogOpen 判定纳入 freeLevel

## 验收标准

- [ ] composer 档位 chip / 设置页 / ⌘⇧M 三入口切「自由」都出能力清单勾选门
- [ ] 未勾选时确认禁用、Enter 无效；Esc/取消不切换
- [ ] 勾选确认后档位即时切换（依赖票据 04 的热切换）并写审计
- [ ] 深浅主题走查；`pnpm typecheck` 通过

## 进展
- 2026-11-05：完成（提交 0efd499）。FreeLevelDialog 落地（能力清单三卡 + 勾选门 + Enter 不直通 + 每次打开重置）；三入口统一走确认门；shortcuts dialogOpen 纳入；深浅主题走查待实机
