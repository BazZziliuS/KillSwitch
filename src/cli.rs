//! Process-level startup helpers and the command-line entry points.

use crate::config::Config;
use crate::i18n;
use crate::platform::{egress, firewall, network};
use crate::protection::rules;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{MB_OK, MessageBoxW, SW_SHOWNORMAL};
use windows::core::{HSTRING, PCWSTR, w};

pub fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin() }.as_bool()
}

pub fn relaunch_elevated(args: &[String]) {
    let Ok(exe) = std::env::current_exe() else { return };
    let parameters = HSTRING::from(args.join(" "));
    unsafe {
        ShellExecuteW(None, w!("runas"), &HSTRING::from(exe.as_os_str()), &parameters, PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn already_running() -> bool {
    // The mutex stays open for the life of the process.
    let mutex = unsafe { CreateMutexW(None, false, w!("Local\\KillSwitch.SingleInstance")) };
    mutex.is_ok() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS
}

/// Recovery path: lifts blocks left behind when the app is not running.
pub fn unblock() {
    let strings = i18n::strings(Config::load().language);
    let text = match firewall::apply(&firewall::Plan::default()) {
        Ok(_) => HSTRING::from(strings.unblock_done),
        Err(error) => HSTRING::from((strings.unblock_failed)(&error)),
    };
    unsafe {
        MessageBoxW(None, &text, w!("KillSwitch"), MB_OK);
    }
}

pub fn print_status() {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
    }
    let config = Config::load();
    let mut snapshot = network::snapshot();
    if let Some(route) = rules::exit_route(&config, &snapshot) {
        match egress::public_ip(route.proxy()) {
            Ok(address) => snapshot.exit_ip = Some(address),
            Err(error) => println!("server check failed: {error}"),
        }
    }
    println!("exit address: {:?}", snapshot.exit_ip);
    println!("networks: {:?}", snapshot.networks);
    println!("route interface: {:?}", snapshot.route_interface);
    for adapter in snapshot.adapters.iter().filter(|adapter| adapter.up) {
        println!("adapter #{} {:?} hardware={}", adapter.index, adapter.name, adapter.hardware);
    }
    println!("proxy: {:?}", snapshot.proxy);
    println!("tun active: {}", rules::tun_active(&config, &snapshot));
    println!("errors: {:?}", snapshot.errors);
    println!("block reasons: {:?}", rules::evaluate(&config, &snapshot));
}
