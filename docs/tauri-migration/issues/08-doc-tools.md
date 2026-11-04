# 08 文档处理工具（P7）

labels: [needs-triage]
status: pending
depends: 05（可与 06 并行）

## 任务

1. `craft-doc` crate docx：zip + quick-xml 低层路线直译现有 JS XML 逻辑（段落内同格式片段文本替换，写前保留其余 entry 原样）。
2. `craft-doc` crate xlsx：**先做 umya-spreadsheet 3.x 保真度 spike**（含图表/条件格式的真实样本），定案保真路线或退 zip+quick-xml 低层路线；实现 `xlsx_set_cell` 等价语义。
3. `craft-doc` crate pdf：pdf-extract 只读文本层；对现样本评估提取质量，不达标再评估 pdfium-render（需随应用分发动态库，另立项）。mupdf-rs 因 AGPL 排除。
4. 写前备份与 `expectedHash` 防覆盖语义对齐（经 craft-core crate）。
5. 对拍测试：固定真实样本文件集（docx/xlsx/pdf 各若干），断言 Rust 输出与 Electron 版输出文本层等价。

## 验收

- 三类样本对拍测试全绿；
- Work 模式走查：读取 → 替换 → 保存 → 再读取一致；
- spike 结论（xlsx 路线）回写 plan §8 未决问题 1。

## 进展
