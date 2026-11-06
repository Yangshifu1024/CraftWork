# CraftWork 全仓审计报告（2026-11-07）

- **审计日期：** 2026-11-07
- **代码基线：** 提交 `eb71430`（工作树干净）
- **审计方式：** 只读审计，未修改任何代码；未运行 `cargo test` / `clippy` / `pnpm typecheck` / GUI 冒烟。
  文中「没有测试」的判断来自源码中没有对应 `#[cfg(test)]` 模块，而不是测试失败的输出。
- **行号说明：** 文中所有 `path:line` 对应上述提交；后续若修改相关文件，行号会漂移，请以函数名/常量为准重新定位。

## 这个仓库在做什么

CraftWork 是一个纯本地的桌面 Agent（Tauri 2 + Rust 领域内核 + React 渲染层）：在用户工作区里读写文件、跑命令、操作 Git 与 Word/Excel/PDF，所有写操作经审批档位门控并落审计，会话与消息以 SQLite 事件表为唯一事实源。

## 假设的负载

按 **单人桌面使用** 判断规模：一次只有一个窗口、一个会话在跑，模型调用走远端，仓库可能很大（十万级文件）、会话可能很长（数百轮工具调用）。并发不是重点，单次操作的耗时、误授权的范围和「写坏文件」才是。

审计覆盖的主链路：`runner.rs`（Run 循环/审批/计划态）、`events.rs` + `recorder.rs`（事件与快照）、`resolve.rs` + `command.rs` + `engine.rs`（路径与风险）、`registry.rs` + `fs.rs` + `shell.rs`（工具）、`platform.rs`（子进程、环境清洗、原子写）、`openai.rs` / `anthropic.rs` / `sse.rs` / `retry.rs`（模型）、`services.rs` + `lib.rs` + `system.rs`（服务与 IPC）、`store/app.ts`（前端事件应用）。

---

## 必须修复

### 1. 「本工作区允许」的授权在所有项目里生效

- **这是什么：** 审批卡的 session / workspace 两种永久授权存在一张全局表里（`crates/craft-agent/src/grants.rs:13`、`:66-76`）。
- **问题：** 键只有 `tool|target`，没有工作区 ID。在项目 A 选「本工作区允许」，之后在项目 B 里模型执行 `shell_exec`（键为 `shell_exec|prefix:npm test`，见 `grants.rs:86-106`）时 `approvals.allowed` 直接命中，不再弹卡、直接执行 B 目录下的脚本。
- **修法：** 授权键带上 `workspace_id`：`grant()` / `allowed()` 增加一个 workspace 参数，持久化键改成 `{workspace}|{tool}|{target}`。约 10 行，调用点只有 `runner.rs:1378` 与 `approvals.rs:293-295`。
- **如果不修：** 用户以为只在 A 项目放行，实际等于整个应用放行，审批边界失真。

### 2. 保存新文件时绕过符号链接校验，可写到工作区之外

- **这是什么：** 编辑器保存走 `workspace_write_file`（`src-tauri/src/services.rs:684-704`）。
- **问题：** 先 `resolve_existing` 做真实路径校验；失败后回退 `lexical_at`（纯词法、不解析符号链接）再写入。工作区里有 `link -> /etc` 时保存 `link/x.txt`：`resolve_at` 判定越界返回 Err，回退分支却认为合法，`create_dir_all` + 写入会跟随链接落到 `/etc/x.txt`。父目录不存在的新文件也永远走这条无校验的路径。
- **修法：** 回退分支先按词法建好父目录，再重新调一次 `resolve_at`（此时父目录存在，`canonicalize` 能成功）并用它返回的真实路径写入；仍然失败才报「路径超出工作区」。
- **如果不修：** AGENTS.md 里写死的「符号链接逃逸必须被真实路径校验拦截」在保存链上是空的。

### 3. `fs_grep` 会跟着符号链接读到工作区外

- **这是什么：** `fs_grep` / `fs_glob` 遍历用的 `walk_files`（`crates/craft-tools/src/fs.rs:206-229`）只校验了根目录，子项不重新校验。
- **问题：** `entry.file_type()` 对符号链接返回的不是目录，于是链接被当成普通文件交给回调，`fs.rs:359` 的 `std::fs::read(&abs)` 会跟随链接读取目标。仓库里存在 `data -> ~/.ssh` 这类链接（克隆来的仓库、monorepo、手工建的链接都常见）时，私钥内容会作为 grep 结果进入模型上下文并发往远端提供商。
- **修法：** 在 `walk_files` 里跳过 `file_type().is_symlink()` 的条目（`fs_read` 已走严格解析，不影响正常读文件），两行。
- **如果不修：** 「不访问工作区之外的路径」这条行为契约对搜索工具不成立。

