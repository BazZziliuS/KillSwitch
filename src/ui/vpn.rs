use super::widgets::{self, Row, Trailing, section};
use super::{App, Page, View};
use crate::config::Config;
use crate::protection::rules;
use eframe::egui::{self, Ui};

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
            server_section(ui, view, config);
            ui.add_space(20.0);
        });
    }
}

fn server_section(ui: &mut Ui, view: View<'_>, config: &mut Config) {
    let (palette, text) = (&view.palette, view.text);
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
                let title = address.to_string();
                let row = Row::new("server-current", &title, Trailing::Switch { on: allowed, enabled: true })
                    .subtitle(text.server_current, palette.secondary);
                toggle_current = group.row(row).trailing_clicked;
            }
            None => {
                group.row(Row::new("server-detecting", text.server_detecting, Trailing::None));
            }
        }
        for (index, address) in config.allowed_servers.iter().enumerate().filter(|(_, address)| Some(**address) != current) {
            let title = address.to_string();
            let row = Row::new(&title, &title, Trailing::Remove).subtitle(text.server_allowed, palette.secondary);
            if group.row(row).trailing_clicked {
                remove = Some(index);
            }
        }
    });
    if toggle_current && let Some(address) = current {
        if config.allowed_servers.contains(&address) {
            config.allowed_servers.retain(|entry| *entry != address);
        } else {
            config.allowed_servers.push(address);
        }
    }
    if let Some(index) = remove {
        config.allowed_servers.remove(index);
    }
}
