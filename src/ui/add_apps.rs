use super::widgets::{self, Row, Trailing, section};
use super::{App, Page, View};
use crate::config::Config;
use eframe::egui::{self, Ui};
use std::path::Path;

impl App {
    pub(super) fn add_apps_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let nav = widgets::nav_bar(ui, palette, text.apps, text.back, Some(text.done));
        if nav.back || nav.done {
            self.page = Page::Home;
            self.notice = None;
        }
        ui.add_space(6.0);
        widgets::text_field(ui, palette, &mut self.search, text.search);
        self.show_notice(ui, view);

        let ctx = ui.ctx().clone();
        let query = self.search.to_lowercase();
        let mut toggled = None;
        let mut browse = false;
        let catalog = &mut self.catalog;
        let running = &self.running;
        egui::ScrollArea::vertical().id_salt("add").auto_shrink(false).show(ui, |ui| {
            section(ui, palette, None, None, |group| {
                browse = group.row(Row::new("browse", text.choose_file, Trailing::None).action(palette.accent)).clicked;
            });
            let mut apps: Vec<_> = running.iter().map(|app| (app, catalog.get(&ctx, &app.path))).collect();
            apps.retain(|(app, info)| {
                info.name.to_lowercase().contains(&query) || file_name(&app.path).to_lowercase().contains(&query)
            });
            apps.sort_by_key(|(_, info)| info.name.to_lowercase());
            for (title, windowed) in [(text.open_apps, true), (text.background_apps, false)] {
                let group_apps: Vec<_> = apps.iter().filter(|(app, _)| app.windowed == windowed).collect();
                if group_apps.is_empty() {
                    continue;
                }
                section(ui, palette, Some(title), None, |group| {
                    for (app, info) in group_apps {
                        let id = app.path.to_string_lossy();
                        let selected = config.apps.contains(&app.path);
                        let row = Row::new(&id, &info.name, Trailing::Check(selected)).leading(info.leading());
                        if group.row(row).clicked {
                            toggled = Some(app.path.clone());
                        }
                    }
                });
            }
            if apps.is_empty() {
                widgets::footnote(ui, text.nothing_found, palette.secondary);
            }
            ui.add_space(20.0);
        });

        if browse && let Some(path) = rfd::FileDialog::new().add_filter(text.executables, &["exe"]).pick_file() {
            self.add_app(config, path, text);
        }
        if let Some(path) = toggled {
            if config.apps.contains(&path) {
                config.apps.retain(|app| *app != path);
            } else {
                self.add_app(config, path, text);
            }
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}
