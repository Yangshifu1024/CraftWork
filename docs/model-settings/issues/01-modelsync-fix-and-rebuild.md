# 1: modelsync 修复与快照/logo 重新生成

**labels:** [ready-for-agent]
**Blocked by:** 无（可立即开始）

**What to build:** 修复同步脚本对 models.dev 新数据结构的两处失效，重新生成快照与 logo 资产：字段兼容（`api` 字段优先、`baseUrl` 兜底）、attachment 过滤反转（models.dev 现语义 `true` = 支持附件输入的对话模型；改为保留 attachment 或 tool_call 为 true 者，嵌入/语音/图像类自然滤除）、坏值防御（含 `${` 占位符或 URL 解析失败的端点置空按无端点处理）、覆盖表扩充（无端点且官方地址公开固定的服务补齐）。重新生成后提供商从 193 恢复约 229，anthropic/xai 回归且端点正确，约 208 个提供商导入即带可用端点；192 个 SVG logo 下载、清洗（剔除 script/事件属性/外部引用，仅收 image/svg+xml，单文件超限丢弃）后内嵌为渲染层 logo 资产模块（id → data URL 映射，生成物标注勿手改）。数据层事实与端点形态统计见 spec §0–§3。

## 验收标准

- [x] `node scripts/modelsync.mjs` 重新生成：提供商数约 229，anthropic/xai 在列且端点正确（anthropic 不含 /v1、deepseek 为 /v1 路径）
- [x] 端点质量：非空 ≥ 205、全部合法 http(s) URL、无 `${` 坏值残留
- [x] 过滤正确：嵌入类模型（如 text-embedding-3-small）不进目录；attachment=true 的对话模型完整保留
- [x] logo 资产模块生成：192 个 SVG 清洗后无 script/事件属性/外部引用，合计体积 < 1MB
- [x] manifest 与内嵌模块同步更新；`--check` 漂移检查工作正常

## 进展

- 2026-11-04：完成（提交 85e6733）。实测：229 提供商 / 7846 模型，anthropic 端点 `https://api.anthropic.com`、xai `https://api.x.ai/v1`、deepseek `https://api.deepseek.com/v1`；端点非空 219（10 个无端点均为坏值防御置空或依赖账户/租户/区域的网关：neon/infomaniak/cloudflare-workers-ai/snowflake-cortex/databricks 含 `${VAR}` 占位符置空，cloudflare-ai-gateway/watsonx/qvac/gitlab/sap-ai-core 无公开固定地址保持空走 UI 引导）；覆盖表补 salad-cloud/aihubmix/vercel；logo 实际 229 个全下载成功（models.dev 已补齐覆盖率，高于 spec 实测的 192），清洗后合计 563KB（< 1MB），抽样解码无 script/事件属性/外部引用；顺带修复既有缺陷：sha256 校验和把 generatedAt 时间戳算入导致 `--check` 恒报漂移，现与 version 统一只基于 providers 内容，连续两次 `--check` 均报无漂移。`pnpm typecheck`、`pnpm test`（84 项）通过。