use super::theme;
use super::widgets;
use super::{App, Page, View};
use crate::config::{self, Config};
use eframe::egui::{self, CornerRadius, Frame, Key, Margin, RichText, Ui};

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
        let field = Frame::new()
            .fill(palette.card)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text(RichText::new("example.com").color(palette.tertiary))
                        .font(theme::body())
                        .text_color(palette.text)
                        .frame(Frame::NONE)
                        .desired_width(f32::INFINITY),
                )
            })
            .inner;
        let submitted = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        widgets::footnote(ui, text.add_site_footer, palette.secondary);
        self.show_notice(ui, view);

        if nav.done || submitted {
            match config::normalize_site(&self.search) {
                Some(site) => {
                    if !config.sites.contains(&site) {
                        config.sites.push(site);
                    }
                    self.notice = None;
                    self.page = Page::Home;
                }
                None => self.notice = Some(text.site_invalid.to_owned()),
            }
        }
    }
}
