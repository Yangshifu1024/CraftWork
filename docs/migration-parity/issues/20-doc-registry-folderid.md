# 20: craft-doc Format 注册表与 folderId 目标根语义

**labels:** [needs-triage]
**对应对账:** spec.md §二 L16

**What:** 两个结构级平移欠账（功能无损）：

- craft-doc `lib.rs:48-88` doc_kind/read_file/replace 以 if-else+match 分发，旧 Go internal/doc/formats.go 是 Format 适配器注册表（新增 Markdown/PPTX 只需实现+注册）——恢复 trait 注册表结构。
- services.rs 的 workspaceListDir/ReadFile/WriteFile 均 `let _ = folder_id;` 按全根解析（旧 Go 以 folderID 限定目标根、空值为主目录）——语义弱化：跨根同名路径的读写目标不确定性。恢复 folderId 限定（定位用 folderId→roots[index]），workspaceLocateFile（票据 16 决策）可作为跨根查找的正解配套。

## 进展

