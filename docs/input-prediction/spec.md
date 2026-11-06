# spec：输入预测

一轮任务正常结束后，基于当前会话上下文与该会话的用户历史输入，后台调用**当前会话模型**预计算「下一句建议提示词」；用户静置 5 秒未输入时，在 composer 输入框中以幽灵文本（hint）展示，Tab 一键应用，直接键入则忽略。设置 → 通用提供「输入预测」开关，**默认开启**。

与自动命名（`crates/craft-agent/src/title.rs`）同一套 best-effort 语义：失败静默、无输出不展示、不阻断运行、用量记账省略；预测是瞬态 UI 辅助，**不落事件表、不写审计**。

## 行为约定

1. **触发**：仅 run 正常完成（`run.completed`）时后端立即预计算（计算时间被 5 秒静默窗吸收，到点即有内容）；`run.canceled` / `run.failed` 不触发（取消意味着用户马上要输入修正指令，失败时上下文不完整）。steer 接力产生的新 run 完成后同样触发，覆盖旧预测。
2. **预计算输入**：当前 run 重建并追加后的完整历史（`history`）：用户历史输入（该会话全部 user 消息文本，体现输入习惯）+ 最近对话转写（体现当前上下文），单轮请求，无工具、无思考档位。
3. **开关门控（后端）**：预计算前读设置 `inputPrediction`（settings 表 `app` 键），关闭则不发起模型调用。开关保存即时生效（下一次 run 完成即按新值）。
4. **展示条件（前端）**：run 结束（running 下降沿）后启动 5 秒定时，到点时满足以下全部条件才拉取并展示 hint——期间用户未键入任何内容；输入框为空；会话态（非草稿）；会话未接力新 run（未 running）；仍停留在该会话。任一不满足则本次不展示。后端未算完（返回空）时补偿重试一次（+4 秒），再为空则本次放弃——sidecar 慢模型在 5 秒窗口内算不完是常态，只拉一次会永远看不到预测。
5. **hint 呈现**：输入框内叠加幽灵文本层（非原生 placeholder，与空输入共存），颜色用 `text-foreground-subtlest`，单行截断，行尾附 Tab 键提示徽标。运行中、草稿态不展示。
6. **应用（Tab）**：hint 存在时 Tab 将预测文本填入输入框、光标置于末尾（复用 `setTextAndCursorEnd`），hint 随即消失；IME 组词中的 Tab 交给输入法（沿用 `isImeComposing` 守卫）。Tab 键此前无任何占用（composer 与全局快捷键均未监听）。
7. **消失时机**：用户键入（onChange）、程序化回填（↑ 历史翻阅）、发送（submit，含失败还原）、切换会话、新 run 开始（running 变 true）。
8. **输出约束**：跟随会话语言的一句话建议，≤50 字，超长截断；清洗规则拒收对话式输出（自我介绍/回答正文）——无效输出等价于无预测。
9. **开关门控（前端）**：设置关闭时前端也不展示（后端关闭即不产出，前端以设置项做即时兜底判断）。
10. **失败静默**：无模型/密钥、调用失败、输出无效 → 无 hint、无 toast、无日志告警升级（会话日志 debug 级记录即可）。

## 实现落点

- `crates/craft-agent/src/prediction.rs`（新）：上下文构造、输出清洗、`generate_prediction`（仿 title.rs 的单轮流消费）。
- `crates/craft-agent/src/runner.rs`：Runner 预测缓存（内存 HashMap）+ `complete()` 后 spawn 钩子。
- `crates/craft-core/src/contract.rs`：`Settings.input_prediction`（`#[serde(default = "default_true")]`，防老库反序列化失败丢设置）。
- IPC：`inputPredictionGet(threadId)` 三处接线（channels.ts / lib.rs dispatch / services.rs）；设置白名单与默认值同步。
- `src/renderer/src/components/conversation.tsx` Composer：hint state、running 下降沿 5 秒定时、幽灵文本叠层、Tab 分支。
- `src/renderer/src/overlays/settings.tsx` 通用分区开关行。
