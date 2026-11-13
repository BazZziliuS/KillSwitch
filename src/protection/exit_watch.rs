//! Keeps track of the VPN's public address and its country on its own thread, because each check is a slow
//! network request.

use crate::platform::egress::{self, Country};
use std::net::IpAddr;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const CHECK_INTERVAL: Duration = Duration::from_secs(15);
const RETRY_INTERVAL: Duration = Duration::from_secs(3);
/// A few failed checks are tolerated; after that the server counts as unknown.
const MAX_AGE: Duration = Duration::from_secs(45);
/// The geolocation service allows 1000 lookups a day, so a failed one waits before it is tried again.
const COUNTRY_RETRY: Duration = Duration::from_secs(5 * 60);

/// The path traffic takes to the VPN. When it changes, whatever was known about the server no longer applies.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Tunnel(u32),
    Proxy(String),
}

impl Route {
    pub fn proxy(&self) -> Option<&str> {
        match self {
            Route::Tunnel(_) => None,
            Route::Proxy(server) => Some(server),
        }
    }
}

#[derive(Default)]
struct State {
    route: Option<Route>,
    generation: u64,
    found: Option<(IpAddr, Instant)>,
    /// The country of the last address looked up. Looked up again only when the address changes.
    country: Option<(IpAddr, Country)>,
    country_failed: Option<(IpAddr, Instant)>,
}

type Cell = (Mutex<State>, Condvar);

pub struct ExitWatch(Arc<Cell>);

impl ExitWatch {
    pub fn spawn(on_change: fn()) -> Self {
        let cell: Arc<Cell> = Arc::default();
        let worker = cell.clone();
        std::thread::spawn(move || run(&worker, on_change));
        Self(cell)
    }

    /// Reports the current route (`None` stops checking) and returns the address, if one is known and recent.
    pub fn observe(&self, route: Option<Route>) -> Option<IpAddr> {
        let mut state = self.0.0.lock().unwrap();
        if state.route != route {
            state.route = route;
            state.generation += 1;
            state.found = None;
            self.0.1.notify_all();
        }
        state.found.filter(|(_, checked)| checked.elapsed() < MAX_AGE).map(|(address, _)| address)
    }

    pub fn country(&self, address: IpAddr) -> Option<Country> {
        let state = self.0.0.lock().unwrap();
        state.country.as_ref().filter(|(known, _)| *known == address).map(|(_, country)| country.clone())
    }
}

fn needs_country(state: &State, address: IpAddr) -> bool {
    let known = state.country.as_ref().is_some_and(|(known, _)| *known == address);
    let failed_recently =
        state.country_failed.is_some_and(|(failed, when)| failed == address && when.elapsed() < COUNTRY_RETRY);
    !known && !failed_recently
}

fn run((state, changed): &Cell, on_change: fn()) {
    loop {
        let guard = changed.wait_while(state.lock().unwrap(), |state| state.route.is_none()).unwrap();
        let generation = guard.generation;
        let route = guard.route.clone().expect("waited for a route");
        drop(guard);

        let result = egress::public_ip(route.proxy());
        let delay = if result.is_ok() { CHECK_INTERVAL } else { RETRY_INTERVAL };
        let country = match result {
            Ok(address) if needs_country(&state.lock().unwrap(), address) => {
                Some((address, egress::country(address, route.proxy())))
            }
            _ => None,
        };

        let mut guard = state.lock().unwrap();
        // An answer that arrived after the route changed describes the previous connection.
        if guard.generation == generation
            && let Ok(address) = result
        {
            guard.found = Some((address, Instant::now()));
        }
        match country {
            Some((address, Ok(country))) if guard.generation == generation => guard.country = Some((address, country)),
            Some((address, Err(_))) => guard.country_failed = Some((address, Instant::now())),
            _ => {}
        }
        drop(guard);
        on_change();

        let guard = state.lock().unwrap();
        drop(changed.wait_timeout_while(guard, delay, |state| state.generation == generation).unwrap());
    }
}
