# 09 系统能力接线与 capabilities 审计（P8）

labels: [needs-triage]
status: done
depends: 06

## 任务

1. 官方插件接线：clipboard-manager（复制 markdown 走 writeHtml）、notification、dialog（注意单次不能同时选文件与文件夹，工作区选择按需拆分）、opener（外链唯一出口）+ `on_navigation` deny 兜底、single-instance（二次启动聚焦既有窗口）、window-state。
2. `services` 层等价物（src-tauri 内）：设置、会话、对话、提供商、工作区、审计、应用（外链/剪贴板/附件）——只做参数校验、编排与错误转换，领域逻辑不进壳层（终端服务已随 D1 移除，不在接线范围）。
3. IPC 全量接线：通道表 → command 清单；`window.craftwork` 的 TS invoke 封装保持渲染层调用面不变；错误经 IPC 包装后由渲染层 `cleanErrorMessage` 剥壳展示中文。
4. **capabilities 权限审计**：全部 command 逐条显式授权；build.rs 命令清单收紧；fs scope 按需最小化。
5. 数据目录收尾：`backups/`、`attachments/`、`workspace/default` 路径逻辑归一；应用退出时子进程清理（shell_exec 遗留进程）。

## 验收

- 按 AGENTS.md §8 走查清单（移除终端项）全量过一遍（隔离目录）：首次启动 → 添加提供商 → 密钥 → 发消息 → 工具调用 → 审批允许/拒绝 → 数据面板 → 主题切换；
- 外链全部经 opener 打开，无渲染层远程加载；
- capabilities 清单与通道表一一对应，无未授权/过宽授权。

## 进展

- 2026-11-05 复核注记：任务 1 实际落地与描述有出入——外链经自研 `src-tauri/src/system.rs::open_external`（协议白名单校验后交系统浏览器），**未引入 opener 插件**；single-instance 与 window-state 未接线。notification 插件 2026-11-05 补齐接入（审批待决/完成/失败三类、失焦才发、设置开关门控，提交 b983121）；dialog、clipboard-manager 已接。任务文本保留作历史，勿按插件名单找代码。
