# 第三方声明

本项目包含来自第三方项目的代码或数据。以下声明按第三方项目分别列出。

## models.dev

- 项目：https://github.com/anomalyco/models.dev
- 用途：模型提供商与模型目录数据通过 `scripts/modelsync.mjs` 同步、裁剪后内嵌进源码，并随应用构建打包。
- 许可：MIT License

```text
MIT License

Copyright (c) 2025 models.dev

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## 随应用分发的核心第三方运行时

以下组件以 npm 依赖形式随应用构建产物分发，均采用 MIT License；许可全文见各项目仓库或 npm 包内声明。

| 组件 | 用途 | 许可 | 项目 |
| --- | --- | --- | --- |
| Electron | 桌面外壳（主进程 / 渲染进程 / 预加载桥） | MIT | https://github.com/electron/electron |
| node-pty | 终端面板的真实 PTY（原生模块） | MIT | https://github.com/microsoft/node-pty |
| @xterm/xterm | 终端前端渲染 | MIT | https://github.com/xtermjs/xterm.js |
| @xterm/addon-fit | 终端尺寸自适应插件 | MIT | https://github.com/xtermjs/xterm.js |

其余 npm 依赖的完整清单见根目录 `package.json`，各自的许可以其 npm 包声明为准。
