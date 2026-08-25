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
//! - `<html class lang>` 首帧改写：`tauri_host` 从 [`preferences`] 本地设置读（CLAUDE.md §4a）。
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
//! # G1 状态（Rust Foundation，W5–10）
//!
//! 本 crate 在 G1 只落 §13.2 的 **typed in-process 层**，其余归后续 gate：
//!
//! | 模块 | 内容 | 方案出处 |
//! | --- | --- | --- |
//! | [`budget`] | §13.2 逐字规定的五个默认队列 / 合并 / deadline 常量，与投递等级分类函数 | §13.2 |
//! | [`window`] | [`WindowIdentity`] / [`EventScope`] / [`ThreadSubscriptions`] —— §13.3 的四个过滤维度 | §13.3 |
//! | [`event`] | [`AppEventRef`]、序号与 [`SequenceGap`]、接收端自检器 [`SequenceTracker`] | §13.2 |
//! | [`broker`] | [`EventBroker`]：一个 broker、多窗口、每窗口有界队列与显式断开 | §13.2 / §13.3 |
//! | [`session`] | [`DesktopSession`]（§13.2 给出的骨架，逐字段保留） | §13.2 |
//! | [`cancel`] | [`CancellationToken`] 与 [`SHUTDOWN_DEADLINE`] | §13.2 |
//! | [`transport`] | [`InProcessTransport`]：把 `Arc<dyn ApplicationService>` 包成 in-process 通道 | §13.2 / §5.2 |
//!
//! **尚未实现**（不要冒充）：可发布 Tauri binary/tauri.conf/capability 清单与真实窗口生命周期
//! assembly（G6）、sidecar/update（§16.2）、screen loopback binary WebSocket（§13.4/G7）。
//! Batch 16 已落 opt-in Tauri 2.11.5 custom-protocol adapter、本地偏好原子文件与首帧改写，
//! 但其许可/RustSec/cargo-vet delta 仍红，不能据此勾 Desktop/G6 整关。
//!
//! # G1 默认路径不引 Tauri 本体（主控裁决，2026-08-22）
//!
//! §24 的 G1 判据是「ApplicationService 经 Axum/Tauri 结果一致」。它要证明的事情只有一件：
//! **没有任何业务规则住在 transport 里**。承载这条风险的是 §13.2 的 typed in-process 层
//! —— 有界队列、投递等级、取消、finite shutdown；而不是 Tauri 的窗口与打包外壳，那属于
//! G6（W11–28）。
//!
//! 此刻引 `tauri` 只会给每条 CI 腿加上 WebView2 / 系统 WebKit 依赖，却不增强那条判据。
//! 所以默认 feature 仍是**纯 Rust的 in-process transport**；G6 后续只在显式
//! `tauri-host` feature 下钉 Tauri/Wry，不改变这条默认路径。
//!
//! ## G6 接 Tauri 时接在哪一层
//!
//! 接在 [`DesktopSession`] **之上**，本模块以下的东西一行都不用改：
//!
//! 1. `tauri::Builder::setup` 里构造一次 [`InProcessTransport`]，放进 managed state；
//! 2. 每个 `WebviewWindow` 起来时，用它的 `label()` 造 [`WindowLabel`]，调
//!    [`InProcessTransport::open_session`] 拿到一个 [`DesktopSession`]；
//! 3. 一个任务把 `session.events` 抽干，把每个 [`AppEventRef`] 转成一帧结构化事件写进
//!    该窗口的 `tauri::ipc::Channel`（§13.4：Channel 只承载结构化事件，画面另走 loopback
//!    binary WebSocket）；
//! 4. `#[tauri::command]` 只做一件事：把 typed [`AppCommand`](openbot_contracts::command::AppCommand)
//!    交给 [`InProcessTransport::execute`]。窗口关闭时 drop 掉 [`DesktopSession`]，
//!    生产侧会在下一次投递时看见 [`DeliveryOutcome::ReceiverGone`] 并停下来。
//!
//! 也就是说，Tauri 在这条链路上只提供**窗口与 IPC 载体**；ACL、有界队列、序号、丢帧
//! 可见性、关停 deadline 全部在 Tauri 之下已经完成。这正是「过滤不能由前端自行完成」
//! （§13.3）在 G6 仍然成立的原因。
//!
//! # 认证：G1 刻意没有
//!
//! [`WindowIdentity`] 只能由一个既有的 [`AuthContext`](openbot_contracts::auth::AuthContext)
//! 绑定而来（[`WindowIdentity::bind`]），本 crate **不铸造** `AuthContext`，也不提供任何
//! “默认放行”的构造。Desktop 的真实身份来源是本机 session（G2：`AuthContextBuilder::
//! from_verified_session` 由 G2 的本机 session 校验调用），届时接在
//! [`InProcessTransport::open_session`] 的调用点上 —— 本模块的签名不需要变。
//!
//! 测试身份走 [`testing`]，由 `cfg(test)` 或 `testkit` feature 门起来，默认关。

// 公开面即契约面：一个没有文档的公开条目等于一个只有作者知道语义的契约。
// 与 `openbot-application` 同口径，用 deny 而不是 warn。
#![deny(missing_docs)]

pub mod broker;
pub mod budget;
pub mod cancel;
pub mod event;
pub mod preferences;
pub mod session;
pub mod transport;
pub mod window;

#[cfg(feature = "tauri-host")]
pub mod tauri_host;

#[cfg(any(test, feature = "testkit"))]
pub mod testing;

pub use broker::{
    DeliveryMetrics, DeliveryOutcome, DisconnectReason, EventBroker, PublishRejected,
    PublishReport, WindowAlreadyOpen, WindowDelivery,
};
pub use budget::{
    COMMAND_QUEUE_CAPACITY, CRITICAL_EVENT_QUEUE_CAPACITY, DeliveryClass,
    TOKEN_DELTA_COALESCE_BYTES, TOKEN_DELTA_COALESCE_WINDOW, delivery_class,
};
pub use cancel::{CancellationToken, SHUTDOWN_DEADLINE};
pub use event::{
    AppEventRef, BrokerEvent, FramePayload, GapCause, SequenceError, SequenceGap, SequenceTracker,
    TERMINAL_FRAME_RESERVE,
};
pub use preferences::DesktopUiPreferenceStore;
pub use session::{DesktopSession, event_of};
pub use transport::{InProcessTransport, OpenSessionError, ShutdownReport};
pub use window::{
    EventScope, FilterReason, ScopeTarget, ThreadSubscriptions, WindowIdentity, WindowLabel,
};

#[cfg(feature = "tauri-host")]
pub use tauri_host::{
    DesktopTauriProtocol, TauriHostError, detect_os_locale, register_tauri_protocol,
};
