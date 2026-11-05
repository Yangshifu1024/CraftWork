# 17: 通知点击聚焦并跳转会话

**labels:** [needs-triage]
**对应对账:** spec.md §二 L13；依赖票据 05

**What:** tauri-plugin-notification 桌面端点击事件/载荷支持有限（on_action 主要在移动端）。评估：macOS 通知点击默认激活应用已满足「聚焦」；「跳转到对应会话」需把 threadId 带入通知载荷并在激活/点击回调中消费（tauri RunEvent::Notification? 或插件 action id）。若插件不支持，评估重聚焦事件里带 last-notify-thread 间接实现。旧语义参考 mygo 899429f（NewNotification 点击聚焦并跳会话）。

## 进展

