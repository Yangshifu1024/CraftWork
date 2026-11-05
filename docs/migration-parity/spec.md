# 迁移设计对账（migration-parity）

CraftWork 经历四代形态，每代重写都造成设计丢失或「代码在、接线断」：

1. **Wails3 + Go + React**（`frontend/` + `internal/`，2026-11-01 ~ 11-03）
2. **mygo 原生 Go UI**（`internal/ui`，11-03 晚，已随 Electron 重构消失）
3. **TypeScript + Electron**（`src/renderer` + `src/main`，adcebf5，11-04）
4. **Tauri 2 + Rust crates**（当前，P1~P9）

第 1→3 代（9ab0d16 清 web 轨 + adcebf5 Electron 重写）丢失最多 UI 设计；第 3→4 代（Tauri 迁移）渲染层基本平移，但 Rust 侧服务层出现一批「实现存在、装配/接线缺失」的隐性损坏。

本 spec 是两轮审计的结论汇总：第一轮对照旧版做**存在性**盘点，第二轮（因连续发现「在但坏了」案例）对全部「已平移/还在」项做**接线与行为级**核验——71 条通道三向对拍（声明↔dispatch↔实现↔前端调用）、18 种事件双向对拍、逐特性追「装配 → 运行时调用 → 持久化回载」。

**结论五态**：丢失（当前无对应物）｜改形（有近似物但语义缩水）｜在（完好）｜**损坏（在但接线/行为断，有断点）**｜留桩（决策性未做，AGENTS §10）。

---

## 一、损坏（在但断，本轮修复）

| # | 特性 | 断点 | 症状 | 票据 |
|---|------|------|------|------|
| D1 | 侧栏跑马灯 | `overflow-title.tsx:50` 只挂 `is-scrolling`，动画选择器要求的 `overflow-title-track` 基准类从未挂载（styles.css:488） | 长标题 hover 两端渐隐（滚动态遮罩预期生效）但不滚动 | 08 |
| D2 | documentPreview 通道 | 实现完整（services.rs:430）但 lib.rs dispatch 缺 `"documentPreview"` 分支 → 兜底报错；前端调用处无 catch | 面板点击文件预览必失败且静默 | 01 |
| D3 | catalogImport 参数形状 | 前端传单对象 `{providerId,all,modelIds}`（channels.ts:114、provider-add.tsx:180），Rust 按位置取 3 个标量（lib.rs:241） | 目录导入提供商必失败 | 01 |
| D4 | providerSave 空 ID | 前端传 `id:""` 期望服务端生成 `custom_*`（provider-add.tsx:239），Rust 空串即报错（services.rs:623） | 手动新增提供商必失败 | 01 |
| D5 | 通道返回类型漂移 9 处 | provider 系 8 方法与 workspaceAdd/Rename 声明返回单对象、实现返回数组；chatSteer 声明 string 实返 `{queued,...}` | 当前调用方忽略返回值故无实害，但已在埋断言地雷 | 01 |
| D6 | workspace.updated 事件 | 前端有完整清草稿槽分支（app.ts:277-292），Rust 侧零发射（仅 contract.rs:28 常量） | 删项目后草稿槽在 localStorage 永久残留 | 01 |
| D7 | 审批档位热切换 | `engine.set_level` 零调用点；settings_save 只持久化（services.rs:137-139），引擎仅启动初始化一次 | 切档位需重启才生效 | 04 |
| D8 | 模型失败转移 | 转移逻辑完整（runner.rs:629-667、factory.rs:119-155）但 `set_fallback_resolver` 从未装配 | 设置里 fallbackModel 存而不用，候选链恒单模型 | 04 |
| D9 | 往返上限 | `set_max_iterations_resolver` 从未装配，恒用常量 300 | 设置项「最大工具轮数」不生效 | 04 |
| D10 | 系统通知 | Runner 的 notifier 钩子完整（runner.rs:94/:921-925）但 `set_notifier` 未装配，且无 tauri-plugin-notification 依赖 | 「系统通知」开关无消费方，完成/失败/待审批通知全失效 | 05 |
| D11 | 审批 resolved 不落库 | approvals.rs:195-206 只 `recorder.emit` 不 `recorder.append`，快照回载（craft-store events.rs:245-264）依赖事件行 | 重启/翻页后已决定审批卡回显「等待决定」，再点报「审批已处理」成僵尸卡 | 03 |
| D12 | 重启后待审批卡 | 快照 `pendingApprovals` 字段被前端整体丢弃（app.ts:169-181 不读）；decide 失败仅 console.warn（conversation.tsx:393-399） | 僵尸卡无任何 UI 反馈 | 03 |
| D13 | 审批取消不落审计 | runner.rs:773-776 取消路径提前 return 绕过 add_audit；approvals.decide 无独立审计行；窗口关闭/退出无取消挂钩（lib.rs:258-273） | 违反 AGENTS 6.2「取消、窗口关闭、应用退出统一按拒绝或取消落审计」 | 03 |
| D14 | cancels token 不回收 | runner.rs:250-252 仅 cancelled 才从 cancels 表移除，正常完成/失败的 token 永久残留 | steer 回退分支首跑后不可达、完成边界竞态丢答、chat_cancel 对空闲会话误报 true | 03 |
| D15 | 自动命名写回条件永不成立 | chatCreateAndStart 建会话 title=derive_title 却 title_auto=true（services.rs:500-515），而写回条件要求 title='新对话'（threads.rs:151-159）；前端唯一建会话路径就是 chatCreateAndStart | 模型起的标题永远被丢弃，每个 run 白发一次 sidecar 调用 | 03 |
| D16 | 「已工作」秒数实时路径 | 后端 run 终态事件带 durationMs（runner.rs:872-917），前端归约丢弃（app.ts:311-323）；快照路径已修（events.rs:320-322） | 活会话完成永不显示「持续 X 秒」，重启才出现 | 07 |
| D17 | 界面字号缩放 boot | 唯一 setProperty 在 settings.tsx:194（拖动时），boot 不应用 settings.fontSize（app.ts:111-142） | 重启后恒 14px，滑杆示数与实际不符 | 07 |
| D18 | context_k=0 强压缩 | plan_compression 对窗口 0 无保护（context.rs:54-93）；目录拉取的模型可落 context_k=0（services.rs:1021-1033） | 此类模型每轮把历史压缩到只剩 4 条 | 04 |
| D19 | 死 CSS ×2 | `.tool-call-progress-indeterminate`（styles.css:453）全 TSX 零引用（与「运行态统一文字扫光」约定冲突）；`collapsible-up` 孤儿 keyframes（:433） | 死代码 | 08 |
| D20 | markdown 注释失实 | 注释称「不渲染原始 HTML（按文本显示）」，实际 streamdown 2.7.1 走 rehype-raw+rehype-sanitize：良性 HTML 渲染、危险内容剥除 | 注释误导安全审计 | 08 |

