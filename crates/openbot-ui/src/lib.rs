//! `openbot-ui` —— GUI：Leptos CSR/WASM 的唯一实现。
//!
//! # 所有权边界（v3 §5.1 / §13.1 + 设计系统文档 §13）
//!
//! 负责：
//!
//! - Leptos CSR/WASM 组件树：primitives（设计系统文档 §6.1 的 21 个原语）、features
//!   （§6.2 的 45 个业务组件，十组）、shell（App shell、路由表、五个 layout）、
//!   theme / i18n / clock / 生成物 icons.rs 与 tokens.rs。
//! - **同一份 bundle 两个宿主**：Server 由 Axum 静态提供，Desktop 由 Tauri custom protocol
//!   提供（v3 §13.1）。不维护 React 第二 GUI。
//! - 视觉契约：token 只从 `design/tokens.toml` 生成（CLAUDE.md §4a）；组件只用 token utility，
//!   禁止字面颜色 / 任意值 / 主题前缀变体；class 必须是源码里的**完整字面量**，禁止
//!   `format!("bg-{}", x)` 这类运行时拼接（设计系统文档 §12.5）。
//!
//! 明确**不**负责（设计系统文档 §13 逐字）：
//!
//! - **不持业务规则、不拼 SQL、不调模型**。依赖面只有 `openbot-contracts`（v3 §5.2）。
//! - 不自行做多窗口 / 多线程的可见性过滤 —— 过滤在 Rust 侧按 window label、actor、thread
//!   subscription 和 auth generation 完成，前端做不算数（v3 §13.3）。
//! - 不自报角色、不自报 `principal=admin`（v3 §5.2）。
//! - 不承载 screen 画面流 —— 画面走独立 loopback binary WebSocket，不走 Tauri event（v3 §13.4）。
//! - 不引入远程资产：字体（Inter Variable 4.1）与图标（Lucide 1.33.0 allowlist）随 bundle
//!   打包；`index.html` 零内联脚本，`<html class lang>` 由 Rust 在首帧改写（v3 §13.1）。
//!
//! # 当前状态
//!
//! G6 已开始落真实 Leptos CSR 树。当前闭合的是 token/font/icon/i18n 生成地基与 durable
//! tool approval 的可点击生产 API 竖切。Batch 16 又接 Server preference API/cookie 与
//! Desktop-local custom-protocol framing；完整 route、原语、业务组件、golden 与可发布 Tauri
//! 宿主仍按台账保持未完成，不能由本 crate 存在而推导为 G6 整关通过。

#![deny(missing_docs)]

pub mod api;
pub mod app;
pub mod features;
pub mod preferences;
pub mod primitives;

/// Generated, strongly typed icon allowlist. The source is `design/icons.toml` plus bundled SVGs.
#[allow(missing_docs)]
pub mod icons {
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}

/// Generated Rust token constants. The only value source is `design/tokens.toml`.
#[allow(missing_docs)]
pub mod tokens {
    include!(concat!(env!("OUT_DIR"), "/tokens.rs"));
}

#[allow(missing_docs)]
mod generated_i18n {
    include!(concat!(env!("OUT_DIR"), "/i18n/mod.rs"));
}

pub use generated_i18n::i18n;

#[cfg(test)]
mod design_tests;

/// Mount the single CSR application bundle into `<body>`.
#[cfg(target_arch = "wasm32")]
pub fn mount() {
    leptos::mount::mount_to_body(app::App);
}
