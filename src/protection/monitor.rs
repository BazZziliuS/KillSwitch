//! Background loop that observes the network and applies the outcome to the firewall.

use super::exit_watch::ExitWatch;
use super::rules::{self, Reason};
use super::site_watch::SiteWatch;
use crate::config::Config;
use crate::i18n;
use crate::platform::firewall::{self, Plan};
use crate::platform::network::{self, Snapshot};
use crate::platform::notify;
use std::ffi::c_void;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::IpHelper::*;
use windows::Win32::Networking::WinSock::AF_UNSPEC;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

/// Upper bound for changes with no OS notification, such as the system proxy being switched off.
const POLL_INTERVAL: Duration = Duration::from_millis(300);
const NOTIFY_SETTLE: Duration = Duration::from_millis(1500);

#[derive(Clone, Default)]
pub struct Status {
    pub snapshot: Snapshot,
    pub reasons: Vec<Reason>,
    pub enabled: bool,
    pub blocked: bool,
    pub firewall_error: Option<String>,
    pub missing: Vec<PathBuf>,
    pub unresolved_sites: Vec<String>,
    pub ipv6_guarded: bool,
}

/// Holds a notification until the state has settled: switching VPN servers blocks and restores within a moment.
#[derive(Default)]
struct Announcer {
    announced: bool,
    pending: Option<(bool, Instant)>,
}

impl Announcer {
    fn tick(&mut self, blocked: bool, now: Instant) -> Option<bool> {
        if blocked == self.announced {
            self.pending = None;
            return None;
        }
        let since = match self.pending {
            Some((state, since)) if state == blocked => since,
            _ => {
                self.pending = Some((blocked, now));
                now
            }
        };
        if now.duration_since(since) < NOTIFY_SETTLE {
            return None;
        }
        self.announced = blocked;
        self.pending = None;
        Some(blocked)
    }
}

pub struct Shared {
    pub config: Mutex<Config>,
    pub status: Mutex<Status>,
    /// Tag of a newer release, once one has been found.
    pub update: Mutex<Option<String>>,
    pub hwnd: AtomicIsize,
    quit: AtomicBool,
}

impl Shared {
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            config: Mutex::new(config),
            status: Mutex::default(),
            update: Mutex::default(),
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
    let exits = ExitWatch::spawn(wake);
    let site_watch = SiteWatch::spawn(wake);
    let mut announcer = Announcer::default();
    let mut applied: Option<Plan> = None;
    let mut firewall_error = None;
    let mut missing = Vec::new();
    while !shared.quit.load(Ordering::SeqCst) {
        let config = shared.config.lock().unwrap().clone();
        let mut snapshot = network::snapshot();
        snapshot.exit_ip = exits.observe(rules::exit_route(&config, &snapshot));
        let reasons = rules::evaluate(&config, &snapshot);
        let must_block = config.enabled && !reasons.is_empty();
        let watched: &[String] = if config.enabled { &config.sites } else { &[] };
        let sites = site_watch.observe(watched, must_block);
        let ipv6_guarded = config.enabled && !must_block && rules::ipv6_bypasses_tunnel(&config, &snapshot);
        let desired = if must_block {
            Plan { apps: config.apps.clone(), sites: sites.addresses, exempt: sites.exempt, ..Plan::default() }
        } else if ipv6_guarded {
            Plan {
                ipv6_only_apps: config.apps.clone(),
                sites: sites.addresses.into_iter().filter(IpAddr::is_ipv6).collect(),
                exempt: sites.exempt,
                ..Plan::default()
            }
        } else {
            Plan::default()
        };
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
        // A failed block must never be announced as the internet coming back.
        if firewall_error.is_none()
            && let Some(blocked) = announcer.tick(must_block, Instant::now())
            && config.notifications
        {
            let text = i18n::strings(config.language);
            if blocked {
                let body = reasons.iter().map(|reason| text.explain(reason)).collect::<Vec<_>>().join(" ");
                notify::send(text.blocked_title.to_owned(), body);
            } else {
                notify::send(text.restored_title.to_owned(), text.restored_body.to_owned());
            }
        }
        *shared.status.lock().unwrap() = Status {
            snapshot,
            reasons,
            enabled: config.enabled,
            blocked: must_block && firewall_error.is_none(),
            firewall_error: firewall_error.clone(),
            missing: missing.clone(),
            unresolved_sites: sites.unresolved,
            ipv6_guarded: ipv6_guarded && firewall_error.is_none(),
        };
        wait();
    }
    let code = i32::from(firewall::apply(&Plan::default()).is_err());
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::{Announcer, NOTIFY_SETTLE};
    use std::time::{Duration, Instant};

    #[test]
    fn announces_only_after_the_state_settles() {
        let start = Instant::now();
        let mut announcer = Announcer::default();
        assert_eq!(announcer.tick(true, start), None);
        assert_eq!(announcer.tick(true, start + NOTIFY_SETTLE), Some(true));
        assert_eq!(announcer.tick(true, start + NOTIFY_SETTLE * 2), None);
    }

    #[test]
    fn a_short_flap_stays_silent() {
        let start = Instant::now();
        let mut announcer = Announcer::default();
        assert_eq!(announcer.tick(true, start), None);
        assert_eq!(announcer.tick(false, start + Duration::from_millis(900)), None);
        assert_eq!(announcer.tick(false, start + NOTIFY_SETTLE * 4), None);
    }

    #[test]
    fn nothing_is_announced_at_startup_when_online() {
        assert_eq!(Announcer::default().tick(false, Instant::now()), None);
    }
}
