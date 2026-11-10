use super::widgets::{self, Row, Trailing, section};
use super::{App, Page, View};
use crate::config::Config;
use crate::protection::rules;
use eframe::egui::{self, Key, Ui};
use std::net::IpAddr;

const MAX_SERVER_NAME: usize = 40;

impl App {
    pub(super) fn vpn_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text, status) = (&view.palette, view.text, view.status);
        if widgets::nav_bar(ui, palette, text.vpn_modes, text.back, None).back {
            self.page = Page::Home;
        }
        egui::ScrollArea::vertical().id_salt("vpn").auto_shrink(false).show(ui, |ui| {
            let tun_on = rules::tun_active(config, &status.snapshot);
            let proxy = &status.snapshot.proxy;
            let tun_label = if tun_on { text.on } else { text.off };
            let tun_color = if tun_on { palette.green } else { palette.secondary };
            let proxy_label = if proxy.active() { proxy.server.as_deref().unwrap_or(text.on) } else { text.off };
            let proxy_color = if proxy.active() { palette.green } else { palette.secondary };

            section(ui, palette, None, Some(text.modes_footer), |group| {
                let tun = Row::new("tun", "TUN", Trailing::Switch { on: config.tun_enabled, enabled: true })
                    .subtitle(tun_label, tun_color);
                if group.row(tun).trailing_clicked {
                    config.tun_enabled = !config.tun_enabled;
                }
                let proxy_row = Row::new("proxy", text.system_proxy, Trailing::Switch { on: config.proxy_enabled, enabled: true })
                    .subtitle(proxy_label, proxy_color);
                if group.row(proxy_row).trailing_clicked {
                    config.proxy_enabled = !config.proxy_enabled;
                }
            });
            if config.tun_enabled {
                section(ui, palette, Some(text.tun_adapter), Some(text.adapter_footer), |group| {
                    for adapter in status.snapshot.virtual_adapters() {
                        let selected = config.tun_adapters.iter().any(|name| rules::adapter_matches(&adapter.name, name));
                        if group.row(Row::new(&adapter.name, &adapter.name, Trailing::Check(selected))).clicked {
                            if selected {
                                config.tun_adapters.retain(|name| !rules::adapter_matches(&adapter.name, name));
                            } else {
                                config.tun_adapters.push(adapter.name.clone());
                            }
                        }
                    }
                });
            }
            if status.ipv6_guarded {
                widgets::footnote(ui, text.ipv6_guarded, palette.orange);
            }
            if let Some(address) = server_section(ui, view, config) {
                self.search = config.server_name(address).unwrap_or_default().to_owned();
                self.editing_server = Some(address);
                self.page = Page::ServerName;
            }
            ui.add_space(20.0);
        });
    }

    pub(super) fn server_name_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let nav = widgets::nav_bar(ui, palette, text.server_name, text.back, Some(text.done));
        ui.add_space(6.0);
        let field = widgets::text_field(ui, palette, &mut self.search, text.server_name_hint);
        let submitted = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        if let Some(address) = self.editing_server {
            widgets::footnote(ui, &address.to_string(), palette.secondary);
            if nav.done || submitted {
                let name: String = self.search.trim().chars().take(MAX_SERVER_NAME).collect();
                if name.is_empty() {
                    config.server_names.remove(&address.to_string());
                } else {
                    config.server_names.insert(address.to_string(), name);
                }
            }
        }
        if nav.back || nav.done || submitted {
            self.editing_server = None;
            self.page = Page::Vpn;
        }
    }
}

/// Returns the server the user clicked to rename.
fn server_section(ui: &mut Ui, view: View<'_>, config: &mut Config) -> Option<IpAddr> {
    let (palette, text) = (&view.palette, view.text);
    let mut rename = None;
    let current = view.status.snapshot.exit_ip;
    let mut toggle_current = false;
    let mut remove = None;
    section(ui, palette, Some(text.servers), Some(text.servers_footer), |group| {
        let row = Row::new("server-check", text.server_check, Trailing::Switch { on: config.server_check, enabled: true });
        if group.row(row).trailing_clicked {
            config.server_check = !config.server_check;
        }
        if !config.server_check {
            return;
        }
        match current {
            Some(address) => {
                let allowed = config.allowed_servers.contains(&address);
                let ip = address.to_string();
                let (title, subtitle) = match config.server_name(address) {
                    Some(name) => (name.to_owned(), format!("{ip} · {}", text.server_current)),
                    None => (ip, text.server_current.to_owned()),
                };
                let mut row = Row::new("server-current", &title, Trailing::Switch { on: allowed, enabled: true })
                    .subtitle(&subtitle, palette.secondary);
                if allowed {
                    row = row.tappable();
                }
                let output = group.row(row);
                toggle_current = output.trailing_clicked;
                if output.clicked && allowed {
                    rename = Some(address);
                }
            }
            None => {
                group.row(Row::new("server-detecting", text.server_detecting, Trailing::None));
            }
        }
        for (index, address) in config.allowed_servers.iter().enumerate().filter(|(_, address)| Some(**address) != current) {
            let ip = address.to_string();
            let (title, subtitle) = match config.server_name(*address) {
                Some(name) => (name, ip.as_str()),
                None => (ip.as_str(), text.server_allowed),
            };
            let output = group.row(Row::new(&ip, title, Trailing::Remove).subtitle(subtitle, palette.secondary).tappable());
            if output.trailing_clicked {
                remove = Some(index);
            } else if output.clicked {
                rename = Some(*address);
            }
        }
    });
    if toggle_current && let Some(address) = current {
        if config.allowed_servers.contains(&address) {
            config.allowed_servers.retain(|entry| *entry != address);
            config.server_names.remove(&address.to_string());
        } else {
            config.allowed_servers.push(address);
        }
    }
    if let Some(index) = remove {
        let address = config.allowed_servers.remove(index);
        config.server_names.remove(&address.to_string());
    }
    rename
}