### 4. 附件读取没做目录包含校验，可读出密钥文件

- **这是什么：** `read_attachment` 直接 `attachment_dir.join(name)` 后读文件（`src-tauri/src/services.rs:1427-1445`）。
- **问题：** `Path::join` 遇到绝对路径会整体替换，传入 `/Users/xxx/.craftwork/keys.json` 或 `../keys.json` 都会成功读取，并以 base64 返回给渲染层——这正好是 AGENTS.md 里「密钥不进入渲染层」的反例。
- **修法：** 先拼路径，再 `canonicalize` 并断言其以 `attachment_dir` 的真实路径为前缀；不合法直接报错。约 6 行。
- **如果不修：** 只要渲染层任何一处把外部字符串当附件名传下去，密钥库就能被读出来。

### 5. 编辑非 UTF-8 文件会把它写坏

- **这是什么：** `fs_edit`（`crates/craft-tools/src/fs.rs:487-506`）和 `apply_patch`（`:694-730`）都用 `String::from_utf8_lossy` 解码后整文件回写。
- **问题：** 打开一个 GBK / Shift-JIS 编码的源文件（Windows 迁移来的项目、含中文注释的旧文件很常见），非法字节被替换成 U+FFFD，替换成功后再写回，原文件里所有非 UTF-8 字节永久丢失——备份在，但没人会想到去恢复。
- **修法：** 改用 `std::fs::read_to_string`，读不出 UTF-8 就返回「文件不是 UTF-8 文本，无法安全编辑」，各一行。
- **如果不修：** 一次「成功」的编辑就是一次静默数据损坏。

### 6. 保存端点 / 请求头会静默把提供商和模型改回「启用」，并丢掉目录元数据

- **这是什么：** `provider_save` 重建条目时写死 `"enabled": true`，模型列表原样透传（`src-tauri/src/services.rs:1169-1188`）。
- **问题：** 设置页在「保存端点」「保存请求头」时只回传 `{id,name,contextK,maxTokens,caps,cost}`（`src/renderer/src/overlays/settings.tsx:449-456`、`:539-546`），于是 `PersistedModel` 的 `enabled` 走默认 `true`、`source` 清空、`reasoningKind` / `reasoningLevels` 清空。结果：用户停用过的模型被重新启用并可能被调用计费，目录来源模型变成「手动来源」（连 ID 都能改了），思考档位声明消失。
- **修法：** `provider_save` 对已存在的提供商 / 模型做字段合并而不是整体替换（`enabled`、`source`、`canonicalId`、`releaseDate`、`reasoning*` 缺省时保留现值）；提供商自身的 `enabled` 同样保留。
- **如果不修：** 用户每改一次端点，模型启停与元数据就悄悄复位一次，且没有任何提示。

### 7. 侧栏里所有会话的模型名恒为「模型不可用」

- **这是什么：** `model_name_for` 只在 `set_model_name_resolver` 注入解析器后才返回真名（`crates/craft-store/src/view.rs:38-54`）。
- **问题：** 全仓库没有任何地方调用 `set_model_name_resolver`（`src-tauri/src/lib.rs` 的装配段也没有），所以只要 `model_id` 非空，`ThreadView.model_name` 永远是「模型不可用」。前端 `validName` 把它当无效值过滤（`src/renderer/src/components/conversation.tsx:1119-1128`），模型胶囊只能退回显示模型 ID 尾段——所有会话的模型标识都不准。
- **修法：** 在 `build_app_state` 里装配一行：`craft_store::view::set_model_name_resolver({ let c = Arc::clone(&catalog_arc); move |r| c.resolve(r).map(|m| m.name) })`。
- **如果不修：** 会话列表、模型胶囊长期显示错误名字，用户无法判断某条会话到底用的哪个模型。

---

## 应该修复

### 8. `fs_read` 的 256KB 前缀让「用 offset 继续读取」永远拿不到后半段

位置：`crates/craft-tools/src/fs.rs:149-173`

- **这是什么：** 文件超过 256KB 时只解码前 256KB，再在这段前缀里按行分页。
- **问题：** 一个 5MB 的日志，`offset=50000` 直接报「offset 超出文件行数」，而提示语还写着「使用 offset=… 继续读取」；文件 256KB 之后的内容根本无法通过该工具读到（8MB 以下不会被拒绝，用户和模型都会以为能读）。
- **修法：** 用 `BufReader` 逐行 `skip(offset-1)` 再 `take(limit)`，累计字节到 `MAX_READ_BYTES` 为止；读与截断都按行做，提示语同时改成准确文案。
- **如果不修：** 大文件尾部排查（日志、生成代码、数据集）只能靠 shell 绕路。

