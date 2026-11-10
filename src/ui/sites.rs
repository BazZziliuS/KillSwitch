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
    pub(super) fn sites_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        widgets::page_title(ui, palette, text.sites);
        self.show_notice(ui, view);
        egui::ScrollArea::vertical().id_salt("sites").auto_shrink(false).show(ui, |ui| {
            let mut remove = None;
            let (mut add, mut filters) = (false, false);
            let filter = &self.host_filter;
            section(ui, palette, None, Some(text.sites_footer), |group| {
                for (index, site) in config.sites.iter().enumerate() {
                    let is_pattern = config::is_pattern(site);
                    let learned = view.status.pattern_addresses.get(site).copied().unwrap_or(0);
                    let pattern_label =
                        if view.status.pattern_watch_failed { text.pattern_unavailable.to_owned() } else { (text.site_pattern)(learned) };
                    let mut row = Row::new(site, site, Trailing::Remove);
                    if is_pattern {
                        row = row.subtitle(&pattern_label, palette.secondary);
                    } else if filter.excludes(site) {
                        row = row.subtitle(text.site_filtered, palette.secondary);
                    } else if view.status.unresolved_sites.contains(site) {
                        row = row.subtitle(text.site_unresolved, palette.orange);
                    }
                    if group.row(row).trailing_clicked {
                        remove = Some(index);
                    }
                }
                add = group.row(Row::new("add-site", text.add_site, Trailing::None).action(palette.accent)).clicked;
            });
            section(ui, palette, None, None, |group| {
                let count = config.site_filters.len().to_string();
                let trailing = Trailing::Chevron { value: Some((&count, palette.secondary)) };
                filters = group.row(Row::new("site-filters", text.site_filters, trailing)).clicked;
            });
            if let Some(index) = remove {
                config.sites.remove(index);
            }
            if add {
                self.open_add_site();
            } else if filters {
                self.open_site_filters(Page::Sites);
            }
            ui.add_space(widgets::TAB_BAR_CLEARANCE);
        });
    }

    pub(super) fn open_add_site(&mut self) {
        self.search.clear();
        self.notice = None;
        self.page = Page::AddSite;
    }

    pub(super) fn add_site_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let nav = widgets::nav_bar(ui, palette, text.add_site, text.back, Some(text.done));
        if nav.back {
            self.page = Page::Sites;
            self.notice = None;
        }
        ui.add_space(6.0);
        let field = widgets::text_field(ui, palette, &mut self.search, "example.com, *inst*");
        let submitted = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        widgets::footnote(ui, text.add_site_footer, palette.secondary);
        self.show_notice(ui, view);

        if nav.done || submitted {
            if config::is_pattern(&self.search) {
                match config::normalize_pattern(&self.search) {
                    Some(pattern) => {
                        if !config.sites.contains(&pattern) {
                            config.sites.push(pattern);
                        }
                        self.notice = None;
                        self.page = Page::Sites;
                    }
                    None => self.notice = Some(text.pattern_invalid.to_owned()),
                }
            } else {
                match config::normalize_site(&self.search) {
                    Some(site) => self.add_site(view, config, site),
                    None => self.notice = Some(text.site_invalid.to_owned()),
                }
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
            self.page = Page::Sites;
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
            self.page = Page::Sites;
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
        let filter = &self.host_filter;
        let mut toggled = None;
        let mut open_filters = false;
        egui::ScrollArea::vertical().id_salt("scan").auto_shrink(false).show(ui, |ui| {
            let count = config.site_filters.len().to_string();
            section(ui, palette, None, None, |group| {
                let trailing = Trailing::Chevron { value: Some((&count, palette.secondary)) };
                open_filters = group.row(Row::new("site-filters", text.site_filters, trailing)).clicked;
            });
            let Some(found) = &scan.found else {
                widgets::footnote(ui, text.scan_running, palette.secondary);
                return;
            };
            let visible = |hosts: &'_ [String]| hosts.iter().filter(|host| !filter.excludes(host)).cloned().collect::<Vec<_>>();
            let (subdomains, services) = (visible(&found.subdomains), visible(&found.services));
            if subdomains.is_empty() && services.is_empty() {
                widgets::footnote(ui, text.scan_nothing, palette.secondary);
            }
            for (title, footer, hosts) in
                [(text.scan_subdomains, None, &subdomains), (text.scan_services, Some(text.scan_services_footer), &services)]
            {
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
        });
        if let Some(host) = toggled
            && !scan.selected.remove(&host)
        {
            scan.selected.insert(host);
        }
        if nav.done {
            config.sites.extend(std::mem::take(&mut scan.selected).into_iter().filter(|host| !filter.excludes(host)));
        }
        if nav.back || nav.done {
            self.scan = None;
            self.page = Page::Sites;
        } else if open_filters {
            self.open_site_filters(Page::SiteScan);
        }
    }

    pub(super) fn open_site_filters(&mut self, back: Page) {
        self.search.clear();
        self.notice = None;
        self.filters_back = back;
        self.page = Page::SiteFilters;
    }

    pub(super) fn site_filters_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let nav = widgets::nav_bar(ui, palette, text.site_filters, text.back, Some(text.done));
        ui.add_space(6.0);
        let field = widgets::text_field(ui, palette, &mut self.search, r"ads|^cdn\.");
        let submitted = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        widgets::footnote(ui, text.site_filters_footer, palette.secondary);

        let pattern = self.search.trim().to_owned();
        let mut leave = nav.back;
        if (nav.done || submitted) && !pattern.is_empty() {
            if config::is_valid_filter(&pattern) {
                if !config.site_filters.contains(&pattern) {
                    config.site_filters.push(pattern);
                }
                self.search.clear();
                self.notice = None;
                field.request_focus();
                leave = nav.done;
            } else {
                self.notice = Some(text.site_filter_invalid.to_owned());
            }
        } else if nav.done {
            leave = true;
        }
        self.show_notice(ui, view);

        let mut remove = None;
        egui::ScrollArea::vertical().id_salt("filters").auto_shrink(false).show(ui, |ui| {
            if config.site_filters.is_empty() {
                return;
            }
            section(ui, palette, None, None, |group| {
                for (index, pattern) in config.site_filters.iter().enumerate() {
                    if group.row(Row::new(pattern, pattern, Trailing::Remove)).trailing_clicked {
                        remove = Some(index);
                    }
                }
            });
        });
        if let Some(index) = remove {
            config.site_filters.remove(index);
        }
        if leave {
            self.notice = None;
            self.page = self.filters_back;
        }
    }
}
