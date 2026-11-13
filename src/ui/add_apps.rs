use super::catalog::AppInfo;
use super::widgets::{self, Row, Trailing, section};
use super::{App, Page, View};
use crate::config::{self, Config};
use eframe::egui::{self, Ui};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Every running copy of one program, listed once under its display name.
struct Candidate {
    name: String,
    info: Rc<AppInfo>,
    paths: Vec<PathBuf>,
    windowed: bool,
}

impl App {
    pub(super) fn add_apps_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let nav = widgets::nav_bar(ui, palette, text.apps, text.back, Some(text.done));
        if nav.back || nav.done {
            self.page = Page::Home;
            self.notice = None;
        }
        ui.add_space(8.0);
        widgets::text_field(ui, palette, &mut self.search, text.search);
        self.show_notice(ui, view);

        let ctx = ui.ctx().clone();
        let query = self.search.to_lowercase();
        let mut candidates: Vec<Candidate> = Vec::new();
        for app in &self.running {
            let info = self.catalog.get(&ctx, &app.path);
            match candidates.iter_mut().find(|candidate| config::same_name(&candidate.name, &info.name)) {
                Some(candidate) => {
                    candidate.paths.push(app.path.clone());
                    candidate.windowed |= app.windowed;
                }
                None => candidates.push(Candidate {
                    name: info.name.clone(),
                    info,
                    paths: vec![app.path.clone()],
                    windowed: app.windowed,
                }),
            }
        }
        // A protected program now also running from another folder, typically after an update, is the same
        // program: cover the new copy too, so the check mark never claims more than is blocked.
        for candidate in &candidates {
            if !config.paths_named(&candidate.name).is_empty() {
                let uncovered: Vec<PathBuf> = candidate.paths.iter().filter(|path| !config.apps.contains(path)).cloned().collect();
                for path in uncovered {
                    config.add_app(path, &candidate.name);
                }
            }
        }
        candidates.retain(|candidate| {
            candidate.name.to_lowercase().contains(&query)
                || candidate.paths.iter().any(|path| file_name(path).to_lowercase().contains(&query))
        });
        candidates.sort_by_key(|candidate| candidate.name.to_lowercase());

        let mut toggled = None;
        let mut browse = false;
        egui::ScrollArea::vertical().id_salt("add").auto_shrink(false).show(ui, |ui| {
            section(ui, palette, None, None, |group| {
                browse = group.row(Row::new("browse", text.choose_file, Trailing::None).action(palette.accent)).clicked;
            });
            for (title, windowed) in [(text.open_apps, true), (text.background_apps, false)] {
                let group_apps: Vec<_> = candidates.iter().filter(|candidate| candidate.windowed == windowed).collect();
                if group_apps.is_empty() {
                    continue;
                }
                section(ui, palette, Some(title), None, |group| {
                    for candidate in group_apps {
                        let selected = is_protected(config, candidate);
                        let row = Row::new(&candidate.name, &candidate.name, Trailing::Check(selected)).leading(candidate.info.leading());
                        if group.row(row).clicked {
                            toggled = Some((candidate, selected));
                        }
                    }
                });
            }
            if candidates.is_empty() {
                widgets::footnote(ui, text.nothing_found, palette.secondary);
            }
            ui.add_space(20.0);
        });

        if let Some((candidate, selected)) = toggled {
            if selected {
                let mut paths = config.paths_named(&candidate.name);
                paths.extend(candidate.paths.iter().cloned());
                config.remove_apps(&paths);
            } else {
                for path in &candidate.paths {
                    self.add_app(config, path.clone(), text);
                }
            }
        }
        if browse && let Some(path) = rfd::FileDialog::new().add_filter(text.executables, &["exe"]).pick_file() {
            self.add_app(config, path, text);
        }
    }
}

fn is_protected(config: &Config, candidate: &Candidate) -> bool {
    candidate.paths.iter().all(|path| config.apps.contains(path))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}
