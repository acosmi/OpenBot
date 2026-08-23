//! `openbot-server` —— Server transport：Axum HTTP/SSE/WS、static GUI、health/readiness。
//!
//! # 所有权边界（v3 §5.1 / §5.2 / §15 / §16.1）
//!
//! 负责（transport 四件事，**只有**这四件，v3 §5.2）：
//!
//! 1. 认证：从 session cookie / IdP 结果构造 `AuthContext`，交给 `openbot-application`。
//! 2. framing：HTTP / SSE / WebSocket 的编解码。
//! 3. 输入大小限制。
//! 4. 错误映射：把 `AppError` 映射成 v3 §15.3 的固定语义 —— 未登录 401；角色不足 403；
//!    资源不可见统一 404（防枚举）；policy refusal 403 + stable code；stale generation /
//!    lease 冲突 409；空 thread history 200 + 空列表。文案可本地化，code / status / audit
//!    类型不变。
//!
//! 另外负责：提供与 Desktop **相同**的静态 GUI bundle（v3 §13.1）、health / readiness 探针。
//!
//! 明确**不**负责：
//!
//! - 实现任何业务规则（v3 §5.2 逐字禁止）。所有 use case 都必须穿过
//!   `openbot-application::ApplicationService`。
//! - 接受自由 method string、renderer 自报角色、renderer 自报 `principal=admin`、任意数据库
//!   query（v3 §5.2 四条逐字禁止项）。
//! - 把 computer token 交给浏览器（v3 §12.4）；Server 的 screen viewer 用同源 `wss` +
//!   session cookie + CSRF-style origin check。
//!
//! # Phase 0 状态
//!
//! 刻意为空。95 条上游 handler 的落点在 `parity/routes.yaml` / `parity/api.yaml` 里逐条归类
//! （v3 §19.3），实现是 G1 之后各闸门的产物。
