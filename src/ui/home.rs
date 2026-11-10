use super::theme::{self, Palette};
use super::widgets::{self, Glyph, Row, Trailing, section};
use super::{App, Page, View};
use crate::config::{Config, TrustedNetwork};
use crate::i18n::{Language, Strings};
use crate::protection::monitor::Status;
use crate::protection::rules::{self, Reason};
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Ui, Vec2};

const COLOR_ANIMATION: f32 = 0.3;
const HERO_SIZE: f32 = 76.0;

struct Hero {
    color: Color32,
    glyph: Glyph,
    title: &'static str,
    subtitle: String,
    trust: Option<TrustedNetwork>,
}

impl App {
    pub(super) fn home_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text, status) = (&view.palette, view.text, view.status);

        // The status and the main switch stay pinned so a long list can never hide why the internet is off.
        if let Some(network) = paint_hero(ui, view, hero_state(view, config)) {
            config.trusted.push(network);
        }
        self.show_notice(ui, view);

        let can_toggle = config.enabled || config.can_enable();
        let requirements = requirements(config, text);
        section(ui, palette, None, requirements.as_deref(), |group| {
            let row = Row::new("protection", text.protection, Trailing::Switch { on: config.enabled, enabled: can_toggle });
            if group.row(row).trailing_clicked {
                config.enabled = !config.enabled;
            }
        });
        ui.add_space(4.0);

        egui::ScrollArea::vertical().id_salt("home").auto_shrink(false).show(ui, |ui| {
            self.apps_section(ui, view, config);
            if sites_section(ui, view, config) {
                self.open_add_site();
            }
            networks_section(ui, view, config);

            let (label, color) = vpn_summary(palette, text, config, status);
            let language = config.language.map_or(text.system_language, Language::native_name);
            let mut target = None;
            section(ui, palette, Some("VPN"), None, |group| {
                let trailing = Trailing::Chevron { value: Some((label, color)) };
                if group.row(Row::new("vpn", text.modes, trailing)).clicked {
                    target = Some(Page::Vpn);
                }
            });
            section(ui, palette, None, None, |group| {
                let trailing = Trailing::Chevron { value: Some((language, palette.secondary)) };
                if group.row(Row::new("language", text.language, trailing)).clicked {
                    target = Some(Page::Language);
                }
            });
            if let Some(page) = target {
                self.page = page;
            }

            ui.add_space(22.0);
            ui.vertical_centered(|ui| {
                let label = RichText::new(text.quit_app).font(theme::callout()).color(palette.secondary);
                if ui.add(egui::Label::new(label).sense(Sense::click())).clicked() {
                    if config.enabled {
                        self.confirm_quit = true;
                    } else {
                        self.shared.quit();
                    }
                }
            });
            ui.add_space(20.0);
        });
    }

    fn apps_section(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let ctx = ui.ctx().clone();
        let mut remove = None;
        let mut open_add = false;
        let catalog = &mut self.catalog;
        section(ui, palette, Some(text.apps), Some(text.apps_footer), |group| {
            for (index, path) in config.apps.iter().enumerate() {
                let info = catalog.get(&ctx, path);
                let id = path.to_string_lossy();
                let mut row = Row::new(&id, &info.name, Trailing::Remove).leading(info.leading());
                if view.status.missing.contains(path) {
                    row = row.subtitle(text.file_missing, palette.orange);
                }
                if group.row(row).trailing_clicked {
                    remove = Some(index);
                }
            }
            open_add = group.row(Row::new("add", text.add_app, Trailing::None).action(palette.accent)).clicked;
        });
        if let Some(index) = remove {
            config.apps.remove(index);
        }
        if open_add {
            self.open_add_apps();
        }
    }
}

/// Returns true when the user asked to add a site.
fn sites_section(ui: &mut Ui, view: View<'_>, config: &mut Config) -> bool {
    let (palette, text) = (&view.palette, view.text);
    let mut remove = None;
    let mut add = false;
    section(ui, palette, Some(text.sites), Some(text.sites_footer), |group| {
        for (index, site) in config.sites.iter().enumerate() {
            let mut row = Row::new(site, site, Trailing::Remove);
            if view.status.unresolved_sites.contains(site) {
                row = row.subtitle(text.site_unresolved, palette.orange);
            }
            if group.row(row).trailing_clicked {
                remove = Some(index);
            }
        }
        add = group.row(Row::new("add-site", text.add_site, Trailing::None).action(palette.accent)).clicked;
    });
    if let Some(index) = remove {
        config.sites.remove(index);
    }
    add
}