### 9. `fs_glob` 对每个文件都重新编译一次正则

位置：`crates/craft-tools/src/fs.rs:232-272`、`:288-293`

- **这是什么：** `match_glob` 内部调 `glob_to_regexp`，而它是在遍历回调里逐文件调用的。
- **问题：** 十万文件的仓库里就是十万次 `Regex::new`，典型需要几秒到几十秒；模型每找一个文件就付一次这个代价。
- **修法：** 在 `execute` 里编译一次 `Regex`（含 `**/` 后缀那一支）后以 `&Regex` 传入 `match_glob`。
- **如果不修：** 大仓库里 glob 慢到让模型误判「没有匹配」。

### 10. `workspaceGitStatuses` 忽略入参，串行给所有项目跑 `git status`

位置：`src-tauri/src/services.rs:745-762`、`src-tauri/src/lib.rs:391`

- **这是什么：** 通道声明带 `workspaceId`（`src/shared/channels.ts:171`），dispatch 把参数丢掉了，服务函数对每个 project 逐个跑 git，各 5s 超时。
- **问题：** `conversation.tsx:1503` 勾选 worktree 前会调它，但只用 `statuses[metaWs.id]` 一个值。有 8 个项目、其中两个在慢盘或网络卷上时，点一下勾选框要等 40 秒；每次调用还会起 8 个子进程。
- **修法：** dispatch 传 `a.str_at(0)`，服务层只查该工作区。
- **如果不修：** 勾选 worktree 会随机卡住十几秒，用户以为界面死了。

### 11. `masked_for` 用字节下标切密钥，含多字节字符时 panic

位置：`crates/craft-core/src/secret.rs:131-141`

- **这是什么：** 长度 > 10 时返回 `&key[key.len()-4..]`。
- **问题：** 用户把带中文或全角符号的字符串粘进密钥框（复制时混入说明文字很常见），`provider_view_json` 构造遮罩时字节切在多字节字符中间 → panic；因为它在 `spawn_blocking` 里，设置页的提供商列表整个打不开，直到手工清空 `keys.json`。
- **修法：** 用 `chars()` 取前 5 与后 4：`format!("{}…{}", head, tail)`。
- **如果不修：** 一次误粘贴就让设置页永久不可用。

### 12. 审批状态机与计划态闸门没有任何测试

位置：`crates/craft-agent/src/approvals.rs` 全文

- **这是什么：** 「请求 → 卡片 → 决定 → 执行/跳过」这条安全主链，包含 scope 归一、`decision IS NULL` 并发保护、`allow_permanent=false` 时降级 once、取消落审计，全部没有测试；`docs/plan-mode/spec.md:93` 也写明 run 层三路径与自由档闸门测试待补。
- **问题：** 一个改动把 `decide()` 里的 `!entry.permanent` 校验删掉，或把 `Allow` / `Ask` 分支顺序调换，`cargo test` 全绿。这类错误的表现是「本该弹卡的写操作直接执行了」，很难在正常使用中被发现。
- **修法：** 先补三个测试：拒绝时 scope 必降为 once、`allow_permanent=false` 时 session / workspace 被拒、取消令牌触发后卡片落 canceled；再加一个 `Engine::action` 与计划闸门的组合用例。
- **如果不修：** 审批是唯一的安全边界，却只有人工回归。

### 13. 每次切会话、每次 run 起始都全量重扫事件表

位置：`crates/craft-store/src/events.rs:243-247`

- **这是什么：** `snapshot` 调 `list_events` 读该会话全部事件、逐条 `serde_json::from_str`，`limit` / `before_seq` 只在这之后裁剪。
- **问题：** 一轮 agent 能产生几百个部件，长会话上千个部件（含 32KB 的工具输出）。切会话和每次发送消息（`runner.rs:552`）都要把这些全部反序列化一遍、再全量排序，界面会明显停顿，且 `limit` 参数完全没有起到「少读」的作用。
- **修法：** 有 `limit` 时先按 `seq DESC LIMIT n` 取尾部事件再升序还原（占位消息需要再加一条 `runs` 查询）；无 `limit` 时保持现状。
- **如果不修：** 会话越长越卡，用户会以为应用变慢了。

### 14. `build_history` 计算了没人用的 `seqs`

位置：`crates/craft-agent/src/history.rs:123-162`

