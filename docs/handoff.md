# 输出上限截断：取证与修复方案

> 对象：会话 `th_54874a9392ebb60b`（标题「各系统Shell选择逻辑」）——末尾回答要求给出三条原则，
> 实际只完整输出第一条，第二条在 `2. **破坏性/网络` 处断在半句，界面无任何提示。
>
> 状态：**只读取证完成，尚未修改任何代码**。日期 2026-11-06。

## 1. 结论

**这不是网络故障，也不是 SSE 流损坏，而是输出上限（max_tokens）被打满后的正常截断。**

三层原因，缺一不可：

1. **直接原因**：该模型的 `maxTokens` 被手动设为 `10240`，而目录中同一模型的值是 `384000`。末轮调用的
   `output_tokens` 精确等于 `10240`——上限被打满。
2. **放大因素**：思考型模型的 reasoning 与正文共享同一份输出预算。该轮 `reasoning_tokens = 8005`，
   正文实际只剩约 2235 token，所以刚写完第一条原则就用尽了空间。
3. **为什么你没察觉**：客户端已经识别出截断，但只写进日志文件，**不投递任何用户可见事件**；
   run 以 `status=done`、`error_code` 空收尾，界面显示「正常完成」。

即：前两层是配置与产品语义问题，第三层才是需要改代码的缺陷。

## 2. 现场证据

| 项 | 值 |
| --- | --- |
| 触发消息 | `events.seq=58`（2026-11-06 09:43:03），`msg_36259fe33edb6f97` |
| 运行 | `run_49c6aa9d9d792d92`，09:43:03 → 09:50:05，`status=done`，`error_code` 空 |
| 末轮调用 | `usage.id=797`：input 87770 / **output 10240** / cache 86016 / **reasoning 8005** |
| 正文事件 | `events.seq=137`（kind `text`，4497 字符），末尾停在 `2. **破坏性/网络` |
| 末轮推理 | `events.seq=136`（kind `reasoning`，30014 字节，即 8005 reasoning token） |
| 配置值 | `settings.model.custom_providers` → `commandcode` → `deepseek/deepseek-v4.1-flash-fast`，`maxTokens: 10240` |
| 目录值 | `assets/catalog` 同一模型 `maxTokens: 384000` |
| 模型能力 | `caps.reasoning = true`，`reasoningKind = "effort"`，档位 `none/low/high/max` |

支撑推断的两点：

- `output_tokens` 与配置上限**精确相等**（10240），恰好等于上限的概率极低；
- 正文尾部是 `2. **破坏性/网络` —— 加粗标记未闭合、句子未收束，是被硬切断的形态，而非模型自然停笔。

**证据的局限（如实说明）**：该会话没有 `logs/th_54874a9392ebb60b.log`。`session_log` 是本次诊断前若干小时
才落地的能力（commit `d6fe02f`），历史 run 无日志落盘，因此**提供商侧 `finish_reason` 无法回放**，
「`length` / `max_tokens`」是由 token 数与文本形态反推的结论，不是直接读到的字段。
下一次复现即可用 `stream_end` 行直接确认。

## 3. 代码链路

### 3.1 截断已被识别，但没有出口

- `crates/craft-agent/src/runner.rs:792` 计算 `anomaly`；`stream_end_anomaly()`（`:1389`）对
  `finish_reason` 为 `length` / `max_tokens` 返回异常描述，识别逻辑本身是对的。
- `runner.rs:821` 拿到 `anomaly` 后，仅通过 `:823` 的 `stream_raw_tail` 写日志文件，**除此之外没有任何动作**。
- `runner.rs:899`：`tool_calls.is_empty()` 成立即 `complete()`，于是 run 落 `done`、`error_code` 为空。
- 渲染层其实早有准备：`crates/craft-core/src/contract.rs:26` 定义了 `NOTICE` 契约常量，
  `src/renderer/src/store/app.ts` 的 `notice` 分支与 `src/renderer/src/components/conversation.tsx:414`
  都已能渲染该部件。**能力齐备，只是没有任何一方投递它。**

### 3.2 `max_tokens = 0` 的语义在两种协议下相反

- `crates/craft-model/src/openai.rs:124`：`if req.max_tokens > 0` 才写入请求体，填 0 = 省略字段、由网关决定。
- `crates/craft-model/src/anthropic.rs:117`：`if req.max_tokens > 0 { .. } else { 4096 }`，填 0 = 最紧的 4096。

