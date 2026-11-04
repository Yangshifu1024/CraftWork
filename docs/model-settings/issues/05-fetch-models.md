# 5: 拉取模型

**labels:** [ready-for-agent]
**Blocked by:** 4（模型列表管理）

**What to build:** [拉取模型] 按钮全流程。服务端新增 providerFetchModels：按协议 GET 端点模型列表（OpenAI 系 `{endpoint}/models`、anthropic `/v1/models?limit=1000`，带密钥、10 秒超时），解析模型 ID 列表，与现有模型按 ID 去重合并——已存在的不覆盖，新增默认启用、source 记为 fetched、上下文取响应字段否则用默认值；返回 `{ fetched, added, models }`。UI：进行中按钮 loading 防重复；完成后内联反馈「拉取到 N 个模型，新增 M 个」；失败走既有错误码归因文案（复用连接测试文案映射）；未配密钥时不发请求直接给可读提示。单测 mock fetch 覆盖两种协议响应、去重合并、HTTP 错误、超时。详见 spec §7 拉取模型。

## 验收标准

- [ ] 对 OpenAI 兼容端点拉取成功，新增模型出现在列表且默认启用
- [ ] 重复拉取不产生重复行，已有模型的配置不被覆盖
- [ ] 未配密钥时点击给出可读提示，不发请求
- [ ] HTTP 错误 / 超时显示归因文案；进行中按钮禁用
- [ ] 单测：两种协议解析、去重合并、错误路径

## 进展
