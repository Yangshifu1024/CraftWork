# 8: 文档同步与全量走查

**labels:** [ready-for-agent]
**status:** done（余 GUI 与真实密钥人工验收）
**Blocked by:** 1、2、3、4、5、6、7（全部前置票）

**What to build:** 收尾与验收。文档同步：GLOSSARY 补术语（供应商启停、模型启停、拉取模型）、README 更新模型设置相关功能与架构描述。全量走查（隔离目录）：开箱主线「导入 DeepSeek → 端点已预填 → 仅配密钥 → 测试连接 → 拉取模型 → 对话可用」；引导线「导入无端点提供商 → 目录徽标 + 详情必填态 → 手填端点后可用」；全操作线（新增/编辑/删除模型、两级启停、改名、删除供应商）；深浅双主题、窗口缩窄不破版。四件套验证。

## 验收标准

- [x] GLOSSARY / README 与实现一致
- [ ] 开箱主线走查通过：导入即有端点，仅配密钥即可对话（静态确认：快照 229 商、导入即预填端点；真实密钥对话闭环待人工验收）
- [ ] 无端点引导线、全操作线走查通过（静态确认：空端点徽标/必填态、增删改/启停/改名/删除全链路代码接线 + 单测；GUI 实操待人工验收）
- [ ] 深浅主题、缩窗走查通过（静态确认：新组件零写死色值、全走语义 token；视觉走查待人工验收）
- [x] `pnpm typecheck && pnpm test && pnpm build && pnpm dist:dir` 全部通过

## 进展

- **文档同步**：GLOSSARY「模型与提供商」表新增 3 条术语（提供商启停、模型启停、拉取模型）并更新「模型目录」条目（logo 内嵌、端点预填）；README 更新「对话与工具」两条（模型目录、模型管理新增一条：主从两栏、两级启停、增删改、拉取模型、选择器联动）与「快速开始 → 模型目录」段（导入预填、拉取清单）。术语名取「提供商启停」对齐词汇基准（表内既有「提供商」，避免「供应商」同义词漂移）。
- **四件套**：worktree 依赖缺失，先 `pnpm install --frozen-lockfile`（1.9s）+ electron 二进制补装（未新增/升级依赖）；`pnpm typecheck` 通过（node + web 双 tsconfig）；`pnpm test` 13 文件 165 用例全过；`pnpm build` 三段产物成功；`pnpm dist:dir` 产出 `dist/mac-arm64/CraftWork.app`。
- **隔离冒烟**：`CRAFTWORK_HOME=/tmp/cw-ticket08 pnpm dev` 启动成功——main/preload 构建、renderer dev server 5173、主进程 + GPU + network + renderer 进程齐全；数据目录建立，`craftwork.db` integrity ok；日志仅 Chromium /tmp 磁盘缓存噪音，无领域异常；验证后杀进程并清理隔离目录。
- **静态走查**（对照 spec §14 与票 1–7 验收）：7 个增量通道 `channels.ts` / `preload/index.ts` / `ipc.ts` 三处同步无缺漏；`catalogLogo` 全仓无残留；`provider-logos.ts` 229 个 logo（生成物头部标注勿手改）；modelsync `api/baseUrl` 兼容 + attachment 语义反转 + SVG 清洗（script/foreignObject/on*/javascript:/外部引用）+ 覆盖表扩充；enabled 缺省迁移、服务端「读→校验→写→reload→toView」、factory 启停拦截与转移跳过、选择器过滤（`selectableModels`）均有代码与单测；主从布局（`grid-cols-[224px]`/`min-h-[32rem]`/`max-w-4xl`）、左栏三色状态点与来源分组、详情卡（内联改名过 IME、空端点必填态、测试连接内联 loading）、模型行徽章/增改弹窗/删除 danger 确认、拉取 loading 防重复与内联反馈、添加页两张入口卡与「需填写端点」徽标、无写死色值——全部落实。
- **待人工验收**（需真实密钥 / GUI 实操）：开箱主线「导入 DeepSeek → 配密钥 → 测试连接 → 拉取 → 对话」、无端点引导线与全操作线 GUI 实操、深浅双主题与缩窗视觉走查、断网状态无网络请求实测（静态论证：logo 为 data URL 内嵌、渲染层无远程加载）。
