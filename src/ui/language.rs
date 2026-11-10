use super::widgets::{self, Row, Trailing, section};
use super::{App, Page, View};
use crate::config::Config;
use crate::i18n::Language;
use eframe::egui::Ui;

impl App {
    pub(super) fn language_page(&mut self, ui: &mut Ui, view: View<'_>, config: &mut Config) {
        let (palette, text) = (&view.palette, view.text);
        if widgets::nav_bar(ui, palette, text.language, text.back, None).back {
            self.page = Page::Home;
        }
        section(ui, palette, None, None, |group| {
            if group.row(Row::new("system", text.system_language, Trailing::Check(config.language.is_none()))).clicked {
                config.language = None;
            }
            for language in Language::ALL {
                let row = Row::new(language.native_name(), language.native_name(), Trailing::Check(config.language == Some(language)));
                if group.row(row).clicked {
                    config.language = Some(language);
                }
            }
        });
    }
}
