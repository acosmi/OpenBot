//! `openbot-desktop` —— Desktop transport：Tauri、window ACL、in-process、sidecar/update。
//!
//! # 所有权边界（v3 §5.1 / §5.2 / §13 / §16.2）
//!
//! 负责（transport 四件事，**只有**这四件，v3 §5.2）：认证、framing、输入大小限制、错误映射。
//!
//! 另外负责：
//!
//! - typed in-process transport（v3 §13.2）：`setup` 里创建一个 `Arc<dyn ApplicationService>`，
//!   普通 request 直接 typed 调用，server request/stream 走有界 channel。
//!   **不复制 Codex 的 initialize / JSON-RPC DTO**；借鉴面仅限 bounded queue、duplicate request
//!   rejection、critical notification、lag visibility、pending cleanup 和 finite shutdown。
//! - 投递等级（v3 §13.2）：terminal / approval / policy decision / server request 不可静默丢，
//!   队列满即显式断开或失败，客户端从 durable cursor replay；text/reasoning delta 可合并但不
//!   改变最终文本；progress/presence 是 latest-value；任意丢弃或合并都产生 metric 与 sequence
//!   gap —— **不能让 GUI 误以为完整**。
//! - window ACL（v3 §13.3）：一个 native broker 可以服务多窗口，但按 window label、actor、
//!   thread subscription 和 auth generation 过滤。window A 永远收不到 window B 的私有 thread、
//!   screen ticket 或 approval。
//! - Tauri capability 按 window label 单独配置；禁止 `windows:["*"]`、宽泛 filesystem、remote
//!   API access；production 禁用 devtools；所有 command 枚举注册并生成审计清单（v3 §13.1）。
//! - sidecar 生命周期与 update（v3 §16.2），含 Desktop 本机 PostgreSQL sidecar 的监管（v3 §14.1）。
//! - `<html class lang>` 首帧改写：Desktop 从本地设置读（CLAUDE.md §4a）。
//!
//! 明确**不**负责：
//!
//! - 实现任何业务规则（v3 §5.2 逐字禁止）。
//! - 让前端自行完成可见性过滤（v3 §13.3：过滤不能由前端自行完成）。
//! - 用 Tauri event 承载画面 —— 持续画面走 loopback binary WebSocket，Tauri Channel 只承载
//!   结构化 Agent/tool/policy 事件（v3 §13.4）。
//! - 加载远程内容：主 WebView 只加载打包本地内容，拒绝 remote navigation；deep link、file
//!   association、clipboard、external URL 一律当不可信输入（v3 §13.1）。
//!
//! # Phase 0 状态
//!
//! 刻意为空，Cargo.toml 里也刻意没有 Tauri 依赖。桌面宿主是 G6 的产物。
