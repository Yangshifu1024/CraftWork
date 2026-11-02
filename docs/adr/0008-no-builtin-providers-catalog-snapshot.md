# ADR-0008：不内置任何提供商，模型目录来自 models.dev 构建期快照

- 状态：Accepted
- 日期：2026-10

## 背景

模型市场变化极快，硬编码提供商与模型清单意味着频繁发版；而「首次启动即有默认提供商」会带来分发合规与误导用户的问题（隐含推荐、密钥流向暗示）。

## 决策

- **应用不内置任何提供商**：首次启动提供商列表为空；发送消息前必须由用户添加提供商并选择模型。
- 提供商与模型清单**只来自 models.dev 的构建期快照**（`internal/model/catalogdata/catalog.json.gz`），由 `cmd/modelsync` 在开发或发版前同步生成；运行时只读嵌入，不访问网络。
- 只允许三种协议：`openai`、`anthropic`、`openai-compatible`；目录同步规则排除 Google / Gemini 原生协议、Azure、Bedrock、Vertex 与模板化端点。兼容网关里的 Gemini 模型仍可导入（走 OpenAI 兼容协议）。
- **禁止在 Go 代码里新增内置提供商或模型清单**。
- 用户提供商保存在 `settings.model.custom_providers`；线程与设置保存 `providerID/modelID` 引用（同一模型 ID 可出现在多个提供商下，解析必须用引用而非裸 ID）。
- 目录只是候选列表，导入由用户触发；价格仅用于用量估算并标注「估算，非账单」。

## 后果

- 不发版也能通过重新同步目录获得新模型。
- 应用与任何模型厂商无利益关联，符合「密钥与数据只流向用户配置的服务」的本地优先承诺。
- 目录漂移由 CI 检查（`models:check`）；`catalog.json.gz` 是生成物，不得手改。
- 旧数据兼容：`model.Resolve` 保留按原始 ID 的回退，仅用于旧会话。
