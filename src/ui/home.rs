use super::theme;
use super::widgets::{self, Glyph, Leading, Row, Trailing, section};
use super::{App, Page, View};
use crate::config::{self, Config, TrustedNetwork};
use crate::protection::rules::{self, Reason};
use crate::updates;
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Ui, Vec2, vec2};
use std::net::IpAddr;
use std::path::PathBuf;

const COLOR_ANIMATION: f32 = 0.3;
const SHIELD_SIZE: f32 = 56.0;
const HERO_RADIUS: u8 = 16;

struct Hero {
    color: Color32,
    glyph: Glyph,
    title: &'static str,
    subtitle: String,
    action: Option<HeroAction>,
}

/// The one-click fix offered for the current block, as Kordon offers "allow this IP" on a VPN alert.
enum HeroAction {
    Trust(TrustedNetwork),
    Allow(IpAddr),
}

/// One program in the list, possibly installed in several folders.
struct Entry {
    name: String,
    paths: Vec<PathBuf>,
}

#[derive(Clone, Copy)]
enum Step {
    AddApp,
    TrustNetwork,
    EnableMode,
    AllowServer,
}

impl App {
    pub(super) fn home_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);

        // The status and the main switch stay pinned so a long list can never hide why the internet is off.
        ui.add_space(12.0);
        match status_card(ui, view, config, hero_state(view, config)) {
            Some(HeroAction::Trust(network)) => config.trusted.push(network),
            Some(HeroAction::Allow(address)) if !config.allowed_servers.contains(&address) => config.allowed_servers.push(address),
            Some(HeroAction::Allow(_)) => {}
            None => {}
        }
        self.show_notice(ui, view);

        let can_toggle = config.enabled || config.can_enable();
        section(ui, palette, None, None, |group| {
            let row = Row::new("protection", text.protection, Trailing::Switch { on: config.enabled, enabled: can_toggle });
            if group.row(row).trailing_clicked {
                config.enabled = !config.enabled;
            }
        });
        let update = self.shared.update.lock().unwrap().clone();
        if let Some(version) = update {
            section(ui, palette, None, None, |group| {
                let title = (text.update_available)(&version);
                if group.row(Row::new("update", &title, Trailing::None).action(palette.accent)).clicked {
                    updates::open_releases_page();
                }
            });
        }
        ui.add_space(4.0);

        egui::ScrollArea::vertical().id_salt("home").auto_shrink(false).show(ui, |ui| {
            if !config.enabled {
                self.setup_section(ui, view, config);
            }
            self.apps_section(ui, view, config);
            networks_section(ui, view, config);
            ui.add_space(widgets::TAB_BAR_CLEARANCE);
        });
    }

    /// Kordon-style checklist of what is still missing before protection can be turned on.
    fn setup_section(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let steps = [
            (Step::AddApp, text.step_add_app, !config.apps.is_empty() || !config.sites.is_empty(), true),
            (Step::TrustNetwork, text.step_trust_network, !config.trusted.is_empty(), true),
            (Step::EnableMode, text.step_enable_mode, config.tun_enabled || config.proxy_enabled, true),
            (Step::AllowServer, text.step_allow_server, !config.allowed_servers.is_empty(), config.server_check),
        ];
        if steps.iter().all(|(_, _, done, needed)| *done || !needed) {
            return;
        }
        let mut chosen = None;
        section(ui, palette, Some(text.setup_title), Some(text.setup_footer), |group| {
            for (step, title, done, needed) in steps {
                if !needed {
                    continue;
                }
                let trailing = if done { Trailing::None } else { Trailing::Chevron { value: None } };
                let mut row = Row::new(title, title, trailing).leading(Leading::Step(done));
                if done {
                    row.title_color = Some(palette.secondary);
                } else {
                    row = row.tappable();
                }
                if group.row(row).clicked && !done {
                    chosen = Some(step);
                }
            }
        });
        match chosen {
            Some(Step::AddApp) => self.open_add_apps(),
            Some(Step::TrustNetwork) => {
                if let Some(network) = view.status.snapshot.networks.first() {
                    config.trusted.push(TrustedNetwork { id: network.id.clone(), name: network.name.clone() });
                }
            }
            Some(Step::EnableMode | Step::AllowServer) => self.page = Page::Vpn,
            None => {}
        }
    }

    fn apps_section(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let ctx = ui.ctx().clone();

        // Settings from before names were remembered: learn each name once, from the file.
        let unnamed: Vec<PathBuf> = config.apps.iter().filter(|path| config.app_name(path).is_none()).cloned().collect();
        for path in unnamed {
            let name = self.catalog.get(&ctx, &path).name.clone();
            config.app_names.insert(path.to_string_lossy().into_owned(), name);
        }
        let mut entries: Vec<Entry> = Vec::new();
        for path in &config.apps {
            let name = config.app_name(path).unwrap_or_default().to_owned();
            match entries.iter_mut().find(|entry| config::same_name(&entry.name, &name)) {
                Some(entry) => entry.paths.push(path.clone()),
                None => entries.push(Entry { name, paths: vec![path.clone()] }),
            }
        }

        let mut remove = None;
        let mut open_add = false;
        let catalog = &mut self.catalog;
        section(ui, palette, Some(text.apps), Some(text.apps_footer), |group| {
            for (index, entry) in entries.iter().enumerate() {
                // The copy that still exists supplies the icon.
                let shown = entry.paths.iter().find(|path| !view.status.missing.contains(path)).unwrap_or(&entry.paths[0]);
                let info = catalog.get(&ctx, shown);
                let copies = (text.app_copies)(entry.paths.len());
                let mut row = Row::new(&entry.name, &entry.name, Trailing::Remove).leading(info.leading());
                if entry.paths.iter().all(|path| view.status.missing.contains(path)) {
                    row = row.subtitle(text.file_missing, palette.orange);
                } else if entry.paths.len() > 1 {
                    row = row.subtitle(&copies, palette.secondary);
                }
                if group.row(row).trailing_clicked {
                    remove = Some(index);
                }
            }
            open_add = group.row(Row::new("add", text.add_app, Trailing::None).action(palette.accent)).clicked;
        });
        if let Some(index) = remove {
            config.remove_apps(&entries[index].paths);
        }
        if open_add {
            self.open_add_apps();
        }
    }
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

