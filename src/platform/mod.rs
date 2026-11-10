//! Thin wrappers over Windows APIs. Nothing here knows about the UI or the protection rules.

pub mod apps;
pub mod autostart;
pub mod egress;
pub mod firewall;
pub mod network;
pub mod notify;
pub mod site_scan;
