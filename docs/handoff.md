# Handoff：注释整理过程中发现的问题清单

> 来源：2026-11-05 全仓库注释整理（只读审查）。本次改动**未修改任何代码逻辑**（仅注释与文档），
> 下列问题均按原样保留，供后续排期修复。每条给出位置、现象、影响与建议。

## P1 功能性缺陷（建议优先修复）

### 1. docx 段落/表格深度统计按标签前缀误匹配派生标签
- 位置：`crates/craft-doc/src/docx.rs:49`（段落）、`docx.rs:157`（表格 `find_balanced`）
- 现象：`starts_with_at(.., b"<w:p")` 会把 `<w:pPr`、`<w:proofErr`、`<w:pict` 也计为段落开标签（depth +1），而对应的 `</w:pPr>` 闭合不会 -1；表格侧 `<w:tbl`/`<w:tr`/`<w:tc` 同理命中 `<w:tblPr`/`<w:trPr`/`<w:tcPr`。
- 影响：真实 Word 文档几乎必含这些子元素，段落闭合判定失效会合并段落；含 `<w:tblPr`（表格标配）时连 `</w:tbl>` 都配对失败，`read()` 在首个表格处中断并丢弃其后全部内容。合成测试样例不含这些元素，故测试全绿。
- 建议：比较完整标签名；或利用「OOXML 段落/表格不嵌套」直接定位最近闭合标签，去掉 depth 逻辑。

### 2. RetryClient 流错误终态用 break 而非 return
- 位置：`crates/craft-model/src/retry.rs:62,81`
- 现象：`Some(Err)` 分支 `yield Err(err)` 后 `break`：(a) 最后一次尝试失败时，循环后的兜底会把更早一次尝试的旧 `last_err` 再 yield 一次（同一条流投递两个 Err）；(b) 第 1 次可重试失败、第 2 次遇到**不可重试**错误时，仍会退避并继续第 3 次尝试。
- 影响：当前依赖消费方「见 Err 即 drop 流」才无实际表现；语义脆弱。
- 建议：yield Err 后一律 `return`；顺带清理 `last_err.clone()` + `let _ = err;` 冗余。

### 3. SSE 整体超时会掐断长流
- 位置：`crates/craft-model/src/sse.rs:147`
- 现象：`open_sse` 的 `.timeout(600s)` 是 reqwest「从建连覆盖到响应体读完」的整体超时，SSE 流超过 10 分钟被以 timeout 错误切断；该错误可重试，会白白整轮重来。
- 影响：长推理输出 / 慢网关的长回合可能中途失败。
- 建议：改为 `connect_timeout` + 无总超时（或上层空闲超时）。

### 4. apply_patch 的 added 行数统计翻倍
- 位置：`crates/craft-tools/src/fs.rs:711`
- 现象：`added += (new_len - new_len.min(old_len)) + (new_len - old_len).max(0)` 两半恒等，净效果 `added = 2 × max(new_len − old_len, 0)`；removed 却是净变化口径。
- 影响：结果卡片「+N −M 行」虚增（纯新增 -0,+5 报 +10）；不影响补丁应用正确性，测试未覆盖该统计。
- 建议：与 removed 统一口径（净变化或 diffstat 二选一）并补单测。

### 5. fs_write 把「读取失败」当「文件不存在」绕过备份
- 位置：`crates/craft-tools/src/fs.rs:424`
- 现象：`if let Ok(existing) = std::fs::read(...)` 在读取失败（如权限不足）时 `had_existing=false`，不备份直接覆盖。
- 影响：原内容与权限在异常路径下丢失，违背「写前备份」承诺。
- 建议：先 `metadata` 判存在；存在但读取失败时报错而非静默覆盖。

### 6. 原子写固定 0o644 重置已存在文件权限
- 位置：`crates/craft-tools/src/fs.rs:437,505,728`（`write_file_atomic(.., 0o644)`）
- 现象：临时文件 + rename + set_mode 的原子写对已有文件固定设 0644。
- 影响：编辑可执行脚本（0755）或私有文件（0600）后权限被改写。
- 建议：目标已存在时读取原 mode 传入保留。

