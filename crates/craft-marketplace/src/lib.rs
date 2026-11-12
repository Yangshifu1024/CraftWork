//! 技能商店领域 crate（决策 D12）：市场源清单探测与解析、下载编排（经 craft-git）、
//! 版本化缓存与原子激活、安装 / 升级 / 回滚 / 卸载编排、已安装清单持久化与审计。
//! 行为规格见 D12。
//!
//! 存储与审计说明：两张商店表（迁移 V14）读写封装在 [`store`]，审计入口在 [`audit`]。
//! 两模块只提供接收 `&Connection` 的纯函数——`Db::lock_conn()` 非重入（AGENTS.md
//! 已知坑），取锁一律由调用方完成并传入，crate 内部不自行取锁。
//!
//! 清单探测与解析（[`manifest`]）是纯函数层：仓库快照目录 → 市场模型（四形态按序
//! 判定），零网络零 git 调用，供添加市场源时的形态判定与商店列表装载复用。
//!
//! 缓存布局与原子激活（[`cache`]）同样是纯本地层：staging 拷贝 → SQLite 事务写指针
//! → rename 生效 → 版本裁剪，外加启动时残留清理（spec 第 3 节）；下载编排（clone /
//! fetch，经 craft-git）由后续安装编排接线，激活入口只接收已就位的来源目录路径。
//!
//! 推荐市场源候选清单（[`recommended`]）来自构建期资产
//! `assets/recommended-marketplaces.json`（`include_bytes!` 嵌入），运行时只解析展示，
//! 不自动添加任何源。
//!
//! 商店核心编排（[`orchestrator`]，票 #5）把上述各层串成完整动作：市场刷新、安装、
//! 升级、回滚、卸载、插件启停（全部经应用内互斥串行化、全部落审计），网络下载经
//! `craft-git` 传输（浅克隆 + 超时重试 + 加速回退），代理与加速前缀以中性网络配置
//! （`craft-core::network` 产物）作参数传入。双轴更新检测的判定在 [`update_check`]
//! （纯函数层），编排级取数（probe 快照、读快照 HEAD、逐插件比对）在
//! [`orchestrator::Orchestrator::check_updates`]。

pub mod audit;
pub mod cache;
pub mod manifest;
pub mod orchestrator;
pub mod recommended;
pub mod store;
pub mod update_check;