同一个设置项，换个协议从「不限」变成「最小」，这是容易踩的陷阱；本案是它的反向情形（把「不限」误改成 10240）。

### 3.3 配置侧无任何提示

`src/renderer/src/overlays/provider-model-dialog.tsx:201` 的「最大输出（tokens）」字段，
只受 `src/renderer/src/lib/model-form.ts:9` 的 `MAX_TOKENS_LIMIT = 4_000_000` 边界约束，
不与目录同模型的值比对。把 384000 改成 10240，界面照收，直到下一次长回复才以「静默半截」暴露。

## 4. 修复方案

### 第 0 步：先恢复这个会话（不改代码）

设置 → 提供商 → CommandCode → 该模型 → 「最大输出」由 `10240` 改为 **`384000`**（对齐目录值），
再对该会话发「继续：第 2、3 条原则接着写完」。

**不要填 0**：按 4.3 的现状，OpenAI 协议下 0 是「交给网关」，一旦换到 Anthropic 协议就退化成 4096，比现在还紧。
等 4.3 修完再考虑用 0 表达「不限」。

### 4.1 让截断对用户可见（核心修复，优先级最高）

**文件**：`crates/craft-agent/src/runner.rs`

- 位置：`runner.rs:821` 的 `if let Some(reason) = anomaly` 分支内。
- 做法：除现有日志外，投递一条 `notice` 部件事件（`craft-core/src/contract.rs:26` 的 `NOTICE`），文案形如：
  > 本次回复触发了模型输出上限（10240 tokens），内容可能不完整。可在设置中调高该模型的「最大输出」，或让我继续写。
- 同时：该轮 run 不应被计为干净完成。建议在 `runs` 侧保留一个可区分的标记（如 `error_code = "output_limit"` 但状态仍为可继续），
  使重放与统计能识别「这类 `done` 是半截的」。**具体落法需先确认**：直接复用 `error_code` 会让界面走错误分支，
  可能反而更差；备选是新增 `notice` + 保持 `done`，仅靠 notice 提示。见第 6 节待决策项。
- 验证：构造 `finish_reason = "length"` 的假流用例，断言产出 `notice` 且 `text` 已落库；`pnpm test:rust`。
- 风险：低。只新增事件，不动既有 `text`/`reasoning` 落库路径。

### 4.2 配置侧提示低值（建议同期做）

**文件**：`src/renderer/src/overlays/provider-model-dialog.tsx`、`src/renderer/src/lib/model-form.ts`

- 当目录中存在同 `canonicalId` 的模型、且用户填的值显著偏低（如 < 目录值 1/4）时，给**非阻断**警告，
  文案指明「目录值为 N，当前值可能不足以完成长回答」。不阻断保存——手动声明低值可能是刻意控制成本。
- 字段说明补一句「思考 token 与正文共享此上限」，这是本案 8005/10240 的实际教训。
- 验证：`src/renderer` 侧 vitest（`tests/model-form.test.ts` 已有同类校验用例）。

### 4.3 统一 `max_tokens = 0` 的语义

**文件**：`crates/craft-model/src/types.rs`、`openai.rs`、`anthropic.rs`、模型弹窗文案

- 建议语义：`0` = 不主动设限，由提供商默认决定；`> 0` = 用户显式上限。
- Anthropic 的 `max_tokens` 是必填字段，无法真正「不设」，需要**区分「用户填 0」与「缺省未填」两种情况**，
  而不是像现在这样共用同一个 4096 兜底（例如缺失时取模型目录值，用户显式填 0 时取目录值而非 4096）。
- 验证：`openai.rs:484` 已有 `request_body_omits_fields_when_max_tokens_zero_and_tools_empty`，补 Anthropic 侧对应用例。
- 风险：中。Anthropic 侧行为会变化，需同步 `docs/arch.md` 的模型层说明。

### 4.4 超限自动续写（待决策，默认不做）

检测到 `finish_reason = length` / `max_tokens` 时，自动追加一次「从断点继续」的请求，上限 1~2 次。

- 收益：用户不必手动催「继续」。
- 代价：多一轮 token 消耗；存在续写重复、开头截断、自激循环（一直续也一直超限）的风险，需要次数上限与去重策略。
- 前置：这是行为规格级改动，应先写 `docs/<feature>/spec.md` 再落地，不要在 runner 里悄悄加。

