//! `openbot-application` —— **唯一业务入口**：所有 use case 的收口点。
//!
//! # 所有权边界（v3 §5.2 Hexagonal ownership / CLAUDE.md §4）
//!
//! 负责：
//!
//! - 暴露 typed service `ApplicationService`，两个方法：
//!   `execute(auth, command) -> Result<AppReply, AppError>` 与
//!   `subscribe(auth, request) -> Result<AppEventStream, AppError>`。
//! - 编排 use case：调 `openbot-domain` 做纯判定，调 `openbot-infra` / `openbot-agent` /
//!   `openbot-computer` 的 port 执行 effect，划事务边界。
//! - 工具执行管线（v3 §8.1）的顺序保证：validation → 权威 actor/target → effect 分类 →
//!   CEL + 内容策略 → 审批 → **事务写 decision + attempt** → 单次 capability → 执行 →
//!   outcome + commit_state。decision 写失败即不执行；执行了但 outcome 写不进去 →
//!   `ReconciliationRequired`，不自动重试。
//!
//! 明确**不**负责：
//!
//! - 认证本身、传输 framing、输入大小限制、错误到 HTTP/IPC 的映射 —— 那是 `openbot-server`
//!   与 `openbot-desktop` 这两个 transport 的活。
//! - 反过来：**任何 transport 都不得各自实现业务规则**（v3 §5.2）。Axum、Tauri、测试和迁移
//!   工具都只能穿过本 crate。
//! - 接受自由 method string、renderer 自报角色、renderer 自报 `principal=admin` 或任意数据库
//!   query —— 这四条在 v3 §5.2 是逐字禁止项。
//! - 用户可见文案与本地化（CLAUDE.md §4a：文案不进 domain / application）。
//!
//! # Phase 0 状态
//!
//! 刻意为空。`ApplicationService` trait 的落地是 G1 的产物。
