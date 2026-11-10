//! Looks for a newer release on GitHub. Nothing is downloaded or installed: the user is only pointed at the page.

use crate::i18n;
use crate::platform::{egress, notify};
use crate::protection::monitor::Shared;
use serde::Deserialize;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::w;

const RELEASES_PAGE: &str = "https://github.com/BazZziliuS/KillSwitch/releases/latest";
/// With autostart the network is often not up yet in the first seconds after logon.
const FIRST_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 60);
const DISABLED_INTERVAL: Duration = Duration::from_secs(10 * 60);
const MAX_RESPONSE: usize = 512 * 1024;

#[derive(Deserialize)]
struct Latest {
    tag_name: String,
}

fn version(tag: &str) -> Option<(u32, u32, u32)> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.').map(str::parse);
    let result = (parts.next()?.ok()?, parts.next()?.ok()?, parts.next()?.ok()?);
    parts.next().is_none().then_some(result)
}

fn is_newer(tag: &str, current: &str) -> bool {
    matches!((version(tag), version(current)), (Some(latest), Some(current)) if latest > current)
}

fn latest_tag(proxy: Option<&str>) -> Result<String, String> {
    let body = egress::fetch(w!("api.github.com"), w!("/repos/BazZziliuS/KillSwitch/releases/latest"), proxy, MAX_RESPONSE)?;
    serde_json::from_slice::<Latest>(&body).map(|latest| latest.tag_name).map_err(|error| error.to_string())
}

/// Opened through Explorer so that the browser doesn't inherit this process's administrator rights.
pub fn open_releases_page() {
    let _ = Command::new("explorer").arg(RELEASES_PAGE).spawn();
}

pub fn run(shared: Arc<Shared>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
    }
    std::thread::sleep(FIRST_DELAY);
    let mut announced: Option<String> = None;
    loop {
        let config = shared.config.lock().unwrap().clone();
        if !config.update_check {
            *shared.update.lock().unwrap() = None;
            std::thread::sleep(DISABLED_INTERVAL);
            continue;
        }
        let proxy = {
            let status = shared.status.lock().unwrap();
            status.snapshot.proxy.active().then(|| status.snapshot.proxy.server.clone()).flatten()
        };
        let delay = match latest_tag(proxy.as_deref()) {
            Ok(tag) => {
                let newer = is_newer(&tag, env!("CARGO_PKG_VERSION"));
                if newer && config.notifications && announced.as_ref() != Some(&tag) {
                    let text = i18n::strings(config.language);
                    notify::send((text.update_available)(&tag), text.update_body.to_owned());
                    announced = Some(tag.clone());
                }
                *shared.update.lock().unwrap() = newer.then_some(tag);
                CHECK_INTERVAL
            }
            Err(_) => RETRY_INTERVAL,
        };
        std::thread::sleep(delay);
    }
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("v1.2.0", "1.1.0"));
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(!is_newer("v1.1.0", "1.1.0"));
        assert!(!is_newer("v1.0.9", "1.1.0"));
    }

    #[test]
    fn ignores_tags_it_cannot_read() {
        assert!(!is_newer("nightly", "1.1.0"));
        assert!(!is_newer("v2.0.0-rc1", "1.1.0"));
        assert!(!is_newer("v2.0", "1.1.0"));
    }
}
