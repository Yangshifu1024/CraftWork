# 3: 供应商详情卡：连接配置与启停

**labels:** [ready-for-agent]
**status:** done
**Blocked by:** 2（设置页主从布局与供应商列表）

**What to build:** 右栏详情卡完整可操作：端点常显编辑（blur/Enter 保存、服务端校验错误内联红字、空端点必填 warning 态）；协议创建后只读展示（改协议等于删除重建）；密钥 masked 展示 + 设置（明文/密文切换）/清除（需确认）；测试连接按钮 + 内联结果反馈（绿字 + 延迟 / 红字 + 归因文案，进行中 loading 防重复点击）；供应商启用开关（服务端 PersistedProvider 新增 enabled 字段、旧数据缺省读为 true，新增 providerSetEnabled，禁用后状态点转灰、详情区降透明度）；名称内联改名（Enter 提交 / Esc 取消、IME composing 不触发，新增 providerRename）；⋯菜单（测试连接 / 删除，删除走确认并提示连带密钥与模型数）。所有 mutation 返回刷新后的 ProviderView（免二次拉取）。交互细节见 spec §6、§8、§10。

## 验收标准

- [x] 端点修改保存生效；校验失败内联报错；清空后必填 warning 出现、恢复后消失
- [x] 密钥设置后显示 masked、明文切换可用；清除需确认且状态点转橙
- [x] 测试连接成功/失败/进行中三态内联展示，进行中按钮禁用
- [x] 启用开关关闭后状态点变灰、详情区降透明度，重开恢复
- [x] 改名 Enter 提交 Esc 取消；中文输入法候选确认 Enter 不触发提交
- [x] 删除走确认弹窗，确认后提供商与密钥一并删除
- [x] 单测：enabled 缺省迁移（旧数据读为 true）、providerSetEnabled / providerRename 成功与报错路径、mutation 返回 ProviderView

## 进展

已实现（`c68cd8f`）。服务端：`PersistedProvider.enabled`（旧数据缺省补 true）；`providerSetEnabled`/`providerRename` 三处接线，mutation 返回刷新 `ProviderView`；`toView` 输出 enabled/docUrl；`buildProvider` 编辑时保留 enabled（防端点编辑复活禁用项）。UI：内联改名（isComposing + keyCode 229 双重 IME 守卫）、端点 blur/Enter 保存 + 空值必填 warning 态、密钥 masked 展示 + 明文切换 + 清除确认、测试连接内联三态（进行中扫光 + 防重复）、启停开关（停用详情区 opacity-60 可逆、状态点经 providerStatus 转灰）、⋯菜单（DropdownMenu）+ 删除确认（文案含连带密钥与模型数）。`tests/providers-service.test.ts` 10 例（真实 SQLite 临时目录）。typecheck + 103 例 + build + dist:dir 全绿。GUI 细节观感待票 8 走查复核。