fn networks_section(ui: &mut Ui, view: View<'_>, config: &mut Config) {
    let (palette, text, networks) = (&view.palette, view.text, &view.status.snapshot.networks);
    let mut toggled = None;
    let mut removed = None;
    section(ui, palette, Some(text.trusted_networks), Some(text.networks_footer), |group| {
        for network in networks {
            let trusted = config.trusted.iter().any(|entry| entry.id == network.id);
            let row = Row::new(&network.id, &network.name, Trailing::Switch { on: trusted, enabled: true })
                .subtitle(text.connected_now, palette.secondary);
            if group.row(row).trailing_clicked {
                toggled = Some((network, trusted));
            }
        }
        let offline = |entry: &TrustedNetwork| !networks.iter().any(|network| network.id == entry.id);
        for (index, entry) in config.trusted.iter().enumerate().filter(|(_, entry)| offline(entry)) {
            let row = Row::new(&entry.id, &entry.name, Trailing::Remove).subtitle(text.not_connected, palette.secondary);
            if group.row(row).trailing_clicked {
                removed = Some(index);
            }
        }
        if networks.is_empty() && config.trusted.is_empty() {
            group.row(Row::new("none", text.no_connection, Trailing::None).subtitle(text.connect_to_mark, palette.secondary));
        }
    });
    if let Some((network, trusted)) = toggled {
        if trusted {
            config.trusted.retain(|entry| entry.id != network.id);
        } else {
            config.trusted.push(TrustedNetwork { id: network.id.clone(), name: network.name.clone() });
        }
    }
    if let Some(index) = removed {
        config.trusted.remove(index);
    }
}

fn paint_hero(ui: &mut Ui, view: View<'_>, hero: Hero) -> Option<TrustedNetwork> {
    let palette = &view.palette;
    let mut accepted = None;
    let animated = |ui: &Ui, channel: &str, value: u8| {
        ui.ctx().animate_value_with_time(egui::Id::unique(("hero", channel)), f32::from(value), COLOR_ANIMATION) as u8
    };
    ui.vertical_centered(|ui| {
        ui.add_space(10.0);
        let color = Color32::from_rgb(
            animated(ui, "r", hero.color.r()),
            animated(ui, "g", hero.color.g()),
            animated(ui, "b", hero.color.b()),
        );
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(HERO_SIZE), Sense::hover());
        widgets::shield(ui, rect, color, hero.glyph);
        ui.add_space(12.0);
        ui.label(RichText::new(hero.title).font(theme::title()).color(palette.text));
        ui.add_space(4.0);
        ui.label(RichText::new(hero.subtitle).font(theme::callout()).color(palette.secondary));
        if let Some(network) = hero.trust {
            ui.add_space(12.0);
            let label = RichText::new(view.text.trust_network).font(theme::headline()).color(Color32::WHITE);
            let button = egui::Button::new(label).fill(palette.accent).corner_radius(CornerRadius::same(16));
            if ui.add_sized(Vec2::new(190.0, 34.0), button).clicked() {
                accepted = Some(network);
            }
        }
    });
    accepted
}

fn hero_state(view: View<'_>, config: &Config) -> Hero {
    let (palette, text, status) = (&view.palette, view.text, view.status);
    let trust = status
        .snapshot
        .networks
        .iter()
        .find(|network| !config.trusted.iter().any(|entry| entry.id == network.id))
        .map(|network| TrustedNetwork { id: network.id.clone(), name: network.name.clone() });

    if let Some(error) = &status.firewall_error {
        let subtitle = if error.contains("0x00000005") { text.needs_admin.to_owned() } else { (text.rules_failed)(error) };
        return Hero { color: palette.orange, glyph: Glyph::Exclamation, title: text.failed_title, subtitle, trust: None };
    }
    if status.blocked {
        let subtitle = status.reasons.iter().map(|reason| explain(reason, text)).collect::<Vec<_>>().join(" ");
        return Hero { color: palette.red, glyph: Glyph::Cross, title: text.blocked_title, subtitle, trust };
    }
    if status.enabled {
        return Hero {
            color: palette.green,
            glyph: Glyph::Check,
            title: text.protected_title,
            subtitle: text.protected_subtitle.to_owned(),
            trust: None,
        };
    }
    Hero {
        color: palette.tertiary,
        glyph: Glyph::Dash,
        title: text.off_title,
        subtitle: text.off_subtitle.to_owned(),
        trust: None,
    }
}

fn explain(reason: &Reason, text: &Strings) -> String {
    match reason {
        Reason::NoNetwork => text.reason_no_network.to_owned(),
        Reason::UntrustedNetwork(name) => (text.reason_untrusted)(name),
        Reason::VpnOff => text.reason_vpn_off.to_owned(),
        Reason::DetectionFailed(_) => text.reason_detection_failed.to_owned(),
        Reason::ForeignServer(address) => (text.reason_foreign_server)(&address.to_string()),
        Reason::ServerUnknown => text.reason_server_unknown.to_owned(),
    }
}

fn requirements(config: &Config, text: &Strings) -> Option<String> {
    let mut steps = Vec::new();
    if config.apps.is_empty() && config.sites.is_empty() {
        steps.push(text.step_add_app);
    }
    if config.trusted.is_empty() {
        steps.push(text.step_trust_network);
    }
    if !config.tun_enabled && !config.proxy_enabled {
        steps.push(text.step_enable_mode);
    }
    if config.server_check && config.allowed_servers.is_empty() {
        steps.push(text.step_allow_server);
    }
    (!steps.is_empty() && !config.enabled).then(|| (text.to_enable)(&steps.join(text.step_separator)))
}

fn vpn_summary(palette: &Palette, text: &'static Strings, config: &Config, status: &Status) -> (&'static str, Color32) {
    if config.tun_enabled && rules::tun_active(config, &status.snapshot) {
        ("TUN", palette.green)
    } else if config.proxy_enabled && status.snapshot.proxy.active() {
        (text.proxy_short, palette.green)
    } else {
        (text.off, palette.secondary)
    }
}
