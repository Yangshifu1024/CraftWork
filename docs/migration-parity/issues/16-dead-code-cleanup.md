# 16: 死通道、死事件分支与死代码清理

**labels:** [needs-triage]
**对应对账:** spec.md §二 L17/L18

**What:** 逐项决策「实装还是删除」，不留在中间态：

- 死通道：workspaceSetActive（语义已被 settingsSave.activeWorkspace 覆盖，删声明或实装独立通道）、workspaceLocateFile（跨根唯一匹配，随票据 10 文件树恢复再定）、catalogLookup
- 死事件分支：settings.updated（外部改设置同步的意图失效，单窗口下可删 case）、tool.started（Rust 以 tool.completed(status:"running") 表达开始，删 case 或 Rust 补发）、thread.updated deleted:true（删 case 或 thread_delete 补广播）
- 死代码：store/app.ts `panelWidth:320` 无消费者、package.json `react-resizable-panels` 已装未用、App.tsx:53 纯装饰分隔条（随票据 10 真分隔条替换）、App.tsx:2 陈旧注释（「含终端/文件/文档」）、threadSearch/threadCreate/threadArchive 等声明+实现齐备但 UI 未用的通道（标注保留原因或删）
- 契约闲置：Envelope 的 run.finished 常量（新旧命名并存，Rust 只发 run.completed/canceled/failed）

## 进展

