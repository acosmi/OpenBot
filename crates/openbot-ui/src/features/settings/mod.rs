//! User settings business projections.

pub mod computer_placeholder_art;
pub mod connected_accounts;
pub mod preferences;
pub mod shell;

pub use computer_placeholder_art::ComputerPlaceholderArt;
pub use connected_accounts::{ConnectedAccountDetailPage, ConnectedAccountsPage};
pub use preferences::SettingsPage;
pub use shell::SettingsShell;
