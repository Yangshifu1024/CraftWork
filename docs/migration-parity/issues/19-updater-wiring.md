# 19: updater 实接（决策 D6 凭据后置）

**labels:** [needs-triage]
**对应对账:** spec.md §二 L15

**What:** 决策 D6 声称「updater 链路配置备好、待凭据」，实际连 tauri-plugin-updater 都未接入（Cargo.toml/tauri.conf.json 无插件与 pubkey；craft-core 亦无清单签名校验）。凭据到位后：接入 tauri-plugin-updater + 签名密钥、迁移旧 internal/update 的 Ed25519 清单校验语义（HTTPS 强制、SHA-256、渠道白名单）或改用 Tauri updater 内置签名、设置页「自动更新/渠道」字段接活、release.yml 产出签名更新清单。旧实现参考 `git show adcebf5~1:internal/update/update.go`。

## 进展