### 7. 权限配置项：可拖拽侧栏分隔条是假的
- 位置：`src/renderer/src/App.tsx:57`；关联 `store/app.ts` 的 `settings.panelWidth`（默认 320）
- 现象：元素带 `role="separator"`、`aria-label="调整侧栏宽度"`、`cursor-ew-resize`，但全渲染层无任何拖拽实现；侧栏宽度固定 `w-[264px]`；`panelWidth` 无任何布局消费。
- 影响：可访问性语义与视觉暗示「可拖拽」但实际无效；死配置误导。
- 建议：实现拖拽接线，或移除 role/aria-label/拖拽光标并清理 `panelWidth` 字段。

## P2 行为隐患（建议排期）

### 8. SQLite 前向版本静默放行
- 位置：`crates/craft-store/src/db.rs:241-243`
- 现象：`user_version >= SCHEMA_VERSION` 直接返回 Ok，新版应用产出的新库被旧版应用打开不报错，随后写入可能破坏新 schema。
- 建议：显式拒绝打开并提示「数据库版本高于当前应用」。

### 9. `MODEL_REQUEST_TIMEOUT_MS` 声明未接线
- 位置：`crates/craft-agent/src/runner.rs:37`
- 现象：常量（3 分钟）全仓库无引用，模型请求实际没有整体超时保护（仅有 P1#3 的 SSE 600s）。
- 建议：接到模型流消费（`tokio::time::timeout` 包裹流循环）或删除，避免暗示存在保护。

### 10. 工具「开始执行」借用完成态事件类型
- 位置：`crates/craft-agent/src/runner.rs:742-743`
- 现象：广播工具开始执行用 `event_type::TOOL_COMPLETED`（`"tool.completed"`），载荷却是 `"status": "running"`；事件类型表缺 `tool.started`。
- 影响：渲染层若只按 kind 判断会显示错误状态（当前靠 payload.status 区分）。
- 建议：在 craft-core contract 增加 `TOOL_STARTED` 常量供增量广播使用。

### 11. `APP_VERSION` 双源维护
- 位置：`src-tauri/src/services.rs:27`
- 现象：硬编码 `"0.1.0"` 与 `tauri.conf.json` 的 `version`（发布版本事实源）双源，发版漏改会使 getInfo 返回旧版本。
- 建议：改用 `env!("CARGO_PKG_VERSION")`，或发版脚本加一致性校验。

### 12. 环境清洗 deny 词表无效
- 位置：`crates/craft-core/src/platform.rs:92-115`
- 现象：`sanitized_env` 是纯白名单语义（白名单外一律不收），`ENV_DENY_SUBSTR`/`ENV_DENY_PREFIX` 永不改变结果。
- 影响：无行为缺陷，但误导读者以为存在「黑名单 + 放行」两级语义。
- 建议：删除两表，或改为「白名单外放行、命中 deny 才剔除」语义（需明确决策）。

### 13. masked_for 按字节回退可能 panic
- 位置：`crates/craft-core/src/secret.rs:132`
- 现象：`&key[key.len() - 4..]` 在密钥含多字节 UTF-8 尾字符时可能不在字符边界导致 panic。
- 影响：实际 API key 均为 ASCII，风险低。
- 建议：改用字符切片。

### 14. docx 段落末尾 `<w:tab/>`/`<w:br/>` 丢失
- 位置：`crates/craft-doc/src/docx.rs:126`
- 现象：命中 `"<w:t"` 前缀后必须找到后续 `</w:t>` 才能落到自闭合处理，若段内其后再无 `<w:t>` 则提前 break，该 tab/br 不输出。
- 建议：命中前缀后先判断自闭合再走对应路径。

### 15. `sudo` 提权匹配有缝隙
- 位置：`crates/craft-policy/src/command.rs:58`
- 现象：`lower.contains("sudo ")` 需要尾随空格，裸 `sudo` 或 `sudo\t` 落 R2——而 R2 属可永久授权档位，提权命令语义上不应进入永久授权候选。
- 建议：对 `sudo` 单独做前缀匹配。

