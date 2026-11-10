use crate::i18n::Language;
use regex::{RegexSet, RegexSetBuilder};
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
    /// Regular expressions; a domain matching any of them is neither offered by the site search nor blocked.
    pub site_filters: Vec<String>,
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
            site_filters: Vec::new(),
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

fn compile_filters<'a>(patterns: impl IntoIterator<Item = &'a String>) -> Result<RegexSet, regex::Error> {
    RegexSetBuilder::new(patterns).case_insensitive(true).build()
}

pub fn is_valid_filter(pattern: &str) -> bool {
    !pattern.is_empty() && compile_filters([&pattern.to_owned()]).is_ok()
}

/// Compiled form of `Config::site_filters`, rebuilt only when the patterns change.
#[derive(Default)]
pub struct HostFilter {
    patterns: Vec<String>,
    compiled: Option<RegexSet>,
}

impl HostFilter {
    pub fn update(&mut self, patterns: &[String]) {
        if self.patterns != patterns {
            self.patterns = patterns.to_vec();
            // Settings edited by hand may hold a broken pattern; it must not disable the others.
            self.compiled = compile_filters(patterns.iter().filter(|pattern| is_valid_filter(pattern))).ok();
        }
    }

    pub fn excludes(&self, host: &str) -> bool {
        self.compiled.as_ref().is_some_and(|set| set.is_match(host))
    }
}

/// A site entry such as `*inst*`: `*` stands for any characters, dots included.
pub fn is_pattern(site: &str) -> bool {
    site.contains('*')
}

/// Patterns need enough literal text not to match nearly every domain.
pub fn normalize_pattern(input: &str) -> Option<String> {
    const MIN_LITERAL: usize = 3;
    let pattern = input.trim().to_lowercase();
    let valid = pattern.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'*'));
    let literal = pattern.bytes().filter(|byte| *byte != b'*').count();
    (valid && is_pattern(&pattern) && literal >= MIN_LITERAL).then_some(pattern)
}

pub fn pattern_matches(pattern: &str, host: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = host.strip_prefix(first) else { return false };
    let mut middle: Vec<&str> = parts.collect();
    let last = middle.pop().unwrap_or_default();
    for part in middle {
        let Some(found) = rest.find(part) else { return false };
        rest = &rest[found + part.len()..];
    }
    rest.ends_with(last)
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
    use super::{Config, HostFilter, is_valid_filter, normalize_pattern, normalize_site, pattern_matches};

    #[test]
    fn patterns_match_any_subdomain_or_zone() {
        for host in ["instagram.com", "www.instagram.com", "scontent.cdninstagram.com", "instagram.co.uk"] {
            assert!(pattern_matches("*inst*", host), "{host}");
        }
        assert!(!pattern_matches("*inst*", "example.com"));
        assert!(pattern_matches("*.example.*", "a.example.org"));
        assert!(!pattern_matches("*.example.*", "example.org"));
        assert!(pattern_matches("inst*", "instagram.com"));
        assert!(!pattern_matches("inst*", "myinstagram.com"));
        assert!(pattern_matches("*.ru", "example.ru"));
        assert!(!pattern_matches("*.ru", "example.ru.com"));
        assert!(!pattern_matches("abc*abc", "abc"));
    }

    #[test]
    fn patterns_need_real_text() {
        assert_eq!(normalize_pattern(" *INST* ").as_deref(), Some("*inst*"));
        for input in ["*", "**", "*a*", "*in st*", "inst", "https://*inst*/"] {
            assert_eq!(normalize_pattern(input), None, "{input}");
        }
    }

    fn filter(patterns: &[&str]) -> HostFilter {
        let mut filter = HostFilter::default();
        filter.update(&patterns.iter().map(|pattern| (*pattern).to_owned()).collect::<Vec<_>>());
        filter
    }

    #[test]
    fn filters_match_keywords_and_anchored_patterns() {
        let filter = filter(&["ads", r"^cdn\.", r"\.ru$"]);
        assert!(filter.excludes("my-ads-server.example.com"));
        assert!(filter.excludes("ADS.example.com"));
        assert!(filter.excludes("cdn.example.com"));
        assert!(filter.excludes("example.ru"));
        assert!(!filter.excludes("static.cdn.example.com"));
        assert!(!filter.excludes("example.com"));
    }

    #[test]
    fn no_filters_exclude_nothing() {
        assert!(!filter(&[]).excludes("example.com"));
    }

    #[test]
    fn a_broken_pattern_does_not_disable_the_others() {
        let filter = filter(&["(unclosed", "ads"]);
        assert!(filter.excludes("ads.example.com"));
        assert!(!filter.excludes("example.com"));
    }

    #[test]
    fn invalid_or_empty_patterns_are_refused() {
        assert!(is_valid_filter(r"^ads\."));
        assert!(!is_valid_filter("(unclosed"));
        assert!(!is_valid_filter(""));
    }

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
