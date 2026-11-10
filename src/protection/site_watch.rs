//! Turns site names into addresses on its own thread, because the filtering layer sees only IP addresses.
//!
//! Answers accumulate and are saved to disk: when the VPN drops DNS may fail or return different addresses, and a block
//! has to work from what was learned earlier.

use crate::config::{self, HostFilter};
use crate::platform::{apps, dns_watch};
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
/// A block is when new DNS answers matter most: the resolver may have just changed.
const BLOCKED_INTERVAL: Duration = Duration::from_secs(15);
const RETRY_INTERVAL: Duration = Duration::from_secs(10);
const MAX_PER_SITE: usize = 64;
/// A pattern covers many domains, each with its own addresses.
const MAX_PER_PATTERN: usize = 512;
const CACHE_FILE: &str = "sites.json";

type Known = BTreeMap<String, BTreeSet<IpAddr>>;

#[derive(Default)]
struct State {
    sites: Vec<String>,
    filters: Vec<String>,
    blocking: bool,
    generation: u64,
    known: Known,
    exempt: Vec<PathBuf>,
    watch_failed: bool,
}

#[derive(Default)]
pub struct SiteView {
    pub addresses: Vec<IpAddr>,
    pub exempt: Vec<PathBuf>,
    pub unresolved: Vec<String>,
    /// How many addresses each pattern has collected so far.
    pub pattern_addresses: BTreeMap<String, usize>,
    pub pattern_watch_failed: bool,
}

type Cell = (Mutex<State>, Condvar);

pub struct SiteWatch(Arc<Cell>);

impl SiteWatch {
    pub fn spawn(on_change: fn()) -> Self {
        let state = State { known: load(), ..State::default() };
        let cell: Arc<Cell> = Arc::new((Mutex::new(state), Condvar::new()));
        let worker = cell.clone();
        std::thread::spawn(move || run(&worker, on_change));
        let pattern_worker = cell.clone();
        std::thread::spawn(move || run_pattern_watch(pattern_worker, on_change));
        Self(cell)
    }

    pub fn observe(&self, sites: &[String], filters: &[String], blocking: bool) -> SiteView {
        let mut state = self.0.0.lock().unwrap();
        if state.sites != sites || state.filters != filters || state.blocking != blocking {
            state.sites = sites.to_vec();
            state.filters = filters.to_vec();
            state.blocking = blocking;
            state.generation += 1;
            self.0.1.notify_all();
        }
        let addresses: BTreeSet<IpAddr> =
            sites.iter().filter_map(|site| state.known.get(site)).flatten().copied().collect();
        SiteView {
            addresses: addresses.into_iter().collect(),
            exempt: state.exempt.clone(),
            unresolved: sites.iter().filter(|site| !config::is_pattern(site) && !state.known.contains_key(*site)).cloned().collect(),
            pattern_addresses: sites
                .iter()
                .filter(|site| config::is_pattern(site))
                .map(|site| (site.clone(), state.known.get(site).map_or(0, BTreeSet::len)))
                .collect(),
            pattern_watch_failed: state.watch_failed,
        }
    }
}

fn run((state, changed): &Cell, on_change: fn()) {
    loop {
        let guard = changed.wait_while(state.lock().unwrap(), |state| state.sites.is_empty()).unwrap();
        let (sites, generation, blocking) = (guard.sites.clone(), guard.generation, guard.blocking);
        drop(guard);

        let answers: Vec<(String, Vec<IpAddr>)> =
            sites.into_iter().filter(|site| !config::is_pattern(site)).map(|site| { let found = lookup(&site); (site, found) }).collect();
        let complete = answers.iter().all(|(_, found)| !found.is_empty());
        let exempt = vpn_clients();

        let mut guard = state.lock().unwrap();
        let mut changed_known = false;
        for (site, found) in answers {
            changed_known |= merge(guard.known.entry(site).or_default(), found, MAX_PER_SITE);
        }
        let current = guard.sites.clone();
        guard.known.retain(|site, addresses| current.contains(site) && !addresses.is_empty());
        guard.exempt = exempt;
        if changed_known {
            save(&guard.known);
        }
        drop(guard);
        on_change();

        let delay = match (complete, blocking) {
            (false, _) => RETRY_INTERVAL,
            (true, true) => BLOCKED_INTERVAL,
            (true, false) => REFRESH_INTERVAL,
        };
        let guard = state.lock().unwrap();
        drop(changed.wait_timeout_while(guard, delay, |state| state.generation == generation).unwrap());
    }
}

