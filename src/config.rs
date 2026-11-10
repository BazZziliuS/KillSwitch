use crate::i18n::Language;
use serde::{Deserialize, Serialize};
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
    pub trusted: Vec<TrustedNetwork>,
    pub tun_enabled: bool,
    /// Adapter aliases counted as the VPN tunnel. Other tunnels (e.g. Tailscale) must not satisfy the check.
    pub tun_adapters: Vec<String>,
    pub proxy_enabled: bool,
    /// `None` follows the Windows display language.
    pub language: Option<Language>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            apps: Vec::new(),
            trusted: Vec::new(),
            tun_enabled: true,
            tun_adapters: vec!["throne-tun".into()],
            proxy_enabled: true,
            language: None,
        }
    }
}

impl Config {
    pub fn can_enable(&self) -> bool {
        !self.apps.is_empty() && !self.trusted.is_empty() && (self.tun_enabled || self.proxy_enabled)
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
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| ".".into());
    base.join("KillSwitch").join("config.json")
}

/// Blocking the VPN client itself would stop it from ever reconnecting.
pub fn is_vpn_client(path: &Path) -> bool {
    const CLIENTS: [&str; 7] = ["throne", "thronecore", "happ", "xray", "sing-box", "nekoray", "nekobox"];
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| CLIENTS.contains(&stem.to_lowercase().as_str()))
}
