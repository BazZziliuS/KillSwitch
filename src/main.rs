#![allow(non_snake_case)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod config;
mod i18n;
mod platform;
mod protection;
mod ui;
mod updates;

use config::Config;
use platform::{autostart, firewall};
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

    let config = Config::load();
    if config.autostart {
        // Keeps the scheduled task pointing here if the program has been moved.
        std::thread::spawn(|| autostart::set(true));
    }
    let shared = Shared::new(config);
    let (monitor_shared, overlay_shared, updates_shared) = (shared.clone(), shared.clone(), shared.clone());
    std::thread::spawn(move || monitor::run(monitor_shared));
    std::thread::spawn(move || ui::overlay::run(overlay_shared));
    std::thread::spawn(move || updates::run(updates_shared));
    ui::run(shared, has(autostart::ARGUMENT))
}
