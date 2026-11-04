# 03 存储层与密钥（P2）

labels: [needs-triage]
status: done
depends: 02

## 任务

1. `craft-core` crate：Recorder 等价物（事件写入 + 广播配对）、原子写（tempfile + rename）、哈希（sha2）、行级 diff（similar 或直译）、密钥文件读写（`~/.craftwork/keys.json`，unix 0600，语义与现有 `core/secret.ts` 一致）。
2. `craft-store` crate：rusqlite（bundled/json/backup）打开与连接管理；**单写线程**（常驻线程 + mpsc 串行提交，WAL + busy_timeout）；迁移链逐版本翻译（不改动已发布版本语义）；threads / workspaces / events（seq 同事务分配）/ runs / approvals / audit_log / usage / settings / backup 各模块平移。
3. 只读连接用于快照重建与查询，与单写者并存。
4. 数据兼容常驻测试资产：Electron 版生成的脱敏 `craftwork.db` 样本入库，断言「打开 → 迁移 → 快照重建」与 Electron 版一致。
5. vitest store/editor/title/context 用例逐条翻译为 cargo test。

## 验收

- Electron 版创建的数据目录被 Rust 版原位打开，会话列表与消息完整；
- 迁移链测试覆盖全部已发布版本号；
- cargo test 覆盖现有 store 侧全部 vitest 用例语义。

## 进展
