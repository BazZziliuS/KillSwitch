//! Read-only view of the machine's network state. Nothing here needs administrator rights.

use std::net::IpAddr;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
use windows::Win32::NetworkManagement::IpHelper::*;
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::NetworkListManager::*;
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR_IN6};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{GUID, w};

const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const FLAG_HARDWARE: u8 = 1 << 0;
const FLAG_FILTER: u8 = 1 << 1;
/// Any public address works: the question is only which interface the OS would route it through.
const ROUTE_PROBE: [u8; 4] = [1, 1, 1, 1];
/// 2606:4700:4700::1111
const ROUTE_PROBE_V6: [u8; 16] = [0x26, 0x06, 0x47, 0, 0x47, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x11, 0x11];

#[derive(Clone, Debug)]
pub struct Adapter {
    pub name: String,
    pub description: String,
    pub index: u32,
    pub guid: GUID,
    pub up: bool,
    pub hardware: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Proxy {
    pub server: Option<String>,
    pub local: bool,
    pub listening: bool,
}

impl Proxy {
    pub fn active(&self) -> bool {
        self.local && self.listening
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub adapters: Vec<Adapter>,
    /// Networks on physical adapters only, so VPN and Tailscale "networks" never count as the current location.
    pub networks: Vec<Network>,
    pub route_interface: Option<u32>,
    /// `None` when this machine has no route to the IPv6 internet.
    pub route_interface_v6: Option<u32>,
    pub proxy: Proxy,
    /// Public address seen through the VPN; `None` until a recent check has succeeded.
    pub exit_ip: Option<IpAddr>,
    pub errors: Vec<String>,
}

impl Snapshot {
    pub fn virtual_adapters(&self) -> impl Iterator<Item = &Adapter> {
        self.adapters
            .iter()
            .filter(|adapter| adapter.up && !adapter.hardware && !adapter.description.contains("Miniport"))
    }
}

pub fn snapshot() -> Snapshot {
    let mut errors = Vec::new();
    let adapters = interfaces().unwrap_or_else(|error| {
        errors.push(error);
        Vec::new()
    });
    let networks = physical_networks(&adapters).unwrap_or_else(|error| {
        errors.push(format!("network list: {error}"));
        Vec::new()
    });
    let mut index = 0u32;
    let route_interface =
        (unsafe { GetBestInterface(u32::from_ne_bytes(ROUTE_PROBE), &mut index) } == ERROR_SUCCESS.0).then_some(index);
    Snapshot { adapters, networks, route_interface, route_interface_v6: ipv6_route(), proxy: system_proxy(), exit_ip: None, errors }
}

fn ipv6_route() -> Option<u32> {
    let mut destination = SOCKADDR_IN6 { sin6_family: AF_INET6, ..Default::default() };
    destination.sin6_addr.u.Byte = ROUTE_PROBE_V6;
    let mut index = 0u32;
    (unsafe { GetBestInterfaceEx((&raw const destination).cast(), &mut index) } == ERROR_SUCCESS.0).then_some(index)
}

fn text(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

fn interfaces() -> Result<Vec<Adapter>, String> {
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        let status = GetIfTable2(&mut table);
        if status != ERROR_SUCCESS {
            return Err(format!("adapter list: {}", status.0));
        }
        let count = (*table).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), count);
        let adapters = rows
            .iter()
            .filter(|row| {
                row.InterfaceAndOperStatusFlags._bitfield & FLAG_FILTER == 0 && row.Type != IF_TYPE_SOFTWARE_LOOPBACK
            })
            .map(|row| Adapter {
                name: text(&row.Alias),
                description: text(&row.Description),
                index: row.InterfaceIndex,
                guid: row.InterfaceGuid,
                up: row.OperStatus == IfOperStatusUp,
                hardware: row.InterfaceAndOperStatusFlags._bitfield & FLAG_HARDWARE != 0,
            })
            .collect();
        FreeMibTable(table.cast());
        Ok(adapters)
    }
}