## 二、丢失（大块，后续票据排期）

| # | 设计 | 原位置 | 现状 | 票据 |
|---|------|--------|------|------|
| L1 | 创建项目对话框（名称/设备/源文件夹多选/信任确认两视图） | 9ca40f6 `frontend/src/features/overlays/CreateProjectDialog.tsx`；240c13f 补单测 | 侧栏退化为 pickFolder 直建（且因 D 系空名报错全废）；后端 AddProject 的名称推导/去重/重名序号/内置目录守卫在 Rust 平移时丢失 | 02（本轮） |
| L2 | 面板 IDE 化五类 tab：文件/编辑器（CodeMirror）/差异/终端/文档 + 保活 + 偏好持久化 + 分隔条 + 面板快捷键 + 过窄自动收起 | c3615ea~4405e43；sidePane.ts/Panel.tsx/Editor.tsx | 塌缩为「文件树 + 死桩文档预览」两视图；无编辑器/差异视图；终端按 D1 决策移除（非丢失） | 10 |
| L3 | 文件树多文件夹分段 +「主要」徽标 | 9ca40f6 FilesPanel | folderId 读写通道在（types/channels/backend），分段 UI 丢 | 10 |
| L4 | 图片附件链：缩略图、放大浮层、用户消息附件渲染、粘贴上传 | 34a5423/e810be3/6ae7856 | savePastedAttachment/readAttachment 通道完备但渲染层零调用；时间线不渲染附件 | 07（本轮接渲染与粘贴） |
| L5 | 上下文占用圆环 + 输出速度估算 + usage.updated 实时消费 | 9b3c0e7 ContextRing | usage.updated 每轮广播、前端丢弃；无任何水位显示 | 11 |
| L6 | path:line 行内引用跳转 | 1e53511 markdownRef.ts | 无编辑器可跳，依赖 L2 | 12 |
| L7 | 消息分页（滚动加载更早） | 8e7842d | threadSnapshotPage 两端齐备、前端零调用（快照全量装载，无截断） | 13 |
| L8 | JS 测试基建（Vitest 单测 + Playwright e2e 截图回归） | 9245db4/6cef891 | 零残留；lib/ 纯函数注释仍写着「由 vitest 直测」 | 14 |
| L9 | 行为规格体系（docs/specs/）与黄金对拍 | d1e5cf8；AGENTS 声称 contract 黄金对拍 | docs/specs 已删而 AGENTS §5/§11 仍引用；contract 仅信封有序列化测试，dispatch 对拍不存在——D2/D3/D4 恰好全在盲区 | 15 / 09（本轮最小恢复） |
| L10 | 全局 Esc 退出链（浮层→面板→运行中双击确认→待审批） | 3390482/1bf8ffc | 窗口级 Esc 直接 return；停止按钮双击确认在但 title 写「停止（Esc）」误导 | 07（本轮恢复） |
| L11 | 自由档确认门（能力清单卡片+勾选门） | e9880ae FreeLevelDialog | 降级为普通确认框（lib/approval.ts:40-48） | 06（本轮复原） |
| L12 | steer「待送达」标记 | 6818db4 | 后端队列在（MAX_STEER_QUEUE=10、message_id 去重），user.message 无 via 字段、前端无标记 | 07（本轮恢复） |
| L13 | 通知点击聚焦跳会话 | 899429f | 依赖通知插件点击载荷，本轮先发通知+默认聚焦 | 17 |
| L14 | composer + 动作菜单 / 正文列 80% 约定 / 空会话留白 / under-window 材质 / 组折叠过渡 / 组计数徽章 / 折叠持久化 / 跑马灯 hover 1s 延时契约 | mygo 时期 8dd3e54/802a2a5/9653d05/4c5d183/bb2e3b8/e74daa9 | 观感回潮或部分保留（气泡、扫光、跑马灯滚动本体、字号缩放在） | 18 |
| L15 | updater 实接（清单签名校验） | 5e1d52d internal/update | 决策 D6 留桩，但实际连 tauri-plugin-updater 都未接入（plan.md「链路备好」与实况不符） | 19 |
| L16 | craft-doc Format 注册表扩展点 / folderId 目标根限定 | 467655f formats.go / 9ca40f6 LocateFile | match 分发（功能齐、扩展点丢）；folderId 被忽略按全根解析（services.rs:338/:403/:431） | 20 |
| L17 | workspaceSetActive / workspaceLocateFile / catalogLookup 死通道 | — | 声明在、dispatch 无、无调用方 | 16（清理或实装） |
| L18 | 事件死分支：settings.updated / tool.started / thread.updated(deleted) 前端有 case 后端零发射；thread 改名/自动命名不广播（侧栏标题延迟） | — | 低害死代码与竞态 | 16 / 21 |

