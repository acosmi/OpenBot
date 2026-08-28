//! User settings business projections.

pub mod components_gallery;
pub mod computer_placeholder_art;
pub mod connected_accounts;
pub mod preferences;
pub mod shell;

pub use components_gallery::{ComponentGalleryDetailPage, ComponentsGalleryPage};
pub use computer_placeholder_art::ComputerPlaceholderArt;
pub use connected_accounts::{ConnectedAccountDetailPage, ConnectedAccountsPage};
pub use preferences::SettingsPage;
pub use shell::SettingsShell;
