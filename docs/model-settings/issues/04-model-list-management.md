# 4: 模型列表管理

**labels:** [ready-for-agent]
**status:** done
**Blocked by:** 3（供应商详情卡：连接配置与启停）

**What to build:** 模型列表区完整管理。每行：名称 + ID（等宽小字，名称与 ID 相同时不重复显示）+ 上下文徽章（Intl compact 格式化：128K / 1M，title 完整数）+ 能力徽章（视觉 / 推理，仅非默认能力显示）+ 编辑 / 删除 / 启用开关 + 行内错误（配置不完整时行下红字）；空态虚线框引导。增改共用弹窗：模型 ID（catalog 来源只读）/ 显示名 / 上下文窗口 K / 最大输出 tokens / 能力勾选；保存成功才关窗、失败内联标错、保存中锁定正文。服务端增量 API：providerSetModelEnabled / providerAddModel / providerUpdateModel / providerRemoveModel，统一「读 → 校验（复用既有单模型校验与 ID 唯一性）→ 写 → 目录 reload → 返回 toView」；PersistedModel 新增 enabled（缺省 true）与 source 取值 fetched（本票仅预留枚举）。详见 spec §6 模型列表区、§7 模型增改弹窗、§8。

## 验收标准

- [x] 每行徽章与开关正确渲染；禁用模型行视觉弱化但不隐藏
- [x] 新增模型：ID 为空 / 含空白 / 提供商内重复 / 上下文超限时内联标错；成功后列表即时更新
- [x] 编辑 catalog 来源模型时 ID 只读，其余字段可改并保存生效
- [x] 删除模型需确认，确认后从列表消失
- [x] 单测：四个增量 API 成功与报错路径（重名、不存在）；上下文长度格式化函数

## 进展

已实现（`952af95`）。服务端：`PersistedModel.enabled`（提供商与模型两级缺省迁移补 true）；`providerSetModelEnabled`/`providerAddModel`/`providerUpdateModel`/`providerRemoveModel` 四通道三处接线（统一 `mutateModel`：读→校验→写→reload→返回 toView；updateModel ID 不可变并保留 enabled/source/cost 等非表单字段；providerSave 全量保存按模型 ID 保留既有启停）；`ModelView` + enabled/source。UI：模型行 = 名称 + ID（同名不重复）+ 上下文徽章（`lib/format-context.ts` 的 formatContextK，Intl compact en-US）+ 视觉/推理徽章 + 编辑/删除（confirm）/启停开关（禁用行 opacity-50 弱化不隐藏）；增改共用弹窗（catalog 来源 ID 只读、保存成功才关窗、服务端错误内联、保存中锁定、Esc 过 IME 守卫）。38 例新单测（全量 143 绿）+ build。
