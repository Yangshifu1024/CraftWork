# Spec：模型设置界面与交互重构（参考 ZCode）

状态：已实现（票 1–8 完成并通过双轴 code-review；GUI 深交互与真实密钥闭环待人工验收，见票 08 走查记录）
日期：2026-11-04（r2：补充数据层修复——logo 本地化与端点直连）
关联：`GLOSSARY.md`（模型与提供商）、`AGENTS.md` §5/§6、`docs/agents/issue-tracker.md`

## 0. 数据层事实源问题（前置修复，2026-11-04 实测）

对照 models.dev 远端 `api.json`（235 提供商）与当前快照（193 提供商）发现两个脚本缺陷，导致「logo 显示」与「端点直连」两个诉求现在都不成立：

| # | 问题 | 根因 | 影响 |
| --- | --- | --- | --- |
| D1 | 快照丢 36 个提供商（含 anthropic、xai） | models.dev 的 `attachment` 字段语义已反转：现在 `true` = 支持附件输入的**对话模型**（gpt-5.2、claude 全系均为 true），`false` 才是嵌入类；`modelsync.mjs:44` 仍按旧语义过滤，把这些提供商的全部模型滤空后整商丢弃 | 目录里没有 Anthropic / xAI 官方可选 |
| D2 | 快照 193 个提供商中 182 个端点为空 | models.dev 已把 `baseUrl` 字段改名 `api`；`modelsync.mjs:75` 仍读 `provider.baseUrl`，恒为 `undefined`，只剩 `endpointOverrides` 13 项兜底 | 导入后必须手填端点，「开箱即用」不成立 |

远端实测数据（2026-11-04）：

- 端点（`api` 字段）形态：以 `/v1`~`/v3` 结尾 199 个；无路径纯域名 2 个（deepseek 等，官方声明两种路径等价，仍走覆盖表取 `/v1`）；其他真路径 9 个（`/openai`、`/v1/solar` 等兼容网关，原值可用）；含 `${VAR}` 占位符坏值 1 个（neon，需置空）；无端点 19 个（其中 13 个已被 `endpointOverrides` 覆盖）。
- 客户端拼接契约（决定端点形态）：OpenAI 系 `endpoint + "/chat/completions"`（`openai.ts:31`），端点需含 `/v1` 级路径；Anthropic `endpoint + "/v1/messages"`（`anthropic.ts:139`），端点不含 `/v1`。修复后约 **208/229（91%）提供商导入即有可用端点**。
- logo：`https://models.dev/logos/{id}.svg`，实测 192/193 可下载（200），合计 458KB 原始 SVG，最大单个 78KB。

## 1. 背景与目标

参考 ZCode 的「模型设置」页（`packages/ui/src/settings/model-provider-section/`），重构 CraftWork 设置中心的「模型与提供商」分区，并修复 §0 的数据层缺陷：

- 界面：现状单列手风琴卡片改为**主从两栏**（左列表 + 右详情），对齐 ZCode 的信息密度与交互语言。
- 数据：快照重新生成后提供商/模型目录恢复完整，logo 离线内嵌、端点开箱即用。

不做的事（与 ZCode 的差异边界）：

- 不引入账号/订阅体系（套餐卡、额度进度条、续费/解绑、OAuth 连接方式全部不适用）。
- 不做供应商/模型拖拽排序。
- 不做 revision 乐观锁与稀疏 overlay（模型级操作走服务端增量 API，mutation 返回权威 View）。
- 运行时不加载任何远程资源：logo 全部构建期内嵌（见 §2），渲染层无网络请求。

## 2. logo 本地化与显示

原则：渲染层不加载远程资源（CSP/离线红线），logo 在 **modelsync 构建期下载并内嵌**。

### 2.1 生成侧（modelsync.mjs 扩展）

