# 21: thread 改名/自动命名广播

**labels:** [needs-triage]
**对应对账:** spec.md §一 D15 附属、§二 L18

**What:** thread_rename（services.rs:227）与 auto_title_thread（title.rs → threads.rs 直写 DB）不广播事件，侧栏标题要等 run 终态的 thread.updated 全量刷新才更新，完成时刻与标题写入存在竞态。方案：Recorder 增加 thread.updated（带新 title）广播路径，前端 case 已兼容（app.ts:430-437 会 refreshThreads）。

## 进展