fn physical_networks(adapters: &[Adapter]) -> windows::core::Result<Vec<Network>> {
    unsafe {
        let manager: INetworkListManager = CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL)?;
        let connections = manager.GetNetworkConnections()?;
        let mut networks: Vec<Network> = Vec::new();
        loop {
            let mut slot = [None];
            let mut fetched = 0u32;
            connections.Next(&mut slot, Some(&mut fetched))?;
            let [Some(connection)] = slot else { break };
            if fetched == 0 {
                break;
            }
            let adapter_id = connection.GetAdapterId()?;
            if !adapters.iter().any(|adapter| adapter.guid == adapter_id && adapter.hardware) {
                continue;
            }
            let network = connection.GetNetwork()?;
            let entry = Network { id: format!("{:?}", network.GetNetworkId()?), name: network.GetName()?.to_string() };
            if !networks.contains(&entry) {
                networks.push(entry);
            }
        }
        Ok(networks)
    }
}

fn system_proxy() -> Proxy {
    let key = w!(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings");
    unsafe {
        let mut enabled = 0u32;
        let mut size = size_of::<u32>() as u32;
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            w!("ProxyEnable"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut enabled).cast()),
            Some(&mut size),
        );
        if status != ERROR_SUCCESS || enabled == 0 {
            return Proxy::default();
        }
        let mut buffer = [0u16; 1024];
        let mut size = size_of_val(&buffer) as u32;
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            w!("ProxyServer"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        );
        if status != ERROR_SUCCESS {
            return Proxy::default();
        }
        let server = text(&buffer);
        let parsed = parse_proxy(&server);
        Proxy {
            local: parsed.is_some(),
            listening: parsed.is_some_and(|port| listening_ports().contains(&port)),
            server: Some(server),
        }
    }
}

/// Port of a proxy on this machine, or None when the proxy is remote or unparsable.
/// Accepts `host:port`, `http=host:port;https=host:port` and `scheme://host:port`.
pub fn parse_proxy(server: &str) -> Option<u16> {
    let entry = server.split(';').next()?.trim();
    let entry = entry.split_once('=').map_or(entry, |(_, value)| value);
    let entry = entry.split_once("://").map_or(entry, |(_, value)| value);
    let (host, port) = entry.rsplit_once(':')?;
    let local = host.eq_ignore_ascii_case("localhost") || host == "[::1]" || host.starts_with("127.");
    local.then(|| port.parse().ok()).flatten()
}

fn listening_ports() -> Vec<u16> {
    unsafe {
        let mut size = 0u32;
        let probe = GetExtendedTcpTable(None, &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0);
        if probe != ERROR_INSUFFICIENT_BUFFER.0 {
            return Vec::new();
        }
        // u32 storage satisfies the table's alignment.
        let mut buffer = vec![0u32; (size as usize).div_ceil(size_of::<u32>())];
        let status = GetExtendedTcpTable(
            Some(buffer.as_mut_ptr().cast()),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if status != ERROR_SUCCESS.0 {
            return Vec::new();
        }
        let table = buffer.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
        std::slice::from_raw_parts((*table).table.as_ptr(), (*table).dwNumEntries as usize)
            .iter()
            .map(|row| u16::from_be(row.dwLocalPort as u16))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::parse_proxy;

    #[test]
    fn parses_local_proxy_forms() {
        assert_eq!(parse_proxy("127.0.0.1:2080"), Some(2080));
        assert_eq!(parse_proxy("http=127.0.0.1:2080;https=127.0.0.1:2080"), Some(2080));
        assert_eq!(parse_proxy("socks5://localhost:1080"), Some(1080));
        assert_eq!(parse_proxy("[::1]:7890"), Some(7890));
    }

    #[test]
    fn rejects_remote_or_invalid_proxy() {
        assert_eq!(parse_proxy("proxy.corp.local:8080"), None);
        assert_eq!(parse_proxy("10.0.0.1:3128"), None);
        assert_eq!(parse_proxy("127.0.0.1"), None);
        assert_eq!(parse_proxy(""), None);
    }
}
