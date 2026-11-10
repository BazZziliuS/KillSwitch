//! Asks a public echo service which address the internet sees, which identifies the VPN server in use.
//!
//! The connection table can't answer this: VPN cores also connect straight to sites excluded from the tunnel, and UDP
//! protocols leave no remote address at all.

use std::ffi::c_void;
use std::net::IpAddr;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::Networking::WinHttp::*;
use windows::core::{HSTRING, PCWSTR, w};

/// IPv4-only hosts, so that one server always yields one comparable address.
const SERVICES: [PCWSTR; 2] = [w!("api.ipify.org"), w!("ipv4.icanhazip.com")];
const TIMEOUT_MS: i32 = 5000;
const MAX_ADDRESS_RESPONSE: usize = 64;

struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void, what: &str) -> Result<Self, String> {
        if raw.is_null() { Err(format!("{what}: {:?}", unsafe { GetLastError() })) } else { Ok(Self(raw)) }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

/// `proxy` is the Windows proxy string to go through; `None` connects directly, which follows the TUN route.
pub fn public_ip(proxy: Option<&str>) -> Result<IpAddr, String> {
    let mut last_error = String::new();
    for service in SERVICES {
        match query(service, proxy) {
            Ok(address) => return Ok(address),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn query(host: PCWSTR, proxy: Option<&str>) -> Result<IpAddr, String> {
    let body = fetch(host, w!("/"), proxy, MAX_ADDRESS_RESPONSE)?;
    parse_address(&body).ok_or_else(|| "unexpected response".to_owned())
}

fn parse_address(body: &[u8]) -> Option<IpAddr> {
    std::str::from_utf8(body).ok()?.trim().parse().ok()
}

/// Performs an HTTPS GET and returns at most `limit` bytes of the body.
pub fn fetch(host: PCWSTR, path: PCWSTR, proxy: Option<&str>, limit: usize) -> Result<Vec<u8>, String> {
    let proxy = proxy.map(HSTRING::from);
    let (access, proxy_name) = match &proxy {
        Some(name) => (WINHTTP_ACCESS_TYPE_NAMED_PROXY, PCWSTR(name.as_ptr())),
        None => (WINHTTP_ACCESS_TYPE_NO_PROXY, PCWSTR::null()),
    };
    unsafe {
        // Sites commonly refuse clients that don't identify as a browser.
        let agent = w!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) KillSwitch");
        let session = Handle::new(WinHttpOpen(agent, access, proxy_name, PCWSTR::null(), 0), "open session")?;
        WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS).map_err(|error| error.to_string())?;
        let connection = Handle::new(WinHttpConnect(session.0, host, INTERNET_DEFAULT_HTTPS_PORT, 0), "connect")?;
        let request = Handle::new(
            WinHttpOpenRequest(
                connection.0,
                w!("GET"),
                path,
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "open request",
        )?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(|error| error.to_string())?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|error| error.to_string())?;

        let mut body = vec![0u8; limit];
        let mut filled = 0usize;
        while filled < limit {
            let mut read = 0u32;
            let free = (limit - filled) as u32;
            WinHttpReadData(request.0, body[filled..].as_mut_ptr().cast(), free, &mut read).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            filled += read as usize;
        }
        body.truncate(filled);
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_address;

    #[test]
    fn accepts_only_a_bare_address() {
        assert_eq!(parse_address(b"203.0.113.7\n"), Some("203.0.113.7".parse().unwrap()));
        assert_eq!(parse_address(b"<html>captive portal</html>"), None);
        assert_eq!(parse_address(b""), None);
    }
}