## 5. 验证与回归

| 项 | 命令 / 方法 |
| --- | --- |
| Rust 单测 | `pnpm test:rust`（重点 `craft-agent`、`craft-model`） |
| 静态检查 | `pnpm check:rust`、`pnpm lint:rust`、`pnpm typecheck` |
| 端到端复现 | `CRAFTWORK_HOME=/tmp/cw-trunc pnpm dev`，把某模型「最大输出」调到极小（如 64），发一条长要求，确认界面出现 notice |
| 日志对照 | 复现后查 `/tmp/cw-trunc/logs/<thread-id>.log` 的 `stream_end`，确认 `finishReason` 字段与 `anomaly` |

回归要点：正常结束（`stop` / `end_turn`）不得出现 notice；工具调用轮（`finishReason: tool_calls`）不得误报。

## 6. 待决策

1. **截断时的 run 终态**：保持 `done` + notice，还是给可区分的 `error_code`？前者界面友好，后者利于统计与重放识别。
2. **4.4 自动续写做不做**？默认不做。
3. **4.2 的告警阈值**（< 目录值 1/4）是否合适，抑或只在「明显低于上下文窗口」时提示。

## 7. 附：证据复现命令（只读）

```sh
cd ~/.craftwork
# 末轮用量：output 打满 10240，reasoning 占 8005
sqlite3 -readonly craftwork.db \
  "select input_tokens,output_tokens,cache_tokens,reasoning_tokens from usage where id=797;"
# 正文尾部：断在 "2. **破坏性/网络"
sqlite3 -readonly craftwork.db \
  "select payload from events where thread_id='th_54874a9392ebb60b' and seq=137;" | tail -c 300
# 运行终态：done 且无 error_code
sqlite3 -readonly -header craftwork.db \
  "select id,status,error_code,started_at,finished_at from runs where thread_id='th_54874a9392ebb60b';"
# 配置值 vs 目录值
sqlite3 -readonly craftwork.db "select value from settings where key='model.custom_providers';"
gunzip -c ~/Code/CraftWork/assets/catalog/catalog.json.gz | grep -o '"maxTokens": 384000' | head -1
```

---

> 迁移说明：本文件此前是 2026-11-05 全仓库注释整理产出的 52 条问题清单，已按指示以本方案整体覆盖。
> 需要旧清单时从 git 取回：`git show HEAD:docs/handoff.md`。

---

# OpenCode Go 网关 400（MissingSessionID）：自定义请求头支持

> 日期 2026-11-06。状态：**已实现**。

## 现象

OpenCode Go 提供商返回 HTTP 400：

> Request is missing x-opencode-session and cannot be routed efficiently（MissingSessionID）

网关要求每个请求携带 `x-opencode-session` 头做会话路由，而 CraftWork 此前只发协议标准头
（OpenAI 的 `Authorization` / Anthropic 的 `x-api-key` + `anthropic-version`），没有透传自定义头的入口。

## 实现（通用能力，不写死任何网关）

- **数据**：`PersistedProvider.headers: Vec<HeaderPair>`（settings 的 `model.custom_providers`，serde default 兼容旧数据）。
- **校验**：`craft_model::types::validate_headers`——名称限 RFC 7230 token、值禁换行（防头注入）、
  `authorization` / `x-api-key` 与密钥字段冲突拒绝（密钥可能来自环境变量，不能被静态头遮蔽）、
  大小写不敏感去重、16 条 / 单值 4KB 上限；保存时服务端复检，未携带 `headers` 的部分更新（端点保存路径）保留现值。
- **发送**：`OpenAICompatClient` / `AnthropicClient` 经 `with_headers` 接收，请求时在认证头之后逐个附加；
  工厂从提供商配置下传；`provider_test`（测试连接）与 `provider_fetch_models`（拉取模型）同源附加，行为一致。
- **会话占位符**：头值中的 `{sessionId}` 在发送时替换为当前 CraftWork 会话 ID——`Request.session_id`
  由 Agent 四个调用点（主对话 / 自动命名 / 输入预测 / 上下文压缩）填入 `thread_id`，
  因此每个会话自动获得独立路由值，配置一次即可，新建会话无需再改配置。
  无会话上下文的场景（测试连接 / 拉取模型）跳过含占位符的头，不发字面 `{sessionId}` 给网关。
