use crate::i18n::Language;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustedNetwork {
    pub id: String,
    pub name: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub apps: Vec<PathBuf>,
    /// Domains made unreachable from every app, except VPN clients, while protection is blocking.
    pub sites: Vec<String>,
    pub trusted: Vec<TrustedNetwork>,
    pub tun_enabled: bool,
    /// Adapter aliases counted as the VPN tunnel. Other tunnels (e.g. Tailscale) must not satisfy the check.
    pub tun_adapters: Vec<String>,
    pub proxy_enabled: bool,
    /// Off by default because the check contacts a third-party service.
    pub server_check: bool,
    /// Public addresses of the VPN servers the protected traffic may leave through.
    pub allowed_servers: Vec<IpAddr>,
    /// User-given names, keyed by address text. Separate from `allowed_servers` so that existing settings keep loading.
    pub server_names: BTreeMap<String, String>,
    pub autostart: bool,
    pub notifications: bool,
    pub overlay: bool,
    pub update_check: bool,
    /// `None` follows the Windows display language.
    pub language: Option<Language>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            apps: Vec::new(),
            sites: Vec::new(),
            trusted: Vec::new(),
            tun_enabled: true,
            tun_adapters: vec!["throne-tun".into()],
            proxy_enabled: true,
            server_check: false,
            allowed_servers: Vec::new(),
            server_names: BTreeMap::new(),
            autostart: false,
            notifications: true,
            overlay: true,
            update_check: true,
            language: None,
        }
    }
}

impl Config {
    pub fn can_enable(&self) -> bool {
        (!self.apps.is_empty() || !self.sites.is_empty())
            && !self.trusted.is_empty()
            && (self.tun_enabled || self.proxy_enabled)
            && (!self.server_check || !self.allowed_servers.is_empty())
    }

    pub fn server_name(&self, address: IpAddr) -> Option<&str> {
        self.server_names.get(&address.to_string()).map(String::as_str)
    }

    pub fn load() -> Self {
        std::fs::read(path())
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
    }
}

fn path() -> PathBuf {
    data_path("config.json")
}

pub fn data_path(file: &str) -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| ".".into());
    base.join("KillSwitch").join(file)
}

/// Reduces a typed address or pasted URL to its bare domain. Non-ASCII domains must be given in punycode.
pub fn normalize_site(input: &str) -> Option<String> {
    let lowered = input.trim().to_lowercase();
    let rest = lowered.split_once("://").map_or(lowered.as_str(), |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host).split(':').next()?;
    let host = host.strip_prefix("www.").unwrap_or(host).trim_end_matches('.');
    let label_ok = |label: &str| {
        !label.is_empty() && label.len() <= 63 && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    };
    (host.contains('.') && host.len() <= 253 && host.split('.').all(label_ok)).then(|| host.to_owned())
}

/// Blocking the VPN client itself would stop it from ever reconnecting.
pub fn is_vpn_client(path: &Path) -> bool {
    const CLIENTS: [&str; 7] = ["throne", "thronecore", "happ", "xray", "sing-box", "nekoray", "nekobox"];
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| CLIENTS.contains(&stem.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::{Config, normalize_site};

    #[test]
    fn settings_from_older_versions_still_load() {
        let old = r#"{"enabled":true,"sites":["example.com"],"server_check":true,"allowed_servers":["203.0.113.1"]}"#;
        let config: Config = serde_json::from_str(old).unwrap();
        assert!(config.enabled && config.notifications && config.overlay && config.update_check && !config.autostart);
        assert_eq!(config.allowed_servers, ["203.0.113.1".parse::<std::net::IpAddr>().unwrap()]);
        assert_eq!(config.server_name(config.allowed_servers[0]), None);
    }

    #[test]
    fn reduces_urls_to_the_domain() {
        for input in ["example.com", " Example.COM ", "www.example.com", "https://www.example.com:8443/a?b#c", "https://user@example.com/"] {
            assert_eq!(normalize_site(input).as_deref(), Some("example.com"), "{input}");
        }
        assert_eq!(normalize_site("a.b.example.com").as_deref(), Some("a.b.example.com"));
        assert_eq!(normalize_site("xn--e1afmkfd.xn--p1ai").as_deref(), Some("xn--e1afmkfd.xn--p1ai"));
    }

    #[test]
    fn rejects_what_is_not_a_domain() {
        for input in ["", "localhost", "exa mple.com", "пример.рф", "example..com", "https://"] {
            assert_eq!(normalize_site(input), None, "{input}");
        }
    }
}