### 16. 自动命名（sidecar）用量未记账
- 位置：`crates/craft-agent/src/title.rs`、`runner.rs:483-505,988`
- 现象：`auto_title` 流消费后丢弃 usage；`record_usage(.., sidecar=true)` 分支是死代码（唯一调用点恒传 false）。
- 影响：自动命名消耗的 token 不进用量统计（数据不完整不致错）。
- 建议：`auto_title` 返回流末尾 usage，由 Runner 以 `sidecar=true` 记账。

### 17. 恢复备份失败用全局 alert()
- 位置：`src/renderer/src/overlays/settings.tsx:1130`
- 现象：`BackupsSection` 恢复失败用 `alert()` 提示，违反「后端错误渲染层 toast 展示」约定（同文件其余错误路径均走 `pushToast`）。
- 建议：改为 toast。

### 18. error 事件空会话兜底产生空 id 消息
- 位置：`src/renderer/src/store/app.ts:430`（`run.started` 分支 ：328 同理）
- 现象：`lastStreamingId(...) ?? ... ?? ""` 兜底为空串后未像 notice 分支那样判空，空会话收到 error 事件会创建 id 为 `""` 的幽灵消息（React key 也为空串）。
- 建议：补空值守卫，或给合成 id（如 `error_${event.seq}`）。

## P3 死代码与轻微问题（顺手清理）

