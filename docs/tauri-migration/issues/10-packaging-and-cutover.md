# 10 打包分发与切换清理（P9）

labels: [needs-triage]
status: done
depends: 06、07、08、09 全部完成

## 任务

1. bundler 配置：macOS `app,dmg`、Developer ID 签名 + 公证（复用现有 APPLE_* 凭据）、`dist:dir` / `dist` 等价脚本；NSIS/AppImage 配置备好（实机验证另排期，node-pty 交叉编译障碍已随 D1 消除）。
2. updater 链路备好（决策 D6）：`tauri signer generate` 密钥管理、`createUpdaterArtifacts`、静态 JSON 托管方案（GitHub Releases）；本阶段产出**未签名包**，凭据（Developer ID、APPLE_*、签名密钥）到位后填配置即生效，不改代码。
3. CI：tauri-action 三平台矩阵改造（macOS 必绿，win/linux 允许失败占位），发布产物上传。
4. **切换提交（一次性）**：删除 Electron 主进程（`src/main`）、electron/electron-builder/electron-vite 依赖与构建配置、`postinstall`/`prepare` 脚本；版本号事实源切换（tauri.conf.json 为准，渲染层 package.json 保留）。
5. 文档同步：README（架构与贡献流程，删除终端章节）、AGENTS.md（命令、目录、已知坑——app-region 坑删除、node-pty/懒下载等 Electron 专属坑清理、跨平台留桩更新）、GLOSSARY.md（产品定义中 Electron 与终端表述、Work 模式定义）、CONTRIBUTING.md。
6. 切换后全量验证：`cargo test` + `pnpm typecheck` + `pnpm test` + 打包产物启动走查；确认老数据目录零迁移可读。

## 验收

- 签名公证的 .app 可启动并通过走查清单；
- 切换提交 revert 后 Electron 版仍可构建发布（回滚演练一次）；
- 文档三处同步完成，无 Electron 与终端残留表述。

## 进展
