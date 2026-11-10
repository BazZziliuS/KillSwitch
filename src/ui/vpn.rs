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
            ui.add_space(20.0);
        });
    }
}
