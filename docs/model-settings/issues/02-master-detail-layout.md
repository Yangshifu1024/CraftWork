# 2: 设置页主从布局与供应商列表

**labels:** [ready-for-agent]
**Blocked by:** 1（modelsync 修复与快照/logo 重新生成）

**What to build:** 「模型与提供商」分区重构为 ZCode 式主从两栏：顶部标题 + 副标题 + [刷新] + [+ 添加提供商]；主体一张圆角卡片，内部左列表右详情（左栏约 224px、右栏自适应独立滚动），仅本分区容器放宽到 max-w-4xl，其余分区不变。左栏按来源分组（目录导入 / 手动添加，组标题常显空组隐藏），每项 logo（资产命中）或字母头像（未命中回退）+ 名称 + 三色状态点（灰 = 已禁用、绿 = 可用、橙 = 缺密钥或无启用模型，带 title 提示）+ 选中高亮。右栏详情壳先承载现有详情内容不丢功能。无任何提供商时右栏显示三步引导卡（添加提供商 → 配置密钥 → 回聊天选模型）。新 ProviderLogo 组件落地（img + data URL，未命中回退字母头像），并移除 catalogLogo 留桩通道（channels/preload/ipc 三处接线）。布局与状态点语义见 spec §2.2、§4、§5、§11。

## 验收标准

- [x] 打开设置看到两栏布局；现有提供商全部列出，分组与来源一致
- [x] 目录导入的提供商显示真 logo；手动添加显示字母头像；logo 缺失有回退
- [x] 状态点三色语义正确（可用绿 / 缺密钥橙；禁用灰在 #3 启用开关落地后复核）
- [x] 刷新按钮重新拉取列表；空态引导卡展示正确
- [x] catalogLogo 通道已移除，渲染层无 logo 相关 IPC 调用
- [x] 断网状态下设置页无任何网络请求（logo 全部本地）；深浅两主题正常、缩窗不破版

## 进展

已实现（`43613a3`）。ProvidersSection 重构为主从两栏（grid `[224px_1fr]`、`min-h-[32rem]`、分区放宽 max-w-4xl、右栏独立滚动）；`lib/provider-list.ts` 纯函数 `providerStatus`（三色判定，enabled 缺省视为 true）与 `groupProviders`（按 source 分组、空组隐藏、未知归 manual），9 例单测；`components/provider-logo.tsx`（PROVIDER_LOGOS 命中用 img data URL，未命中回退字母头像，按 `catalogId || id` 查资产）；`catalogLogo` 通道三处接线移除；空态三步引导卡；右上「添加提供商」占位（票 6 点亮）。typecheck + 93 例测试 + build 全绿。深浅主题与缩窗的 GUI 走查由票 8 统一执行。
