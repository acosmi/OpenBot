//! Closed browser-engine protocol and authority-owned residency decisions. No free CDP method or
//! host path crosses this boundary.

pub mod eviction;
pub mod protocol;

pub use protocol::{
    BrowserInput, BrowserInputKind, BrowserOperation, BrowserOperationKind, ElementTarget,
    InputProtocolError, KeyInput, ModifierMask, MouseButton, NavigateOperation, PointerInput,
    ProfileOperation, ScreencastOperation, ScrollOperation, SecretInsert, TextInput,
};
