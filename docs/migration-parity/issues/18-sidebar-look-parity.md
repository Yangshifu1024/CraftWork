# 18: 侧栏与观感回潮项

**labels:** [needs-triage]
**对应对账:** spec.md §二 L14

**What:** mygo 定稿的观感在 Electron 重写期部分回潮，逐项对照恢复或重新拍板：

- 空会话引导回潮：9653d05/374f6b4 定稿是空会话完全留白聚焦输入壳，当前渲染「开始你的第一条消息」（conversation.tsx:105-108）
- 正文列宽：802a2a5「恒定 80% 居中」用户观感约定，当前恒 max-w-3xl(768px)
- composer：8dd3e54 的 + 动作菜单（当前 Plus 直接触发文件选择）、列宽断点（已被 80% 约定取代，拍板即可）
- 侧栏：组头计数徽章（e74daa9）、折叠状态持久化（旧 localStorage 键语义，当前重启丢失）、分组折叠展开行过渡（bb2e3b8 的 Motion{Collapse}，当前瞬时条件渲染）
- 跑马灯 hover 1 秒延时契约（overflow-title.tsx 注释称 1s 后滚动，实际立即滚）
- under-window 材质（4c5d183：macOSPrivateApi 已开未用，--zai-background-win-alt token 零引用；透明+vibrancy 需 tauri windowEffects，注意 76e9e8b 修过最大化黑边，勿回退）
- 组折叠行 Enter/Exit 过渡与 prefers-reduced-motion

## 进展

