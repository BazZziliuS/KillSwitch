#![allow(non_snake_case)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod config;
mod i18n;
mod platform;
mod protection;
mod ui;

use config::Config;
use platform::firewall;
use protection::monitor::{self, Shared};
use std::sync::atomic::Ordering;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|arg| arg == flag);

    if has("--status") {
        cli::print_status();
        return Ok(());
    }
    // --dry-run runs the whole app unprivileged for development, without ever writing a filter.
    firewall::DRY_RUN.store(has("--dry-run"), Ordering::SeqCst);
    if !has("--dry-run") && !cli::is_elevated() {
        cli::relaunch_elevated(&args);
        return Ok(());
    }
    if has("--unblock") {
        cli::unblock();
        return Ok(());
    }
    if cli::already_running() {
        return Ok(());
    }

    let shared = Shared::new(Config::load());
    let monitor_shared = shared.clone();
    std::thread::spawn(move || monitor::run(monitor_shared));
    ui::run(shared)
}