| # | 位置 | 现象与建议 |
| --- | --- | --- |
| 19 | `crates/craft-store/src/runs.rs:97` | `interrupt_stale_runs` 签名与函数体挤在一行（历史编辑事故），rustfmt 可修复 |
| 20 | `crates/craft-store/src/view.rs:59-62` | `let speed = match ..` 块缩进不对齐 |
| 21 | `crates/craft-store/src/view.rs:82` | `use crate::db::StoreResult;` 位于文件末尾，建议归位到顶部 use 区 |
| 22 | `crates/craft-store/src/db.rs:220-222` | V7 迁移删空 threads 时不清理名下孤儿 runs 行（影响极小，可在后续迁移顺带清理） |
| 23 | `crates/craft-store/src/threads.rs` | `deleted_at` 软删列只读残留：所有读取过滤它，但无任何写入路径；无 Node 版软删场景可删列 |
| 24 | `crates/craft-doc/src/xlsx.rs:76` | 空 if 分支无任何效果（历史遗留死代码），建议删除 |
| 25 | `crates/craft-doc/src/xlsx.rs:351` | `is_plain_number` 不支持正号与科学计数法（与 TS 版行为一致则属有意取舍，建议在行为 spec 记一句） |
| 26 | `crates/craft-doc/src/docx.rs:186-188` | `extract_table` 每轮从 0 重找 + `max(i)` 对齐，guard 近乎不可达，建议改前向 find |
| 27 | `crates/craft-doc/src/lib.rs:95` | `replace()` 对不支持格式的错误文案与 `read_file` 不一致，建议统一 |
| 28 | `crates/craft-policy/src/resolve.rs:80,130` | 词法段反斜杠检查不一致（`\\` vs `\`），实际影响极小，建议统一前缀黑名单 |
| 29 | `crates/craft-model/src/openai.rs:201,228,250` | `text_seen` 只写不读的死状态，建议删除 |
| 30 | `crates/craft-model/src/retry.rs:23` | `RetryClient::new` 的 `_provider` 参数未使用，建议移除或真正使用 |
| 31 | `crates/craft-model/src/factory.rs:110-111` | 缺密钥错误归因用 `provider.name`、停用错误用 `provider.id`，且未复用 `display_name`（name 空时文案以空格开头），建议统一 |
| 32 | `crates/craft-model/src/types.rs:266-269` | `types::ModelError` 与 `types::errors::ModelError` 双通道重导出，可二选一 |
| 33 | `crates/craft-tools/src/shell.rs:104` | `git()` 返回 `(RunResult, String)`，第二项全是调用点丢弃的克隆，建议直接返回 `RunResult` |
| 34 | `crates/craft-agent/src/approvals.rs:273` | `let _ = new_id(id_prefix::PART);` 空调用（压 unused 警告），每次 decide 白白生成一个 ID，建议删除 |
| 35 | `crates/craft-agent/src/runner.rs:483-504` | `title_pending` 防重复窗口只覆盖任务派发瞬间（remove 未入任务内），保护几乎无效；建议 remove 移入任务内部 |
| 36 | `crates/craft-agent/src/runner.rs:710` | failover 失败路径以 `now_millis()` 作为 started_at，run.durationMs 略失真 |
| 37 | `crates/craft-agent/src/runner.rs:523` | 恒等三元 `max_tokens: if >0 {x} else {0}` 可简化 |
| 38 | `crates/craft-agent/src/runner.rs:1021-1023` | `command_prefix_of` 纯转发包装，可删除 |
| 39 | `crates/craft-agent/src/grants.rs`、`history.rs` | `Grants::snapshot` 与 `HistoryResult.seqs` 无调用方（死代码；`seqs` 每次 build 都白构造） |
| 40 | `src-tauri/src/system.rs:115` | `null_value()` 全仓库零调用（死代码） |
| 41 | `src/shared/channels.ts:160` | `channelOf()` 死代码（Electron 遗留，注释已修正） |
| 42 | `src/shared/channels.ts:120,128,135` | 三个死通道（catalogLookup / workspaceSetActive / workspaceLocateFile）：dispatch 缺臂，受 dispatch_test 白名单登记，去留待定 |
| 43 | `src/renderer/src/store/app.ts:467` | `thread.updated` 分支 deleted 与 else 动作完全相同，可合并 |
| 44 | `src/renderer/src/lib/paste.ts:27` | 「超限丢弃」注释与实现（continue 跳过后更小文件仍可入选）语义不一致——注释已按实现修正；若产品本意是整批截断则代码是缺陷，需确认 |
| 45 | `src/renderer/src/overlays/dialogs.tsx:30-47` | 完全相同的 `useEffect`（prompt 时 setValue + 延迟 select）重复两次，建议删一个 |
| 46 | `src/renderer/src/components/conversation.tsx:570` | `ApprovalCard` 尾部 `<span className="hidden">{threadId}</span>` 疑似压 unused 警告的 hack，建议删 prop 链或真正使用 |
| 47 | `src/renderer/src/components/panel.tsx:192` | `DocView` 的 `<span className="hidden">{setText.name}</span>` 同为压警告 hack，建议 `const [text] = useState(..)` |
| 48 | `src/renderer/src/components/sidebar.tsx:536-537` | 文件尾 `void api;` 残留（`api` 已在复制会话 ID 处实际使用），可删除 |
| 49 | `src/renderer/src/components/sidebar.tsx:487-489` | threads 为空且有项目分组时同屏双空态（项目组「暂无任务」+ 全局「还没有会话」） |
| 50 | `src/renderer/src/components/cards.tsx:25` | TodoCard 条目 `key={i}` 用数组索引（TodoItem 无 id 字段）；整批替换当前无碍，若支持删除/重排会错位，建议后端加 id |
| 51 | `crates/craft-doc/src/pdf.rs:28` | `"stream"` 字样定界是启发式近似（压缩数据偶现同名字节会误判边界），解压失败回落原文兜底；彻底解决需解析 xref/对象结构 |
| 52 | `src-tauri/gen/schemas/` | 4 个生成物（acl-manifests / capabilities / desktop-schema / macOS-schema）被 git 跟踪，与 AGENTS.md「不能提交的生成物」清单矛盾；建议 `git rm -r --cached src-tauri/gen` 并加入 `.gitignore` |

## 已知行为差异说明（非缺陷，供对照）

- fs_list 保留 `.env.example`，fs_glob / fs_grep 跳过一切隐藏文件——有意设计（展示可见性 vs 避免误搜敏感文件）。
- `expectedHash` 防覆盖只在用户编辑保存链（`services.rs` workspaceWriteFile），Agent 工具链按设计以写前备份兜底。
- 全部事件（含 token 流增量）统一走 Bus 攒批 → emit `craftwork:events`；历史文档中「tauri::ipc::Channel 攒批」的表述未落地，已从现行文档移除。
