# 1: modelsync 修复与快照/logo 重新生成

**labels:** [ready-for-agent]
**Blocked by:** 无（可立即开始）

**What to build:** 修复同步脚本对 models.dev 新数据结构的两处失效，重新生成快照与 logo 资产：字段兼容（`api` 字段优先、`baseUrl` 兜底）、attachment 过滤反转（models.dev 现语义 `true` = 支持附件输入的对话模型；改为保留 attachment 或 tool_call 为 true 者，嵌入/语音/图像类自然滤除）、坏值防御（含 `${` 占位符或 URL 解析失败的端点置空按无端点处理）、覆盖表扩充（无端点且官方地址公开固定的服务补齐）。重新生成后提供商从 193 恢复约 229，anthropic/xai 回归且端点正确，约 208 个提供商导入即带可用端点；192 个 SVG logo 下载、清洗（剔除 script/事件属性/外部引用，仅收 image/svg+xml，单文件超限丢弃）后内嵌为渲染层 logo 资产模块（id → data URL 映射，生成物标注勿手改）。数据层事实与端点形态统计见 spec §0–§3。

## 验收标准

- [ ] `node scripts/modelsync.mjs` 重新生成：提供商数约 229，anthropic/xai 在列且端点正确（anthropic 不含 /v1、deepseek 为 /v1 路径）
- [ ] 端点质量：非空 ≥ 205、全部合法 http(s) URL、无 `${` 坏值残留
- [ ] 过滤正确：嵌入类模型（如 text-embedding-3-small）不进目录；attachment=true 的对话模型完整保留
- [ ] logo 资产模块生成：192 个 SVG 清洗后无 script/事件属性/外部引用，合计体积 < 1MB
- [ ] manifest 与内嵌模块同步更新；`--check` 漂移检查工作正常

## 进展
