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

## 随应用分发的核心第三方组件

以下组件随应用构建产物分发；许可全文见各项目仓库或包内声明。

| 组件 | 用途 | 许可 | 项目 |
| --- | --- | --- | --- |
| Tauri | 桌面应用壳（Rust 侧，随二进制分发） | MIT / Apache-2.0 | https://github.com/tauri-apps/tauri |
| tauri-plugin-dialog | 系统对话框 | MIT / Apache-2.0 | https://github.com/tauri-apps/plugins-workspace |
| tauri-plugin-clipboard-manager | 剪贴板 | MIT / Apache-2.0 | https://github.com/tauri-apps/plugins-workspace |
| tauri-plugin-notification | 系统通知 | MIT / Apache-2.0 | https://github.com/tauri-apps/plugins-workspace |
| React / react-dom | 界面框架（随 webview 资源分发） | MIT | https://github.com/facebook/react |

Rust 侧依赖的完整清单见根目录 `Cargo.lock`，npm 侧见 `package.json`；各自的许可以其包内声明为准。