/// Starts watching DNS lookups once the first pattern exists, so that nothing is traced for people who don't use them.
fn run_pattern_watch(cell: Arc<Cell>, on_change: fn()) {
    drop(cell.1.wait_while(cell.0.lock().unwrap(), |state| !state.sites.iter().any(|site| config::is_pattern(site))).unwrap());
    let learner = cell.clone();
    let filter = Mutex::new(HostFilter::default());
    let result = dns_watch::run(Box::new(move |name, addresses| learn(&learner, &filter, on_change, name, addresses)));
    if result.is_err() {
        cell.0.lock().unwrap().watch_failed = true;
        on_change();
    }
}

fn learn(cell: &Cell, filter: &Mutex<HostFilter>, on_change: fn(), name: &str, addresses: Vec<IpAddr>) {
    let addresses: Vec<IpAddr> = addresses.into_iter().filter(|address| is_public(*address)).collect();
    if addresses.is_empty() {
        return;
    }
    let mut state = cell.0.lock().unwrap();
    let mut filter = filter.lock().unwrap();
    filter.update(&state.filters);
    if filter.excludes(name) {
        return;
    }
    let matching: Vec<String> =
        state.sites.iter().filter(|site| config::is_pattern(site) && config::pattern_matches(site, name)).cloned().collect();
    let mut learned = false;
    for pattern in matching {
        learned |= merge(state.known.entry(pattern).or_default(), addresses.clone(), MAX_PER_PATTERN);
    }
    if learned {
        save(&state.known);
        drop(state);
        on_change();
    }
}

/// Returns whether anything new was learned. A site on a rotating CDN restarts from its latest answer at the cap.
fn merge(known: &mut BTreeSet<IpAddr>, found: Vec<IpAddr>, cap: usize) -> bool {
    let before = known.len();
    if found.iter().collect::<BTreeSet<_>>().into_iter().filter(|address| !known.contains(*address)).count() + known.len() > cap {
        known.clear();
        known.extend(found.into_iter().take(cap));
        return true;
    }
    known.extend(found);
    known.len() != before
}

fn lookup(site: &str) -> Vec<IpAddr> {
    let hosts = [site.to_owned(), format!("www.{site}")];
    let mut found: Vec<IpAddr> = hosts
        .iter()
        .filter_map(|host| (host.as_str(), 443).to_socket_addrs().ok())
        .flatten()
        .map(|socket| socket.ip())
        .filter(|address| is_public(*address))
        .collect();
    found.sort();
    found.dedup();
    found
}

/// Excludes anything that could cut the machine off from itself or its LAN, and the 198.18/15 range VPN clients hand
/// out as placeholder DNS answers.
fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (18..20).contains(&b)))
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || first & 0xFE00 == 0xFC00 || first & 0xFFC0 == 0xFE80)
            }
        },
    }
}

fn vpn_clients() -> Vec<PathBuf> {
    let mut clients: Vec<PathBuf> =
        apps::running().into_iter().map(|app| app.path).filter(|path| config::is_vpn_client(path)).collect();
    clients.sort();
    clients
}

fn load() -> Known {
    std::fs::read(config::data_path(CACHE_FILE)).ok().and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default()
}

fn save(known: &Known) {
    if let Ok(data) = serde_json::to_vec_pretty(known) {
        let _ = std::fs::write(config::data_path(CACHE_FILE), data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public(address: &str) -> bool {
        is_public(address.parse().unwrap())
    }

    #[test]
    fn only_public_addresses_can_be_blocked() {
        assert!(public("93.184.216.34"));
        assert!(public("2606:2800:220:1::1"));
        for local in ["127.0.0.1", "192.168.1.1", "10.0.0.1", "100.100.1.1", "198.18.0.5", "0.0.0.0", "::1", "fe80::1", "fd00::1", "::ffff:192.168.1.1"] {
            assert!(!public(local), "{local}");
        }
    }

    #[test]
    fn merge_accumulates_and_reports_news() {
        let (a, b): (IpAddr, IpAddr) = ("1.1.1.1".parse().unwrap(), "8.8.8.8".parse().unwrap());
        let mut known = BTreeSet::new();
        assert!(merge(&mut known, vec![a], MAX_PER_SITE));
        assert!(!merge(&mut known, vec![a], MAX_PER_SITE));
        assert!(merge(&mut known, vec![b], MAX_PER_SITE));
        assert_eq!(known.len(), 2);
    }

    #[test]
    fn merge_restarts_at_the_cap() {
        let mut known: BTreeSet<IpAddr> = (0..MAX_PER_SITE as u32).map(|n| IpAddr::V4((0x0100_0000 + n).into())).collect();
        let fresh: IpAddr = "9.9.9.9".parse().unwrap();
        assert!(merge(&mut known, vec![fresh], MAX_PER_SITE));
        assert_eq!(known.into_iter().collect::<Vec<_>>(), [fresh]);
    }
}