- **界面**：设置 → 提供商详情卡新增「请求头」行——表单形式逐条编辑（名称 + 值两个输入框一行，
  可增删行，⌘↩ 保存 / Esc 取消），保存时整理为 `HeaderPair[]` 以 JSON 提交（存储格式即 JSON 数组）；
  整理与预校验收敛在 `src/renderer/src/lib/provider-headers.ts`，规则与服务端对齐（最终以服务端为准）。

## 使用（OpenCode Go）

设置 → 模型与提供商 → 该提供商 → 请求头 → 添加请求头，名称填 `x-opencode-session`、值填 `{sessionId}`：

保存即全局生效：每个 CraftWork 会话的请求自动携带自己的会话 ID 作为路由头，新建会话零操作。
如果网关要求固定值（如绑定控制台会话），直接填字面值即可，不带占位符就原样发送。

## 验证

- `cargo test -p craft-model`：`validate_headers_*` 8 项 + 旧数据无 `headers` 字段反序列化兼容。
- `cargo test --workspace`：202 项全过；`cargo clippy --workspace --all-targets -- -D warnings` 干净。
- `pnpm typecheck` 通过；服务层集成测试 `provider_custom_headers_save_validate_and_partial_update_preservation`
  覆盖保存 / 冲突拒绝 / 注入拒绝 / 部分更新保留 / 显式清空 / 目录导入不覆盖。

---

# 命令只读判定：`;` 不切分导致白名单首词被借用（R0 审批绕过）

> 日期 2026-11-07。状态：**已修复**。

## 现象

计划模式下两条只读调研命令被「计划模式下只允许只读调研」闸门拒绝（runner.rs 计划态只读闸门，
R2+ 一律拒）：

- `sed -n '1,80p' src-tauri/src/lib.rs; echo ---; grep -n "a\|b" … | head -40`
- `ls src-tauri/tests 2>/dev/null; grep -rn "skillsList" … | head`

排查发现比误拒更严重的是**误放**：`craft-policy::command` 的 `is_readonly_command` 只按
`&&`/`||`/`|` 切段，`;` 不参与，段级判定只看首词——因此 `cat x; rm y`、`ls ; rm x`
这类「白名单首词 + 分号链」整条被判 R0「只读命令」，分级档下 `Engine::action` 直接放行、
不弹审批卡，分号后的任意非破坏性/非联网命令（如 `python evil.py`）静默执行。

## 根因与修复（`crates/craft-policy/src/command.rs`）

1. **`;` 纳入切分**：重写 `is_readonly_command` 为引号感知的单趟字符扫描，未加引号的
   `&&`/`||`/`;` 切段、逐段独立过白名单（`all()` 语义不变），堵住首词借用；引号内元字符
   按字面并入当前段（顺带修复 grep 交替模式 `"a\|b"` 被误切为管道）；未加引号裸 `&`
   与未闭合引号一律拒绝，方向安全。
2. **丢弃型重定向剥除**：新增 `DISCARD_REDIRECTS`（`2>/dev/null`、`>/dev/null`、`2>&1`
   等字面形态），`classify_command` 入口先剥除再匹配——既放行 `ls x 2>/dev/null` 这类
   无副作用调研命令，也消除 `> /dev/null` 对破坏性子串 `> /dev/`（写设备）的误命中；
   剥除要求目标后跟边界符，`>/dev/nullfoo` 前缀伪装照旧拒绝。
3. **sed 维持整程序排除**（-i 写文件、`e` 执行、`w` 写出，2026-11 复核维持），计划模式
   在系统提示词计划态动态块（`craft-agent/src/prompt.rs`）注入白名单引导：改用
   head/cat/grep/fs_read 取片段。

## 验证

- `cargo test -p craft-policy`：新增 `semicolon_chain_cannot_borrow_first_word_whitelist`
  （绕过回归）、`discard_redirections_to_dev_null_map_to_r0`、`write_redirections_stay_rejected`
  （含 `>/dev/nullfoo` 伪装）、`quoted_metacharacters_are_literal_text`、
  `sed_print_mode_stays_non_readonly` 等；既有分类用例全部保持通过。
- `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings` 干净。