- **这是什么：** 返回结构 `HistoryResult { messages, seqs }`，`seqs` 在每个消息上 push 一次。
- **问题：** 唯一调用方只取 `.messages`（`crates/craft-agent/src/runner.rs:563`），每次重建历史都在为一个死字段做分配和拷贝。
- **修法：** 删掉 `seqs` 字段与相关 push，返回 `Vec<Message>`。
- **如果不修：** 次要，但这是「看起来有人需要、实际没人需要」的典型，会误导后续维护者。

---

## 可以更好

### 15. 纯死代码，可以直接删（约 45 行）

- **这是什么：** 一批写了但全仓库无人调用的函数与常量：
  `crates/craft-agent/src/runner.rs:44` `MODEL_REQUEST_TIMEOUT_MS`（真正的超时是 `crates/craft-model/src/sse.rs:205` 的 600s）、
  `crates/craft-tools/src/registry.rs:389` `more_hint`、
  `crates/craft-policy/src/engine.rs:106` `is_workspace_trusted`（服务层直接比字符串）、
  `crates/craft-store/src/threads.rs:292` `new_thread_id`、
  `crates/craft-model/src/catalog.rs:370` `normalize_model_ref`、
  `crates/craft-model/src/factory.rs:70` `catalog_arc`、
  `crates/craft-agent/src/grants.rs:79` `snapshot`、
  `crates/craft-core/src/platform.rs:380` `run_async`、
  `src-tauri/src/system.rs:138` `null_value`、
  `crates/craft-core/src/contract.rs:12` `RUN_FINISHED`（实际广播的是字面量 `"run.completed"`）、
  `src/shared/channels.ts:201` `channelOf`。
- **问题：** 每一条都要被读、被维护、被误以为是接线点（例如 `MODEL_REQUEST_TIMEOUT_MS` 会让人以为请求有 3 分钟上限，而真实的 600s 上限在 `sse.rs` 里）。
- **修法：** 逐一删除；`RUN_FINISHED` 若保留就改成使用常量而非字面量。
- **如果不修：** 后续维护者会按这些名字去找接线，白花时间；空常量还可能被误当配置项热改。

### 16. `apply_patch` 的「+N 行」统计算错

位置：`crates/craft-tools/src/fs.rs:704-713`

- **这是什么：** `added += new_len - new_len.min(old_len) + (new_len - old_len).max(0)`。
- **问题：** `old=1, new=3` 时算出 +4（应为 +2）；纯新增块也被重复计入。用户看到的「+4 −0 行」与实际不符。
- **修法：** 直接按标签计：`added += 行内 '+' 数量`、`removed += 行内 '-' 数量`。
- **如果不修：** 模型与用户都会拿这个数字判断改动规模，报错会让「只加了两行」看起来像「加了四行」，影响后续取舍。

### 17. 4 个通道渲染层完全不调

位置：`src/shared/channels.ts:80`、`:146-147`、`:154-155`、`:161-162`

- **这是什么：** `threadSnapshotPage`、`catalogLookup`、`workspaceSetActive`、`workspaceLocateFile`；后三个已登记在 `src-tauri/tests/dispatch_test.rs:11` 的白名单里，第一个连白名单都不需要（有实现但无人调用）。
- **问题：** 三处同步接线（channels / bridge / dispatch）都要为它们维护签名与参数顺序，翻页快照还有 30 行分页逻辑在跑但从不被验证。
- **修法：** 删掉未用的四个签名与对应 dispatch 臂；翻页能力若将来要用，等前端真需要时再加。
- **如果不修：** 每加一个通道都要在三处同步维护这些死签名，`dispatch_test` 的白名单也会一直是「待清理」状态，掩盖真正缺失的接线。

---

## 结论

**Verdict：** 内核结构干净、事件与审批的设计是站得住的，但有 7 处必须先修：授权不按工作区隔离、两条路径可以逃出工作区、非 UTF-8 文件会被写坏、改端点会静默复位模型启停、以及所有会话模型名显示错误。先修 1、2、5、6，其它按顺序排。

**Lean: 约 −45 行、−0 个依赖**（死代码清单 15 与死通道 17）。

## 未检查的部分

前端组件只读了 `store/app.ts` 全量、`settings.tsx` / `conversation.tsx` / `provider-add.tsx` 的相关片段，其余界面代码、`craft-doc` 的 docx / xlsx / xml 细节、`scripts/modelsync.mjs`、`tests/fixtures` 与 `assets/catalog` 数据未逐行读；本次是只读审计，没有运行 `cargo test`、`clippy`、`pnpm typecheck` 或 GUI 冒烟，上述「无测试」结论来自源码中没有 `#[cfg(test)]` 模块，而非测试失败输出。
