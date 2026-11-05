# 5: 拉取模型

**labels:** [ready-for-agent]
**status:** done
**Blocked by:** 4（模型列表管理）

**What to build:** [拉取模型] 按钮全流程。服务端新增 providerFetchModels：按协议 GET 端点模型列表（OpenAI 系 `{endpoint}/models`、anthropic `/v1/models?limit=1000`，带密钥、10 秒超时），解析模型 ID 列表，与现有模型按 ID 去重合并——已存在的不覆盖，新增默认启用、source 记为 fetched、上下文取响应字段否则用默认值；返回 `{ fetched, added, models }`。UI：进行中按钮 loading 防重复；完成后内联反馈「拉取到 N 个模型，新增 M 个」；失败走既有错误码归因文案（复用连接测试文案映射）；未配密钥时不发请求直接给可读提示。单测 mock fetch 覆盖两种协议响应、去重合并、HTTP 错误、超时。详见 spec §7 拉取模型。

## 验收标准

- [x] 对 OpenAI 兼容端点拉取成功，新增模型出现在列表且默认启用
- [x] 重复拉取不产生重复行，已有模型的配置不被覆盖
- [x] 未配密钥时点击给出可读提示，不发请求
- [x] HTTP 错误 / 超时显示归因文案；进行中按钮禁用
- [x] 单测：两种协议解析、去重合并、错误路径

## 进展

已实现（`adbe162`，并入集成分支后 `e62aace`）。服务端 `providerFetchModels`：按协议 GET（OpenAI 系 `{endpoint}/models`、anthropic `joinServiceURL(endpoint,"/v1/models")?limit=1000`）、密钥缺失不发请求直接可读报错、10 秒超时归因 timeout、`data[]` 容错解析（字符串条目/单条坏数据跳过、结构不符抛「响应格式不符」）、按 ID 去重合并（已存在不覆盖不重排、保留原 enabled）、新增 `enabled: true`/`source: "fetched"`/contextK 换算（数值 token、字符串 128k/1M）缺省 32、合并前重读数据防竞态；`FetchedModelsResult` 契约三处接线；HTTP/网络错误复用 statusError/transportError + CONNECTION_MESSAGES 归因。UI：[拉取模型] 置于模型列表标题行（端点空禁用 + title、进行中扫光防重复、成功内联「拉取到 N 个，新增 M 个」、失败内联红字归因）。11 例新单测（含 fake timers 驱动超时）。全量 165 测试绿 + build。
