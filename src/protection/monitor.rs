//! Background loop that observes the network and applies the outcome to the firewall.

use super::rules::{self, Reason};
use crate::config::Config;
use crate::platform::firewall;
use crate::platform::network::{self, Snapshot};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::IpHelper::*;
use windows::Win32::Networking::WinSock::AF_UNSPEC;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

/// Upper bound for changes with no OS notification, such as the system proxy being switched off.
const POLL_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Clone, Default)]
pub struct Status {
    pub snapshot: Snapshot,
    pub reasons: Vec<Reason>,
    pub enabled: bool,
    pub blocked: bool,
    pub firewall_error: Option<String>,
    pub missing: Vec<PathBuf>,
}

pub struct Shared {
    pub config: Mutex<Config>,
    pub status: Mutex<Status>,
    pub hwnd: AtomicIsize,
    quit: AtomicBool,
}

impl Shared {
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            config: Mutex::new(config),
            status: Mutex::default(),
            hwnd: AtomicIsize::new(0),
            quit: AtomicBool::new(false),
        })
    }

    pub fn update_config(&self, config: Config) -> std::io::Result<()> {
        let result = config.save();
        *self.config.lock().unwrap() = config;
        wake();
        result
    }

    /// Asks the monitor to lift every block and end the process.
    pub fn quit(&self) {
        self.quit.store(true, Ordering::SeqCst);
        wake();
    }
}

static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

fn wake() {
    *WAKE.0.lock().unwrap() = true;
    WAKE.1.notify_all();
}

fn wait() {
    let guard = WAKE.0.lock().unwrap();
    let (mut pending, _) = WAKE.1.wait_timeout_while(guard, POLL_INTERVAL, |pending| !*pending).unwrap();
    *pending = false;
}

unsafe extern "system" fn on_interface(_: *const c_void, _: *const MIB_IPINTERFACE_ROW, _: MIB_NOTIFICATION_TYPE) {
    wake();
}

unsafe extern "system" fn on_route(_: *const c_void, _: *const MIB_IPFORWARD_ROW2, _: MIB_NOTIFICATION_TYPE) {
    wake();
}

/// React to adapter and route changes immediately instead of at the next poll. The handles live as long as the process.
fn subscribe() {
    let mut interface_handle = HANDLE::default();
    let mut route_handle = HANDLE::default();
    unsafe {
        let _ = NotifyIpInterfaceChange(AF_UNSPEC, Some(on_interface), None, false, &mut interface_handle);
        let _ = NotifyRouteChange2(AF_UNSPEC, Some(on_route), std::ptr::null(), false, &mut route_handle);
    }
}

pub fn run(shared: Arc<Shared>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
    }
    subscribe();
    let mut applied: Option<Vec<PathBuf>> = None;
    let mut firewall_error = None;
    let mut missing = Vec::new();
    while !shared.quit.load(Ordering::SeqCst) {
        let config = shared.config.lock().unwrap().clone();
        let snapshot = network::snapshot();
        let reasons = rules::evaluate(&config, &snapshot);
        let must_block = config.enabled && !reasons.is_empty();
        let desired = if must_block { config.apps.clone() } else { Vec::new() };
        if applied.as_ref() != Some(&desired) {
            match firewall::apply(&desired) {
                Ok(not_found) => {
                    missing = not_found;
                    firewall_error = None;
                    applied = Some(desired);
                }
                Err(error) => firewall_error = Some(error),
            }
        }
        *shared.status.lock().unwrap() = Status {
            snapshot,
            reasons,
            enabled: config.enabled,
            blocked: must_block && firewall_error.is_none(),
            firewall_error: firewall_error.clone(),
            missing: missing.clone(),
        };
        wait();
    }
    let code = i32::from(firewall::apply(&[]).is_err());
    std::process::exit(code);
}
