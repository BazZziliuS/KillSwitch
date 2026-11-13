//! Pure decision logic: from configuration and observed network state to block reasons.

use super::exit_watch::Route;
use crate::config::Config;
use crate::platform::network::Snapshot;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Reason {
    NoNetwork,
    UntrustedNetwork(String),
    VpnOff,
    DetectionFailed(String),
    ForeignServer(IpAddr),
    ServerUnknown,
}

/// Windows names a new adapter "throne-tun 2" while a stale instance still holds the plain name.
pub fn adapter_matches(alias: &str, configured: &str) -> bool {
    let alias = alias.to_lowercase();
    let configured = configured.to_lowercase();
    match alias.strip_prefix(&configured) {
        Some("") => true,
        Some(rest) => rest.strip_prefix(' ').is_some_and(|number| number.parse::<u16>().is_ok()),
        None => false,
    }
}

fn is_tunnel(config: &Config, snapshot: &Snapshot, interface: Option<u32>) -> bool {
    snapshot.adapters.iter().any(|adapter| {
        adapter.up
            && interface == Some(adapter.index)
            && config.tun_adapters.iter().any(|name| adapter_matches(&adapter.name, name))
    })
}

pub fn tun_active(config: &Config, snapshot: &Snapshot) -> bool {
    is_tunnel(config, snapshot, snapshot.route_interface)
}

/// The tunnel carries IPv4, but IPv6 would leave through another adapter.
pub fn ipv6_bypasses_tunnel(config: &Config, snapshot: &Snapshot) -> bool {
    config.tun_enabled
        && tun_active(config, snapshot)
        && snapshot.route_interface_v6.is_some()
        && !is_tunnel(config, snapshot, snapshot.route_interface_v6)
}

/// How to reach the internet through the VPN to learn its address; `None` when there is nothing to check. Without
/// the VPN nothing is asked, so the services never see the real address.
pub fn exit_route(config: &Config, snapshot: &Snapshot) -> Option<Route> {
    if config.server_check || config.ip_info { vpn_route(config, snapshot) } else { None }
}

/// How to reach the internet through the VPN right now; `None` when the VPN is off.
pub fn vpn_route(config: &Config, snapshot: &Snapshot) -> Option<Route> {
    if config.tun_enabled && tun_active(config, snapshot) {
        snapshot.route_interface.map(Route::Tunnel)
    } else if config.proxy_enabled && snapshot.proxy.active() {
        snapshot.proxy.server.clone().map(Route::Proxy)
    } else {
        None
    }
}

