# 04 模型客户端（P3）

labels: [needs-triage]
status: done
depends: 03

## 任务

1. `craft-model` crate：统一模型客户端**自研平移**（决策 D2，不引入 genai 等）：reqwest（stream feature）+ SSE 状态机直译现有 JS 解析逻辑；OpenAI / Anthropic 双协议适配器；错误归一化到既有错误码（`auth / rate_limit / timeout / context_overflow / …`）；重试与超时策略平移。协议抽象与 wire/transport 分层参考 codex-rs 的 provider 设计（只参考设计，不复制代码，见 plan §6 风险 5）。
2. 模型目录：`assets/catalog/` 快照原样复用（生成物 modelsync.mjs 不受影响），Rust 侧加载 + 用户提供商（settings `model.custom_providers`）合并；`provider/model` 引用解析语义不变。
3. 密钥只在 Rust 侧拼装请求，任何路径不得把密钥传给渲染层。
4. model 包 vitest 用例翻译（错误归一化、目录过滤、SSE 解析用固定样本流）。

## 验收

- mock SSE 样本流的解析/错误映射单测全绿；
- 流式中断、超时、重试路径有单测覆盖；
- 无任何内置提供商分支（AGENTS.md §7 红线）。

## 进展
