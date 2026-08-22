//! `openbot-testkit` —— 测试与闸门工具：golden trace、fault injection、fake provider、xtask。
//!
//! # 所有权边界（v3 §5.1 / §19.3 / §21）
//!
//! 负责：
//!
//! - golden trace 的录制、回放与比对（v3 §21.1 parity / §21.2 Agent-Protocol）；
//!   shadow Agent 只重放录制的 provider stream，**不执行 live tool**（v3 §20.1）。
//! - fault injection 与 fake provider：让 retry / cancel / budget / stall 这些路径在没有真实
//!   厂商端点的情况下也能被确定性覆盖。
//! - golden 截图比对工具面（设计系统文档 §10.4；仅本 crate 使用 `image` `0.25.10` 与
//!   `xcap` `0.9.8`，G6 开工时引入）。
//! - **xtask**：仓库闸门驱动器，落在本 crate 的 bin target `src/bin/xtask.rs`，
//!   `required-features = ["xtask"]`。v3 §5.1 的 crate 表里本 crate 的职责原文就含 "xtask"，
//!   所以这是方案内既定归属，不是本轮新增的第 11 个 crate。
//!
//! 明确**不**负责：
//!
//! - 进入任何发行物。本 crate 是开发期工具，`publish = false`，不被十个业务 crate 依赖。
//! - 实现业务规则或绕开 `openbot-application`：测试也必须穿过 `ApplicationService`（v3 §5.2
//!   逐字把"测试"与 Axum、Tauri、迁移工具并列为只做认证 / framing / 限制 / 错误映射的一方）。
//! - 替代 CI：`xtask ci` 只是把 v3 §16.3 的固定清单按顺序跑一遍，不新增也不删减判据。
//!
//! # Phase 0 状态
//!
//! 库面刻意为空；本轮只落地 bin target `xtask`（`parity-check` 与 `ci` 两个子命令）。
//! golden trace / fault injection / fake provider 随各自闸门（G1 起）逐步落地。
