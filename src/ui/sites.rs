use super::widgets::{self, Row, Trailing, section};
use super::{App, Page, View};
use crate::config::{self, Config};
use crate::platform::site_scan::{self, Found};
use crate::protection::rules;
use eframe::egui::{self, Key, Ui};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

/// A running or finished search for the addresses related to a newly added site.
pub(super) struct Scan {
    result: Arc<Mutex<Option<Found>>>,
    found: Option<Found>,
    selected: BTreeSet<String>,
}

impl App {
    pub(super) fn open_add_site(&mut self) {
        self.search.clear();
        self.notice = None;
        self.page = Page::AddSite;
    }

    pub(super) fn add_site_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let nav = widgets::nav_bar(ui, palette, text.add_site, text.back, Some(text.done));
        if nav.back {
            self.page = Page::Home;
            self.notice = None;
        }
        ui.add_space(6.0);
        let field = widgets::text_field(ui, palette, &mut self.search, "example.com");
        let submitted = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        widgets::footnote(ui, text.add_site_footer, palette.secondary);
        self.show_notice(ui, view);

        if nav.done || submitted {
            match config::normalize_site(&self.search) {
                Some(site) => self.add_site(view, config, site),
                None => self.notice = Some(text.site_invalid.to_owned()),
            }
        }
    }

    fn add_site(&mut self, view: View<'_>, config: &mut Config, site: String) {
        if !config.sites.contains(&site) {
            config.sites.push(site.clone());
        }
        // Without the VPN the search itself would expose the visit the user wants protected.
        let Some(route) = rules::vpn_route(config, &view.status.snapshot) else {
            self.notice = Some(view.text.scan_needs_vpn.to_owned());
            self.page = Page::Home;
            return;
        };
        let result = Arc::new(Mutex::new(None));
        let worker = result.clone();
        std::thread::spawn(move || {
            let found = site_scan::scan(&site, route.proxy());
            *worker.lock().unwrap() = Some(found);
        });
        self.scan = Some(Scan { result, found: None, selected: BTreeSet::new() });
        self.notice = None;
        self.page = Page::SiteScan;
    }

    pub(super) fn site_scan_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let Some(scan) = &mut self.scan else {
            self.page = Page::Home;
            return;
        };
        if scan.found.is_none()
            && let Some(mut found) = scan.result.lock().unwrap().take()
        {
            found.subdomains.retain(|host| !config.sites.contains(host));
            found.services.retain(|host| !config.sites.contains(host));
            scan.selected = found.subdomains.iter().cloned().collect();
            scan.found = Some(found);
        }

        let nav = widgets::nav_bar(ui, palette, text.scan_title, text.back, Some(text.done));
        let mut toggled = None;
        egui::ScrollArea::vertical().id_salt("scan").auto_shrink(false).show(ui, |ui| match &scan.found {
            None => widgets::footnote(ui, text.scan_running, palette.secondary),
            Some(found) if found.subdomains.is_empty() && found.services.is_empty() => {
                widgets::footnote(ui, text.scan_nothing, palette.secondary);
            }
            Some(found) => {
                for (title, footer, hosts) in [
                    (text.scan_subdomains, None, &found.subdomains),
                    (text.scan_services, Some(text.scan_services_footer), &found.services),
                ] {
                    if hosts.is_empty() {
                        continue;
                    }
                    section(ui, palette, Some(title), footer, |group| {
                        for host in hosts {
                            if group.row(Row::new(host, host, Trailing::Check(scan.selected.contains(host)))).clicked {
                                toggled = Some(host.clone());
                            }
                        }
                    });
                }
                ui.add_space(20.0);
            }
        });
        if let Some(host) = toggled
            && !scan.selected.remove(&host)
        {
            scan.selected.insert(host);
        }
        if nav.done {
            config.sites.extend(std::mem::take(&mut scan.selected));
        }
        if nav.back || nav.done {
            self.scan = None;
            self.page = Page::Home;
        }
    }
}
