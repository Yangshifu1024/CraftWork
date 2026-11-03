# 10. 浏览器面板嵌入技术 PoC

Status: needs-triage

Type: research

## 背景

路线图项（proposal 4.6）：浏览器 tab（人工浏览 / 预览本地应用）是 ZCode 侧最常用面板之一，但 ZCode 靠 Electron `<webview>` + main 进程 CDP，Wails3 两者都没有，需要先验证技术路线再立项。本票只做 PoC 与选型结论，不实现面板。

## 待验证的三个候选

1. **iframe 嵌入**：CraftWork 生产 CSP（`internal/app/csp_prod.go`）需放开 `frame-src`；验证目标站点 `X-Frame-Options` / `frame-ancestors` 拒绝率（预计大量站点不可嵌）；验证本地 dev server（localhost）预览全链路；评估 cookie/存储分区与崩溃隔离风险。
2. **Wails3 第二窗口按锚点摆位**：为浏览器 tab 开无边框子 `WebviewWindow`，面板 tab 作锚点，宿主移动/缩放时跟随（ZCode 插件停靠层 `pluginUiPagePlane.ts` 的窗口版）。验证：多窗口 z 序/焦点/遮挡、WKWebView 多实例内存开销、Windows/Linux 窗口行为差异、全屏与最小化时的表现。
3. **系统浏览器外部打开**（现有 `OpenExternal`）：面板只放 URL 工具卡与浏览记录，不内嵌。验证体验下限即可。

## 产出

- PoC 代码（放临时目录，不进 main）+ 结论文档：每候选的可行性、限制清单、推荐路线与理由。
- 若推荐候选 1 或 2：给出 CSP 变更点 / 窗口管理复杂度的量化说明，供立项决策。

## 验收

- 结论能回答：浏览器 tab 立项走哪条路、什么场景做不了、CSP 与安全影响（外链仍只走 `AppService.OpenExternal` 的安全约束不破坏）。

## 依赖

无。建议在 01–05 稳定后启动，避免与 tab 模型落地互相干扰。

## Comments