/// Every reason the protected applications must be offline right now; empty means they may connect.
pub fn evaluate(config: &Config, snapshot: &Snapshot) -> Vec<Reason> {
    let mut reasons: Vec<Reason> = snapshot.errors.iter().cloned().map(Reason::DetectionFailed).collect();
    if snapshot.networks.is_empty() && snapshot.errors.is_empty() {
        reasons.push(Reason::NoNetwork);
    }
    for network in &snapshot.networks {
        if !config.trusted.iter().any(|trusted| trusted.id == network.id) {
            reasons.push(Reason::UntrustedNetwork(network.name.clone()));
        }
    }
    let vpn_on = (config.tun_enabled && tun_active(config, snapshot)) || (config.proxy_enabled && snapshot.proxy.active());
    if !vpn_on {
        reasons.push(Reason::VpnOff);
    } else if config.server_check {
        match snapshot.exit_ip {
            Some(address) if config.allowed_servers.contains(&address) => {}
            Some(address) => reasons.push(Reason::ForeignServer(address)),
            None => reasons.push(Reason::ServerUnknown),
        }
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrustedNetwork;
    use crate::platform::network::{Adapter, Network, Proxy};
    use windows::core::GUID;

    const TUN_INDEX: u32 = 8;
    const TAILSCALE_INDEX: u32 = 7;

    fn adapter(name: &str, index: u32) -> Adapter {
        Adapter { name: name.into(), description: String::new(), index, guid: GUID::zeroed(), up: true, hardware: false }
    }

    fn config() -> Config {
        Config {
            enabled: true,
            trusted: vec![TrustedNetwork { id: "home".into(), name: "Home".into() }],
            ..Config::default()
        }
    }

    fn safe_snapshot() -> Snapshot {
        Snapshot {
            adapters: vec![adapter("throne-tun", TUN_INDEX), adapter("Tailscale", TAILSCALE_INDEX)],
            networks: vec![Network { id: "home".into(), name: "Home".into() }],
            route_interface: Some(TUN_INDEX),
            ..Snapshot::default()
        }
    }

    #[test]
    fn trusted_network_with_tun_is_allowed() {
        assert!(evaluate(&config(), &safe_snapshot()).is_empty());
    }

    #[test]
    fn ipv6_outside_the_tunnel_is_detected() {
        const PHYSICAL_INDEX: u32 = 3;
        let without_route = safe_snapshot();
        assert!(!ipv6_bypasses_tunnel(&config(), &without_route));
        let through_tunnel = Snapshot { route_interface_v6: Some(TUN_INDEX), ..safe_snapshot() };
        assert!(!ipv6_bypasses_tunnel(&config(), &through_tunnel));
        let around_tunnel = Snapshot { route_interface_v6: Some(PHYSICAL_INDEX), ..safe_snapshot() };
        assert!(ipv6_bypasses_tunnel(&config(), &around_tunnel));
    }

    #[test]
    fn ipv6_is_not_guarded_without_a_tunnel() {
        let proxy_only = Snapshot { route_interface: Some(3), route_interface_v6: Some(3), ..safe_snapshot() };
        assert!(!ipv6_bypasses_tunnel(&config(), &proxy_only));
    }

    fn checked_config() -> Config {
        Config { server_check: true, allowed_servers: vec!["203.0.113.1".parse().unwrap()], ..config() }
    }

    #[test]
    fn allowed_server_stays_online() {
        let snapshot = Snapshot { exit_ip: Some("203.0.113.1".parse().unwrap()), ..safe_snapshot() };
        assert!(evaluate(&checked_config(), &snapshot).is_empty());
    }

    #[test]
    fn other_server_blocks() {
        let address: IpAddr = "198.51.100.9".parse().unwrap();
        let snapshot = Snapshot { exit_ip: Some(address), ..safe_snapshot() };
        assert_eq!(evaluate(&checked_config(), &snapshot), [Reason::ForeignServer(address)]);
    }

    #[test]
    fn unknown_server_blocks() {
        assert_eq!(evaluate(&checked_config(), &safe_snapshot()), [Reason::ServerUnknown]);
    }

    #[test]
    fn server_is_ignored_unless_the_check_is_on() {
        let snapshot = Snapshot { exit_ip: Some("198.51.100.9".parse().unwrap()), ..safe_snapshot() };
        assert!(evaluate(&config(), &snapshot).is_empty());
        let silent = Config { ip_info: false, ..config() };
        assert_eq!(exit_route(&silent, &snapshot), None);
    }

    #[test]
    fn the_address_is_looked_up_only_through_the_vpn() {
        let snapshot = safe_snapshot();
        assert_eq!(exit_route(&config(), &snapshot), vpn_route(&config(), &snapshot));
        assert!(exit_route(&config(), &snapshot).is_some());
        let mut no_vpn = safe_snapshot();
        no_vpn.adapters.remove(0);
        assert_eq!(exit_route(&config(), &no_vpn), None);
    }

    #[test]
    fn vpn_off_is_reported_instead_of_the_server() {
        let mut snapshot = safe_snapshot();
        snapshot.adapters.remove(0);
        assert_eq!(evaluate(&checked_config(), &snapshot), [Reason::VpnOff]);
        assert_eq!(exit_route(&checked_config(), &snapshot), None);
    }

    #[test]
    fn check_follows_the_tunnel_before_the_proxy() {
        let mut snapshot = safe_snapshot();
        snapshot.proxy = Proxy { server: Some("127.0.0.1:2080".into()), local: true, listening: true };
        assert_eq!(exit_route(&checked_config(), &snapshot), Some(Route::Tunnel(TUN_INDEX)));
        snapshot.adapters.clear();
        assert_eq!(exit_route(&checked_config(), &snapshot), Some(Route::Proxy("127.0.0.1:2080".into())));
    }

    #[test]
    fn untrusted_network_blocks_even_with_vpn_on() {
        let mut snapshot = safe_snapshot();
        snapshot.networks.push(Network { id: "cafe".into(), name: "Cafe".into() });
        assert_eq!(evaluate(&config(), &snapshot), [Reason::UntrustedNetwork("Cafe".into())]);
    }

    #[test]
    fn tun_adapter_removed_blocks() {
        let mut snapshot = safe_snapshot();
        snapshot.adapters.remove(0);
        assert_eq!(evaluate(&config(), &snapshot), [Reason::VpnOff]);
    }

    #[test]
    fn tun_present_without_route_blocks() {
        let mut snapshot = safe_snapshot();
        snapshot.route_interface = Some(15);
        assert_eq!(evaluate(&config(), &snapshot), [Reason::VpnOff]);
    }

    #[test]
    fn other_tunnel_does_not_count_as_vpn() {
        let mut snapshot = safe_snapshot();
        snapshot.route_interface = Some(TAILSCALE_INDEX);
        assert_eq!(evaluate(&config(), &snapshot), [Reason::VpnOff]);
    }

    #[test]
    fn local_proxy_counts_only_while_allowed_and_listening() {
        let mut snapshot = safe_snapshot();
        snapshot.adapters.clear();
        snapshot.proxy = Proxy { server: Some("127.0.0.1:2080".into()), local: true, listening: true };
        assert!(evaluate(&config(), &snapshot).is_empty());

        let tun_only = Config { proxy_enabled: false, ..config() };
        assert_eq!(evaluate(&tun_only, &snapshot), [Reason::VpnOff]);

        snapshot.proxy.listening = false;
        assert_eq!(evaluate(&config(), &snapshot), [Reason::VpnOff]);
    }

    #[test]
    fn no_network_and_detection_errors_block() {
        let mut snapshot = safe_snapshot();
        snapshot.networks.clear();
        assert_eq!(evaluate(&config(), &snapshot), [Reason::NoNetwork]);

        snapshot.errors.push("boom".into());
        assert_eq!(evaluate(&config(), &snapshot), [Reason::DetectionFailed("boom".into())]);
    }

    #[test]
    fn adapter_alias_follows_windows_numbering() {
        assert!(adapter_matches("throne-tun", "throne-tun"));
        assert!(adapter_matches("Throne-TUN 2", "throne-tun"));
        assert!(!adapter_matches("throne-tunnel", "throne-tun"));
        assert!(!adapter_matches("throne-tun x", "throne-tun"));
    }
}
