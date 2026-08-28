//! `openbot-computer` —— 计算机控制层：manager、supervisor、browser protocol、screen、file/shell。
//!
//! # 所有权边界（v3 §5.1 / §10 / §11 / §12）
//!
//! 负责：
//!
//! - computer manager 与 supervisor：生命周期、`ComputerGeneration` 递增、崩溃与孤儿回收。
//! - `ComputerSecurityScope`（v3 §10.1）：`bot_id` 不足以隔离，scope 是多 Bot / 多用户隔离的
//!   真源；engine 只能看到 v3 §10.2 允许它看到的东西。
//! - browser engine 协议（v3 §11.2）：**单一 engine**（v3 §11.1），Electron `43.3.0` /
//!   Chromium `150.0.7871.212`（CLAUDE.md §3）。不提供自由 CDP 通道。
//! - screen：screencast 帧契约（v3 §12.3）、viewer ticket 校验（v3 §12.4）、`BrowserInput` union
//!   与 `HumanLease` epoch（v3 §12.5）。lease transfer / release / expiry / navigation /
//!   computer restart 都递增 epoch，旧 input 即使在 socket buffer 里也被拒绝。
//! - file / shell effect 的受控执行面。
//!
//! 明确**不**负责：
//!
//! - 决定某次操作允不允许 —— policy 判定在 `openbot-domain`，管线顺序在 `openbot-application`。
//! - 直接对外提供 HTTP/WS 端点 —— Server 面在 `openbot-server`，Desktop 面在 `openbot-desktop`。
//!   （screen 的 loopback binary WebSocket 属于 v3 §13.4 明示的独立数据通道，仍由 transport
//!   crate 挂载，本 crate 只提供帧源与 ticket 校验。）
//! - 把 computer token 交给浏览器（v3 §12.4 逐字禁止）。
//! - 相信 browser engine 回传的 scope / actor / generation 字段（v3 §5.3）。
//!
//! # 当前实施边界
//!
//! Batch51 starts the G5 implementation with the closed browser/input protocol and the
//! authority-owned HumanLease state machine. The Electron/Chromium process, authenticated engine
//! framing, screen hub, file/shell executors and supervisor remain separate unfinished boundaries;
//! protocol types alone are not evidence that an operation executed.

pub mod browser;
pub mod control;

pub use control::{
    ControlError, ControlHolder, ControlService, ControlSnapshot, HumanInputTicket,
    HumanLeaseEpoch, PendingSecretTarget,
};
