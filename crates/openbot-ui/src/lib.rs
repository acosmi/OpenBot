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
//!   禁止字面颜色 / 任意值 / `dark:` 变体；class 必须是源码里的**完整字面量**，禁止
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
//! G2 只先落了不需要组件树的纯 Rust tool transcript projection；
//! **Cargo.toml 里仍刻意没有 leptos 依赖**，不冒充 G6 GUI 已开工。
//! 钉版表以注释形式备查在 `crates/openbot-ui/Cargo.toml`（真源 = 设计系统文档 §12.4）。
//! GUI 是 G6 的产物；Phase 0 只产出 `parity/ui.yaml`、`fixtures/ui/**` 与 `tools/pins.toml`
//! （设计系统文档 §11，已并入 v3 §19.3）。

#![deny(missing_docs)]

pub mod features;
