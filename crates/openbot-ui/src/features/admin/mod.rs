//! Administrator-only product surfaces.

pub mod audit;
pub mod home;
pub mod playground;
pub mod shell;

pub use audit::AdminAuditPage;
pub use home::AdminHomePage;
pub use playground::SandboxPlaygroundPage;
pub use shell::AdminShell;
