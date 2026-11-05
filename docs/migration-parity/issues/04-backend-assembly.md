# 4: 装配断接通——档位热切换、失败转移、往返上限、压缩守卫

**labels:** [ready-for-agent]
**对应对账:** spec.md §一 D7~D9、D18

**What to build:**

1. **档位热切换**：`services.rs settings_save` 持久化后调 `craft_policy::engine::Engine::set_level`（解析路径与 `lib.rs:77-83` 启动初始化一致），确认门后档位即时不重启生效；补 services 测试（save → 查询引擎档位）。
2. **失败转移**：`lib.rs build_app_state` 装配 `factory.set_fallback_resolver`，闭包按当前 settings 的 fallbackModel 解析（签名对齐 `craft-model/src/factory.rs:119-155` 与其测试；fallback 为空/不存在/停用/密钥缺失时安全返回空串即不转移）。
3. **往返上限**：`runner.set_max_iterations_resolver` 读 settings `max_iterations`（0 → DEFAULT_MAX_ITERATIONS）。
4. **压缩守卫**：`craft-agent/src/context.rs plan_compression` 对窗口 <=0 直接跳过（防 context_k=0 模型每轮压缩到只剩 MIN_KEEP 条）；补单测。

## 验收标准

- [ ] 设置页切换审批档位，不重启，下一次工具审批即按新档位处置
- [ ] settings 配 fallbackModel 且主模型不可用时：run 转移到备选并出 notice；fallback 无效时不转移不出错
- [ ] 「最大工具轮数」改为小值实测生效（循环到上限报错）
- [ ] `pnpm test:rust`、`pnpm lint:rust` 通过

## 进展