/// The state at a glance: a tinted card with the shield and why, then the network, VPN and server underneath,
/// like the panel Kordon opens from its tray icon.
fn status_card(ui: &mut Ui, view: View<'_>, config: &Config, hero: Hero) -> Option<HeroAction> {
    let palette = &view.palette;
    let animated = |ui: &Ui, channel: &str, value: u8| {
        ui.ctx().animate_value_with_time(egui::Id::unique(("hero", channel)), f32::from(value), COLOR_ANIMATION) as u8
    };
    let color = Color32::from_rgb(
        animated(ui, "r", hero.color.r()),
        animated(ui, "g", hero.color.g()),
        animated(ui, "b", hero.color.b()),
    );
    let mut clicked = false;
    // A plain surface: the shield is the one colored thing on the screen.
    Frame::new()
        .fill(palette.card)
        .corner_radius(CornerRadius::same(HERO_RADIUS))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(SHIELD_SIZE), Sense::hover());
                widgets::shield(ui, rect, color, hero.glyph);
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(hero.title).font(theme::title_small()).color(palette.text));
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(&hero.subtitle).font(theme::callout()).color(palette.secondary)).wrap());
                });
            });
            if let Some(action) = &hero.action {
                let label = match action {
                    HeroAction::Trust(_) => view.text.trust_network,
                    HeroAction::Allow(_) => view.text.allow_server,
                };
                ui.add_space(12.0);
                let label = RichText::new(label).font(theme::headline()).color(Color32::WHITE);
                let button = egui::Button::new(label).fill(palette.accent).corner_radius(CornerRadius::same(10));
                clicked = ui.add_sized(vec2(ui.available_width(), 36.0), button).clicked();
            }
            ui.add_space(16.0);
            let pixel = 1.0 / ui.pixels_per_point();
            let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), pixel), Sense::hover());
            ui.painter().rect_filled(line, CornerRadius::ZERO, palette.separator);
            ui.add_space(12.0);
            status_strip(ui, view, config);
        });
    if clicked { hero.action } else { None }
}