1. 拉取目录后，对每个提供商下载 `https://models.dev/logos/{id}.svg`（保留快照已有的 200 实测结论；失败的不收录，回退字母头像）。
2. **SVG 清洗**后落盘 `assets/catalog/logos/{id}.svg`：
   - 剔除 `<script>`、`<foreignObject>`、事件属性（`on*`）、`javascript:` 引用、外部 `xlink:href`/`url()` 引用；
   - 仅收纳 `image/svg+xml` 响应；单文件 > 100KB 丢弃（防异常体积）。
3. 生成的快照 zip 不变（logo 不进 `catalog.json.gz`），另生成渲染层资产模块：
   - `src/renderer/src/assets/provider-logos.ts`：`export const PROVIDER_LOGOS: Record<string, string>`（providerId → `data:image/svg+xml;base64,…`）。
   - 以 data URL 而非文件 URL：无路径解析问题（file:// 相对路径已由 `base: "./"` 保障，但 data URL 免去按 id 找文件的间接层，也更符合「内嵌」既有风格）。
   - 全量 192 个约 458KB 原始 / ~610KB base64，构建体积可接受；`provider-logos.ts` 是生成物，头部标注勿手改。

### 2.2 消费侧（渲染层）

- 新组件 `ProviderLogo`（对齐 ZCode 同名组件）：`PROVIDER_LOGOS[id]` 命中 → `<img src=dataURL>`（`<img>` 上下文中 SVG 脚本不执行，双重保险靠 §2.1 清洗）；未命中 → 字母头像（首字符 + `bg-tag` 圆角方块）。
- 使用位置：左栏列表项（size-4）、右栏详情头部（size-6）、添加页目录列表（复用）。
- 通道表 **删除 `catalogLogo`**（现留桩返回 `""`，三处接线一并移除），logo 由渲染层直接消费生成模块，无 IPC 往返。

## 3. 端点开箱即用

### 3.1 生成侧（modelsync.mjs 修复）

1. 字段兼容：`endpoint = ENDPOINT_OVERRIDES[id] ?? provider.api ?? provider.baseUrl ?? ""`（新字段优先，旧字段兜底）。
2. 坏值防御：端点含 `${` / 非 http(s) / URL 解析失败 → 置空，按「无端点」处理。
3. 覆盖表扩充（`rules.json` `endpointOverrides`）：为无 `api` 字段且端点公开固定的服务补齐——本期至少补 `salad-cloud`、`aihubmix`、`gitlab`、`vercel`、`watsonx`、`sap-ai-core` 中可确认官方地址者；其余（cloudflare-ai-gateway、qvac 等网关/私有网关）保持空，走 UI 引导。
4. D1 修复后重新生成快照：提供商 193 → 约 229，提交 `.json.gz` + manifest + `catalogdata.ts` + `logos/` + `provider-logos.ts`。

### 3.2 消费侧（UI）

- 目录浏览器（CatalogBrowser）提供商行：端点为空时显示 warning 徽标「需填写端点」，右侧模型勾选与导入按钮保持可用（导入后详情卡端点区高亮必填）。
- 详情卡端点区：空端点显示必填态（warning 条「请填写服务端点后才能调用」），输入框聚焦；有效后 warning 消失。
- 请求前兜底保持现状（`openai.ts:114` / `anthropic.ts:136` 的 `missingEndpointError`）。

## 4. 布局

