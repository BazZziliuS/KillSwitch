use super::theme;
use super::widgets::{self, Row, Trailing, section};
use super::{App, Page, View};
use crate::config::Config;
use crate::i18n::Language;
use crate::platform::autostart;
use eframe::egui::{self, RichText, Sense, Ui};

impl App {
    pub(super) fn settings_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        widgets::page_title(ui, palette, text.settings);
        self.show_notice(ui, view);
        egui::ScrollArea::vertical().id_salt("settings").auto_shrink(false).show(ui, |ui| {
            self.general_section(ui, view, config);

            let language = config.language.map_or(text.system_language, Language::native_name);
            section(ui, palette, None, None, |group| {
                let trailing = Trailing::Chevron { value: Some((language, palette.secondary)) };
                if group.row(Row::new("language", text.language, trailing)).clicked {
                    self.page = Page::Language;
                }
            });

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
            ui.add_space(widgets::TAB_BAR_CLEARANCE);
        });
    }

    fn general_section(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        let mut toggle_autostart = false;
        section(ui, palette, Some(text.general), None, |group| {
            let switch = |on| Trailing::Switch { on, enabled: true };
            toggle_autostart = group.row(Row::new("autostart", text.autostart, switch(config.autostart))).trailing_clicked;
            if group.row(Row::new("notifications", text.notifications, switch(config.notifications))).trailing_clicked {
                config.notifications = !config.notifications;
            }
            if group.row(Row::new("overlay", text.overlay, switch(config.overlay))).trailing_clicked {
                config.overlay = !config.overlay;
            }
            if group.row(Row::new("update-check", text.update_check, switch(config.update_check))).trailing_clicked {
                config.update_check = !config.update_check;
            }
        });
        if toggle_autostart {
            match autostart::set(!config.autostart) {
                Ok(()) => config.autostart = !config.autostart,
                Err(error) => self.notice = Some((text.autostart_failed)(&error)),
            }
        }
    }
}
