//! Asks a public echo service which address the internet sees, which identifies the VPN server in use, and a
//! geolocation service which country that address belongs to.
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
const GEO_SERVICE: PCWSTR = w!("ipwho.is");
const MAX_GEO_RESPONSE: usize = 1024;

/// Where an address is registered, with the country named in each interface language.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Country {
    pub code: String,
    pub name_en: String,
    pub name_ru: String,
}

#[derive(serde::Deserialize)]
struct GeoAnswer {
    success: bool,
    #[serde(default)]
    country: String,
    #[serde(default)]
    country_code: String,
}

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

/// Looks the address up once per language, through the same route as the address check, so the service sees
/// only the VPN's address.
pub fn country(address: IpAddr, proxy: Option<&str>) -> Result<Country, String> {
    let lookup = |lang: &str| -> Result<GeoAnswer, String> {
        let path = HSTRING::from(format!("/{address}?lang={lang}&fields=success,country,country_code"));
        let body = fetch(GEO_SERVICE, PCWSTR(path.as_ptr()), proxy, MAX_GEO_RESPONSE)?;
        parse_geo(&body).ok_or_else(|| "unexpected response".to_owned())
    };
    let english = lookup("en")?;
    let russian = lookup("ru").map_or_else(|_| english.country.clone(), |answer| answer.country);
    Ok(Country { code: english.country_code, name_en: english.country, name_ru: russian })
}

fn parse_geo(body: &[u8]) -> Option<GeoAnswer> {
    serde_json::from_slice::<GeoAnswer>(body).ok().filter(|answer| answer.success && !answer.country.is_empty())
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
    use super::{parse_address, parse_geo};

    #[test]
    fn reads_the_country_from_the_geolocation_answer() {
        let answer = parse_geo(br#"{"success":true,"country":"Netherlands","country_code":"NL"}"#).unwrap();
        assert_eq!((answer.country.as_str(), answer.country_code.as_str()), ("Netherlands", "NL"));
        assert!(parse_geo(br#"{"success":false,"message":"Invalid IP address"}"#).is_none());
        assert!(parse_geo(b"<html>").is_none());
    }

    #[test]
    fn accepts_only_a_bare_address() {
        assert_eq!(parse_address(b"203.0.113.7\n"), Some("203.0.113.7".parse().unwrap()));
        assert_eq!(parse_address(b"<html>captive portal</html>"), None);
        assert_eq!(parse_address(b""), None);
    }
}
