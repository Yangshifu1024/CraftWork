# 02 终端移除、应用壳与渲染层平移（P1）

labels: [needs-triage]
status: done
depends: 01

## 任务

1. **先移除终端功能（决策 D1）**：渲染层 `src/` 由两版共享，故在平移前于当前 Electron 版同步移除——删除终端 UI 组件、zustand 终端状态、通道表中终端方法、preload 对应项、`node-pty` 依赖、`services/terminal.ts`、`src/main` 终端接线；右侧面板移除终端标签后保留「文件 / 文档」两个标签、默认标签改为「文件」（决策 D4）；按文档同步约定更新受影响表述。
2. `src-tauri/` 应用壳：tauri.conf.json（窗口尺寸/标题/最低系统版本）、capabilities 骨架、CSP 配置（`connect-src` 含 `ipc: http://ipc.localhost`）、devUrl 指向现有 Vite dev server。
3. Vite 四件套接线（beforeDevCommand / devUrl / frontendDist / beforeBuildCommand），构建目标改 `es2021` + `safari13`；electron-vite 配置暂保留（并行期两套共存）。
4. 标题栏重做：`data-tauri-drag-region` 逐元素模型替换 `-webkit-app-region`（注意：现有「no-drag 挖洞」结构反向，属性只挂标题条，内部控件天然可点）。
5. 渲染层全量走查（macOS WKWebView 基线）：空状态、深浅主题 token、streamdown 流式渲染与 KaTeX 字体资产、命令面板、`prefers-reduced-motion`、中文输入法。

## 验收

- 两个版本均无终端残留（通道表、状态、依赖、目录零引用）；
- `pnpm tauri dev` 启动即见完整界面，主题切换、流式 Markdown 渲染正常；
- 中文 IME 在输入框正常；Electron 版 `pnpm dev` 仍可用作对照。

## 进展