```
┌─ 设置浮层 ──────────────────────────────────────────────┐
│ 模型与提供商                                    [刷新][+ 添加提供商] │
│ 管理模型供应商与模型，配置后在聊天中选择使用。                      │
│ ┌──────────────┬──────────────────────────────────────┐ │
│ │ 供应商        │ [logo] DeepSeek             [启用 ⬤] │ │
│ │ ─ 目录导入 ─  │ openai-compatible · 3 个模型    [⋯]  │ │
│ │ ▣ DeepSeek ● │ ┌ 连接 ────────────────────────────┐ │ │
│ │ ▣ GLM     ●  │ │ 端点  https://api.deepseek.com… ✎ │ │ │
│ │ ─ 手动添加 ─  │ │ 密钥  sk-···abcd  [设置][清除]    │ │ │
│ │ □ 本地网关 ●  │ │ [测试连接]  ✓ 连接成功 · 230ms     │ │ │
│ │              │ └──────────────────────────────────┘ │ │
│ │              │ 模型列表            [拉取模型][+ 添加] │ │
│ │              │ │ deepseek-chat    128K      🗑  ⬤ │ │
│ │              │ │ deepseek-reason  128K 视觉  🗑  ⬤ │ │
│ │              │ └──────────────────────────────────┘ │ │
│ └──────────────┴──────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

- 顶部行：标题沿用分区标题；副标题一句话说明；右侧「刷新」+「添加提供商」主按钮。
- 主体一张 `rounded-xl border bg-panel overflow-clip` 卡片，grid `grid-cols-[224px_minmax(0,1fr)]`、`min-h-[32rem]`；providers 分区容器放宽到 `max-w-4xl`，其余分区不变。

## 5. 左栏：供应商列表

- **按 `source` 分两组**：「目录导入」（catalog）/「手动添加」（manual），组标题常显、空组隐藏（ZCode 的「智谱」组是账号品牌入口，CraftWork 的对应语义就是来源）。
- 每项：`ProviderLogo`（或字母头像）+ 名称 truncate + 右侧状态点。
- 三色状态点：灰 = `enabled === false`；绿 = 启用且密钥就绪（或无需密钥）且有启用模型；橙 = 启用但缺密钥/无启用模型。带 `title`。
- 选中项 `bg-selected` + `aria-selected`；选中态存组件 state。

## 6. 右栏：供应商详情

- **头部**：logo + 名称内联编辑（Enter/Esc + IME composing 检查）+ 协议徽章 + 模型数 + 供应商启用开关 + ⋯菜单（测试连接 / 删除）。
- **连接配置卡**：端点（常显等宽输入，blur/Enter 保存，服务端校验错误内联红字，空值必填态见 §3.2）；协议（创建后只读）；密钥（`keyMasked` + 明文切换 + 设置/清除[confirm]，缺密钥 warning 条）；测试连接（按钮 + 内联结果：绿字+延迟 / 红字+归因文案，复用 `CONNECTION_MESSAGES`）。
- **模型列表区**：标题 + [拉取模型] + [+ 添加模型]；每行 = 名称 + ID（等宽小字）+ 上下文徽章（Intl compact：`128K`/`1M`，title 完整数）+ 能力徽章（视觉/推理）+ 编辑/删除[confirm]/启用开关；行内错误标配置不完整；空态虚线框引导。

## 7. 关键流程

- **添加提供商**：右上按钮 → 右栏切换「添加」页（带返回），两张入口卡：① 从模型目录导入（`CatalogBrowser` 移入右栏，logo 上墙、空端点徽标）② 手动创建（右栏内嵌表单：名称/协议/端点/密钥/首个模型）。对齐 ZCode「创建即编辑」。
- **拉取模型**：新增 `providerFetchModels(providerId)`，按协议 GET `/models` 或 `/v1/models?limit=1000`，按 ID 去重合并，新增默认启用、`source: "fetched"`；返回 `{ fetched, added, models }`，内联反馈。
- **模型增改弹窗**：ID（catalog 来源只读）/显示名/上下文 K/最大输出/能力勾选；保存成功才关窗、失败内联标错、保存中锁正文。
- **删除**：模型与供应商均 `openConfirm` danger（供应商删除提示连带密钥与模型数）。

## 8. 数据模型与契约变更

- **持久化**（`catalog.ts`）：`PersistedProvider`、`PersistedModel` 各加 `enabled: boolean`（旧数据缺省读为 `true`）；`PersistedModel.source` 新增取值 `"fetched"`。
- **共享类型**：`ProviderView` + `enabled` / `source` / `docUrl`；`ModelView` + `enabled`；新增 `FetchedModelsResult`；删除 `CraftworkApi.catalogLogo`。
- **通道表**（channels/preload/ipc 三处同步）新增 7 个方法，全部返回刷新后的 `ProviderView`：

  `providerSetEnabled` / `providerSetModelEnabled` / `providerAddModel` / `providerUpdateModel` / `providerRemoveModel` / `providerRename` / `providerFetchModels`

- **服务端**：增量方法统一「读→校验（复用 `buildModel` 规则）→写→`catalog.reload()`→返回 `toView`」。

## 9. 联动语义

- `model-picker` 只列 `provider.enabled && model.enabled` 的模型；分组标题仅提供商名。
- 历史会话引用不因禁用而清除，仍可解析显示；调用时 `factory.client` 对禁用项抛「该模型已停用，请重新选择模型」。
- 设置默认模型（default/fallback）本期不动。

## 10. 交互细节清单（对齐 ZCode）

- Enter/Escape 一律过 IME composing 检查；破坏性操作一律 `openConfirm` danger。
- 测试连接/拉取模型进行中 loading + 禁用防重复。
- 反馈优先内联展示，toast 只用于跨区通知。
- 语义色 token 全量使用，不写死色值；lucide 图标按需导入；`prefers-reduced-motion` 降级。

## 11. 空态与引导

- 无任何提供商：右栏引导卡（① 添加提供商 → ② 配置 API 密钥 → ③ 回聊天选模型）+ 两个入口按钮。
- 导入后未配密钥：warning 条 + [设置密钥] 高亮；端点为空：必填态（§3.2）。

## 12. 测试

- `modelsync`：本仓脚本无测试基建，以「重新生成 + diff 审查」验证：提供商数恢复 ~229、anthropic/xai 回归且端点正确、嵌入模型（text-embedding-3-small）不进目录、logo 192 个清洗后无 `<script`。
- `tests/`（vitest）扩展：
  - enabled 缺省迁移；`providerSetEnabled` / `providerSetModelEnabled` / 增量模型方法（重名、不存在报错）；
  - `providerFetchModels` 解析与去重（mock fetch：openai/anthropic、HTTP 错误、超时）；
  - `toView` 新字段；endpoint 空值校验路径。
- 渲染层不强制组件测试，走隔离目录冒烟走查。

## 13. 分期（issues）

| # | 内容 | 层 |
| --- | --- | --- |
| 0 | modelsync 修复：`api` 字段兼容、attachment 过滤反转、坏值防御、覆盖表扩充、logo 下载/清洗/内嵌、快照重新生成 | scripts/assets |
| 1 | 数据模型 + 7 个服务方法 + fetchModels + 单测（含 catalogLogo 通道移除） | main |
| 2 | 主从布局重构 + 左栏列表（logo/状态点/分组）+ 空态引导 | renderer |
| 3 | 供应商详情卡（连接区/启停/改名/删除/测试连接内联反馈/空端点必填态） | renderer |
| 4 | 模型列表管理（行/徽章/启停/增改弹窗/删除/拉取 UI） | renderer |
| 5 | 添加提供商入口（目录导入入右栏 + 手动创建表单） | renderer |
| 6 | 模型选择器过滤联动 + factory 禁用语义 | main+renderer |
| 7 | GLOSSARY/README 同步 + 全量走查（隔离目录、双主题、缩窗） | docs |

## 14. 验收走查

1. `node scripts/modelsync.mjs` 重新生成，diff 审查（提供商数、anthropic 端点、logo 模块体积）。
2. `CRAFTWORK_HOME=/tmp/cw-model pnpm dev` 隔离冒烟。
3. 目录导入 DeepSeek → 端点已预填 → 仅配密钥 → 测试连接成功 → 拉取模型 → 对话可用（「开箱即用」主线）。
4. 导入无端点提供商（如 cloudflare-ai-gateway）→ 目录徽标 + 详情必填态引导。
5. logo 走查：目录导入组显示真 logo，手动添加组字母头像，断网状态无网络请求。
6. 其余：增删改模型、启停联动选择器、删除确认、双主题、缩窗。
7. `pnpm typecheck && pnpm test && pnpm build && pnpm dist:dir`。