fn status_strip(ui: &mut Ui, view: View<'_>, config: &Config) {
    let (palette, text, snapshot) = (&view.palette, view.text, &view.status.snapshot);
    let network = match snapshot.networks.first() {
        Some(network) if config.trusted.iter().any(|entry| entry.id == network.id) => {
            (network.name.clone(), text.network_trusted, palette.green)
        }
        Some(network) => (network.name.clone(), text.network_untrusted, palette.orange),
        None => (text.no_connection.to_owned(), "", palette.red),
    };
    let vpn = if config.tun_enabled && rules::tun_active(config, snapshot) {
        ("TUN".to_owned(), palette.green)
    } else if config.proxy_enabled && snapshot.proxy.active() {
        (text.vpn_proxy.to_owned(), palette.green)
    } else {
        (text.off.to_owned(), palette.red)
    };
    // The address and its country, as Kordon shows them: asked only through the VPN, so there is none without it.
    let country = snapshot.exit_country.as_ref().map_or("", |country| (text.country_name)(country));
    let server = match snapshot.exit_ip {
        _ if !config.server_check && !config.ip_info => (text.ip_unchecked.to_owned(), palette.tertiary),
        None if rules::vpn_route(config, snapshot).is_none() => ("—".to_owned(), palette.tertiary),
        None => ("…".to_owned(), palette.secondary),
        Some(address) => {
            let allowed = !config.server_check || config.allowed_servers.contains(&address);
            let label = config.server_name(address).map_or_else(|| address.to_string(), str::to_owned);
            (label, if allowed { palette.green } else { palette.orange })
        }
    };
    let cells = [
        (text.status_network, network.0, network.1, network.2),
        (text.status_vpn, vpn.0, "", vpn.1),
        (text.status_ip, server.0, country, server.1),
    ];
    ui.columns(cells.len(), |columns| {
        for (column, (caption, value, note, color)) in columns.iter_mut().zip(cells) {
            column.label(RichText::new(caption).font(theme::footnote()).color(palette.secondary));
            column.add_space(4.0);
            // The lamp is the only colored mark; the words stay in the text colors.
            column.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.0, color);
                ui.add_space(8.0);
                ui.add(egui::Label::new(RichText::new(value).font(theme::callout()).color(palette.text)).truncate());
            });
            if !note.is_empty() {
                column.label(RichText::new(note).font(theme::footnote()).color(palette.secondary));
            }
        }
    });
}

fn hero_state(view: View<'_>, config: &Config) -> Hero {
    let (palette, text, status) = (&view.palette, view.text, view.status);
    let untrusted = status
        .snapshot
        .networks
        .iter()
        .find(|network| !config.trusted.iter().any(|entry| entry.id == network.id))
        .map(|network| HeroAction::Trust(TrustedNetwork { id: network.id.clone(), name: network.name.clone() }));
    let foreign_server = status.reasons.iter().find_map(|reason| match reason {
        Reason::ForeignServer(address) => Some(HeroAction::Allow(*address)),
        _ => None,
    });

    if let Some(error) = &status.firewall_error {
        let subtitle = if error.contains("0x00000005") { text.needs_admin.to_owned() } else { (text.rules_failed)(error) };
        return Hero { color: palette.orange, glyph: Glyph::Exclamation, title: text.failed_title, subtitle, action: None };
    }
    if status.blocked {
        let subtitle = status.reasons.iter().map(|reason| text.explain(reason)).collect::<Vec<_>>().join(" ");
        return Hero { color: palette.red, glyph: Glyph::Cross, title: text.blocked_title, subtitle, action: untrusted.or(foreign_server) };
    }
    if status.enabled {
        return Hero {
            color: palette.green,
            glyph: Glyph::Check,
            title: text.protected_title,
            subtitle: text.protected_subtitle.to_owned(),
            action: None,
        };
    }
    Hero { color: palette.tertiary, glyph: Glyph::Dash, title: text.off_title, subtitle: text.off_subtitle.to_owned(), action: None }
}
