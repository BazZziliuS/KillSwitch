mod add_apps;
mod catalog;
mod home;
mod language;
pub mod overlay;
mod sites;
mod theme;
mod tray;
mod vpn;
mod widgets;

use crate::config::{self, Config};
use crate::i18n::{self, Strings};
use crate::platform::apps::{self, RunningApp};
use crate::protection::monitor::{Shared, Status};
use catalog::Catalog;
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Ui, Vec2};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use theme::Palette;

const WINDOW_SIZE: [f32; 2] = [420.0, 720.0];
const WINDOW_ICON_SIZE: u32 = 64;
const WINDOW_ICON_COLOR: [u8; 3] = [0, 122, 255];
const REFRESH: Duration = Duration::from_millis(500);

/// `start_hidden` leaves only the tray icon, for starting at logon.
pub fn run(shared: Arc<Shared>, start_hidden: bool) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("KillSwitch")
            .with_inner_size(WINDOW_SIZE)
            .with_resizable(false)
            .with_maximize_button(false)
            .with_icon(egui::IconData {
                rgba: widgets::shield_rgba(WINDOW_ICON_SIZE, WINDOW_ICON_COLOR),
                width: WINDOW_ICON_SIZE,
                height: WINDOW_ICON_SIZE,
            }),
        ..Default::default()
    };
    eframe::run_native("KillSwitch", options, Box::new(move |cc| Ok(Box::new(App::new(cc, shared, start_hidden)))))
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Home,
    Vpn,
    AddApps,
    AddSite,
    SiteScan,
    ServerName,
    Language,
}

/// What every page needs to draw itself for the current frame.
#[derive(Clone, Copy)]
struct View<'a> {
    palette: Palette,
    text: &'static Strings,
    status: &'a Status,
}

struct App {
    shared: Arc<Shared>,
    page: Page,
    running: Vec<RunningApp>,
    catalog: Catalog,
    search: String,
    notice: Option<String>,
    confirm_quit: bool,
    editing_server: Option<IpAddr>,
    scan: Option<sites::Scan>,
    start_hidden: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, shared: Arc<Shared>, start_hidden: bool) -> Self {
        if let Ok(handle) = cc.window_handle()
            && let RawWindowHandle::Win32(win32) = handle.as_raw()
        {
            shared.hwnd.store(win32.hwnd.get(), Ordering::SeqCst);
        }
        if start_hidden {
            tray::begin_hidden_start(&shared);
        }
        theme::install(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let tray_shared = shared.clone();
        std::thread::spawn(move || tray::run(tray_shared, ctx));
        Self {
            shared,
            page: Page::Home,
            running: Vec::new(),
            catalog: Catalog::default(),
            search: String::new(),
            notice: None,
            confirm_quit: false,
            editing_server: None,
            scan: None,
            start_hidden,
        }
    }

    fn open_add_apps(&mut self) {
        self.running = apps::running().into_iter().filter(|app| !config::is_vpn_client(&app.path)).collect();
        self.search.clear();
        self.notice = None;
        self.page = Page::AddApps;
    }

    fn add_app(&mut self, config: &mut Config, path: PathBuf, text: &Strings) {
        if config::is_vpn_client(&path) {
            self.notice = Some(text.vpn_client_refused.to_owned());
        } else if !config.apps.contains(&path) {
            config.apps.push(path);
            self.notice = None;
        }
    }

    fn show_notice(&self, ui: &mut Ui, view: View<'_>) {
        if let Some(notice) = &self.notice {
            widgets::footnote(ui, notice, view.palette.orange);
        }
    }

    fn quit_alert(&mut self, ctx: &egui::Context, view: View<'_>) {
        if !self.confirm_quit {
            return;
        }
        let (palette, text) = (view.palette, view.text);
        let frame = Frame::new().fill(palette.card).corner_radius(CornerRadius::same(14)).inner_margin(Margin::same(20));
        let response = egui::Modal::new(egui::Id::unique("quit")).frame(frame).show(ctx, |ui| {
            ui.set_width(270.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(text.quit_title).font(theme::headline()).color(palette.text));
                ui.add_space(6.0);
                ui.label(RichText::new(text.quit_body).font(theme::callout()).color(palette.secondary));
                ui.add_space(16.0);
            });
            let mut result = None;
            ui.columns(2, |columns| {
                if alert_button(&mut columns[0], text.cancel, palette.text, &palette) {
                    result = Some(false);
                }
                if alert_button(&mut columns[1], text.quit, palette.red, &palette) {
                    result = Some(true);
                }
            });
            result
        });
        match response.inner {
            Some(true) => self.shared.quit(),
            Some(false) => self.confirm_quit = false,
            None if response.should_close() => self.confirm_quit = false,
            None => {}
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::hide_window(&self.shared);
        }
        ctx.request_repaint_after(REFRESH);
        if self.start_hidden {
            if ctx.cumulative_frame_nr() == 0 {
                ctx.request_repaint();
            } else {
                tray::finish_hidden_start(&self.shared);
                self.start_hidden = false;
            }
        }

        let status = self.shared.status.lock().unwrap().clone();
        let mut config = self.shared.config.lock().unwrap().clone();
        let before = config.clone();
        let view = View { palette: theme::palette(ctx.theme()), text: i18n::strings(config.language), status: &status };

        let page_frame = Frame::new().fill(view.palette.background).inner_margin(Margin::symmetric(18, 6));
        egui::CentralPanel::default().frame(page_frame).show(ui, |ui| match self.page {
            Page::Home => self.home_page(ui, view, &mut config),
            Page::Vpn => self.vpn_page(ui, view, &mut config),
            Page::AddApps => self.add_apps_page(ui, view, &mut config),
            Page::AddSite => self.add_site_page(ui, view, &mut config),
            Page::SiteScan => self.site_scan_page(ui, view, &mut config),
            Page::ServerName => self.server_name_page(ui, view, &mut config),
            Page::Language => self.language_page(ui, view, &mut config),
        });
        self.quit_alert(&ctx, view);

        if config != before
            && let Err(error) = self.shared.update_config(config)
        {
            self.notice = Some((view.text.settings_not_saved)(&error.to_string()));
        }
    }
}

fn alert_button(ui: &mut Ui, text: &str, color: Color32, palette: &Palette) -> bool {
    let label = RichText::new(text).font(theme::headline()).color(color);
    ui.add_sized(
        Vec2::new(ui.available_width(), 36.0),
        egui::Button::new(label).fill(palette.background).corner_radius(CornerRadius::same(9)),
    )
    .clicked()
}
