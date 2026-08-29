//! Administrator-only product surfaces.

pub mod audit;
pub mod boundaries;
pub mod home;
pub mod people;
pub mod playground;
pub mod shell;

pub use audit::AdminAuditPage;
pub use boundaries::AdminBoundariesPage;
pub use home::AdminHomePage;
pub use people::AdminPeoplePage;
pub use playground::SandboxPlaygroundPage;
pub use shell::AdminShell;