## 三、确认完好（两轮核验通过，不动）

grants 回载（settings 键懒读，重启授权保留）｜审批活链（卡片→决策→执行→resolved 回流）｜中断恢复与重试（interrupt_stale_runs + retryable + 重试按钮）｜上下文压缩链路（阈值/保留窗/三事件落库/前端 notice 渲染）｜shell 前缀授权（前缀键与受限区语义）｜工具注册与 spec 声明（16 工具、路径/命令参数齐全、风险分级一致）｜apply_patch 预演逐块应用｜git 四工具｜多根 Roots（folders 全量进 roots、附加目录进提示词）｜写前备份与恢复前备份｜audit/usage/备份通道与设置页三分区｜自动命名短消息短路｜附件加载与模型侧 vision 过滤｜贴底滚动（use-stick-to-bottom 全语义）｜命令面板与快捷键表｜主题三态跟随系统｜字号缩放拖动应用｜草稿态存取与首轮创建并首发｜toast 链｜streamdown 渲染（@source + 字号映射 + controls）。

## 四、本轮修复范围与提交切分

见 `issues/01~09`（本轮，labels: ready-for-agent）；`issues/10~21` 为后续排期（labels: needs-triage）。

## 验收

- [x] 对账表覆盖两轮审计全部结论，损坏项均有断点 file:line
- [x] 本轮每张修复票有可验证的验收标准
- [x] AGENTS.md 失效引用（docs/specs、黄金对拍描述）修正为实况
