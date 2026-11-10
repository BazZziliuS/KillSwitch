//! Finds the hosts a site talks to by reading its page and scripts as text.
//!
//! Nothing is executed: this process runs as administrator and must never run a website's code. Hosts that a page only
//! computes in JavaScript at run time are therefore not found.

use super::egress;
use crate::config;
use std::collections::BTreeSet;
use windows::core::{HSTRING, PCWSTR};

const MAX_PAGE: usize = 2 * 1024 * 1024;
const MAX_SCRIPTS: usize = 8;
const MAX_HOSTS: usize = 100;
/// Appear in nearly every page as XML namespaces or metadata, without any connection being made to them.
const NOT_CONNECTIONS: [&str; 5] = ["w3.org", "schema.org", "ogp.me", "purl.org", "xmlsoap.org"];

#[derive(Debug, Default, PartialEq)]
pub struct Found {
    pub subdomains: Vec<String>,
    pub services: Vec<String>,
}

fn is_host_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.'
}

fn valid_host(host: &str) -> Option<String> {
    let host = config::normalize_site(host)?;
    let tld = host.rsplit('.').next()?;
    (tld.len() >= 2 && tld.bytes().all(|byte| byte.is_ascii_alphabetic())).then_some(host)
}

/// Absolute and protocol-relative URLs in the text, as (host, path).
fn urls(text: &str) -> Vec<(String, String)> {
    let text = text.replace("\\/", "/");
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut start = 0;
    while let Some(offset) = text[start..].find("//") {
        let host_start = start + offset + 2;
        start = host_start;
        let before = bytes[..host_start - 2].last().copied().unwrap_or(b' ');
        // "https://" or a protocol-relative value such as src="//cdn.example.com/a.js"; not a "// comment".
        if !matches!(before, b':' | b'"' | b'\'' | b'(' | b'=') {
            continue;
        }
        let host_end = text[host_start..].bytes().position(|byte| !is_host_byte(byte)).map_or(text.len(), |end| host_start + end);
        let Some(host) = valid_host(&text[host_start..host_end]) else { continue };
        let path_end = text[host_end..]
            .bytes()
            .position(|byte| matches!(byte, b'"' | b'\'' | b')' | b' ' | b'<' | b'>' | b'\n'))
            .map_or(text.len(), |end| host_end + end);
        found.push((host, text[host_end..path_end].to_owned()));
    }
    found
}

/// Every token in the text naming a subdomain of `site`, which also catches hosts mentioned without a URL.
fn subdomain_tokens(text: &str, site: &str) -> Vec<String> {
    let suffix = format!(".{site}");
    text.split(|character: char| !character.is_ascii() || !is_host_byte(character as u8))
        .filter(|token| token.to_ascii_lowercase().ends_with(&suffix))
        .filter_map(valid_host)
        .collect()
}

fn is_subdomain(host: &str, site: &str) -> bool {
    host.strip_suffix(site).is_some_and(|rest| rest.ends_with('.'))
}

fn collect(text: &str, site: &str, hosts: &mut BTreeSet<String>) {
    hosts.extend(urls(text).into_iter().map(|(host, _)| host));
    hosts.extend(subdomain_tokens(text, site));
}

/// Scripts on the site itself, which is where its other hosts are usually configured.
fn own_scripts(text: &str, site: &str) -> Vec<(String, String)> {
    let mut scripts: Vec<(String, String)> = urls(text)
        .into_iter()
        .filter(|(host, path)| (host == site || is_subdomain(host, site)) && path.split(['?', '#']).next().is_some_and(|path| path.ends_with(".js")))
        .collect();
    scripts.sort();
    scripts.dedup();
    scripts.truncate(MAX_SCRIPTS);
    scripts
}

fn get(host: &str, path: &str, proxy: Option<&str>) -> Option<String> {
    let (host, path) = (HSTRING::from(host), HSTRING::from(path));
    let body = egress::fetch(PCWSTR(host.as_ptr()), PCWSTR(path.as_ptr()), proxy, MAX_PAGE).ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

fn sort_out(hosts: BTreeSet<String>, site: &str) -> Found {
    let unrelated = |host: &str| NOT_CONNECTIONS.iter().any(|name| host == *name || is_subdomain(host, name));
    let mut found = Found::default();
    for host in hosts.into_iter().filter(|host| host != site && !unrelated(host)) {
        if is_subdomain(&host, site) { found.subdomains.push(host) } else { found.services.push(host) }
    }
    found.subdomains.truncate(MAX_HOSTS);
    found.services.truncate(MAX_HOSTS);
    found
}

/// `proxy` is the Windows proxy string to go through; `None` connects directly, which follows the TUN route.
pub fn scan(site: &str, proxy: Option<&str>) -> Found {
    let Some(page) = get(site, "/", proxy).or_else(|| get(&format!("www.{site}"), "/", proxy)) else {
        return Found::default();
    };
    let mut hosts = BTreeSet::new();
    collect(&page, site, &mut hosts);
    for (host, path) in own_scripts(&page, site) {
        if let Some(script) = get(&host, &path, proxy) {
            collect(&script, site, &mut hosts);
        }
    }
    sort_out(hosts, site)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
        <html xmlns="http://www.w3.org/1999/xhtml">
        <script src="https://static.example.com/app.js?v=1"></script>
        <script src="//cdn.other.net/lib.js"></script>
        <img src='https://img.example.com/a.png'>
        <style>a{background:url(https://fonts.vendor.io/f.woff)}</style>
        <script>// see notes.example.org for details
        var api = "https:\/\/api.example.com\/v1"; var ws = "wss://live.example.com/feed"; host = "media.example.com";</script>
        <a href="https://EXAMPLE.com/about">about</a>
    "#;

    #[test]
    fn separates_subdomains_from_other_services() {
        let mut hosts = BTreeSet::new();
        collect(PAGE, "example.com", &mut hosts);
        let found = sort_out(hosts, "example.com");
        assert_eq!(found.subdomains, ["api.example.com", "img.example.com", "live.example.com", "media.example.com", "static.example.com"]);
        assert_eq!(found.services, ["cdn.other.net", "fonts.vendor.io"]);
    }

    #[test]
    fn a_similar_name_is_not_a_subdomain() {
        assert!(is_subdomain("a.example.com", "example.com"));
        assert!(!is_subdomain("notexample.com", "example.com"));
        assert!(!is_subdomain("example.com", "example.com"));
    }

    #[test]
    fn follows_only_the_sites_own_scripts() {
        assert_eq!(own_scripts(PAGE, "example.com"), [("static.example.com".to_owned(), "/app.js?v=1".to_owned())]);
    }

    #[test]
    fn ignores_file_names_and_garbage() {
        assert!(urls("//app.js \"//version.1\" ://").is_empty());
    }
}
