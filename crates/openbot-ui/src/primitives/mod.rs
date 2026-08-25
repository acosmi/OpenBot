//! First-party Leptos primitives governed by the GUI first source.

mod badge;
mod button;
mod empty_state;
mod icon;
mod locale_switch;
mod theme_toggle;

pub use badge::{Badge, BadgeTone};
pub use button::{Button, ButtonSize, ButtonVariant};
pub use empty_state::EmptyState;
pub use icon::{IconSize, IconView};
pub use locale_switch::LocaleSwitch;
pub use theme_toggle::{Theme, ThemeToggle};
