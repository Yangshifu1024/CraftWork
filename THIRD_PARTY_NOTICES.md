# Third-Party Notices

This project includes code or data from third-party projects. The notices are listed per project below.

## models.dev

- Project: https://github.com/anomalyco/models.dev
- Use: model provider and model catalog data is synced via `scripts/modelsync.mjs`, trimmed, embedded into the source, and packaged with the app build.
- License: MIT License

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

## Core third-party components distributed with the app

The following components ship with the app build; see each project's repository or in-package declaration for the full license text.

| Component | Use | License | Project |
| --- | --- | --- | --- |
| Tauri | Desktop app shell (Rust side, shipped in the binary) | MIT / Apache-2.0 | https://github.com/tauri-apps/tauri |
| tauri-plugin-dialog | System dialogs | MIT / Apache-2.0 | https://github.com/tauri-apps/plugins-workspace |
| tauri-plugin-clipboard-manager | Clipboard | MIT / Apache-2.0 | https://github.com/tauri-apps/plugins-workspace |
| tauri-plugin-single-instance | Single-instance enforcement | MIT / Apache-2.0 | https://github.com/tauri-apps/plugins-workspace |
| tauri-plugin-notification | System notifications | MIT / Apache-2.0 | https://github.com/tauri-apps/plugins-workspace |
| React / react-dom | UI framework (shipped with the webview assets) | MIT | https://github.com/facebook/react |

The full list of Rust-side dependencies is in the root `Cargo.lock`, and the npm side is in `package.json`; each dependency's license is governed by its own in-package declaration.
