//! The tray runs on its own thread with its own message loop: egui stops updating while the window is hidden,
//! and the icon must keep reflecting the protection state.

use super::widgets;
use crate::i18n::{self, Strings};
use crate::protection::monitor::{Shared, Status};
use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;

const ICON_SIZE: u32 = 32;
const LOOP_TIMEOUT_MS: u32 = 200;

#[derive(Clone, Copy, PartialEq)]
enum State {
    Off,
    Protected,
    Blocked,
    Failed,
}

impl State {
    fn of(status: &Status) -> Self {
        if status.firewall_error.is_some() {
            State::Failed
        } else if status.blocked {
            State::Blocked
        } else if status.enabled {
            State::Protected
        } else {
            State::Off
        }
    }

    fn color(self) -> [u8; 3] {
        match self {
            State::Off => [140, 140, 140],
            State::Protected => [52, 199, 89],
            State::Blocked => [255, 59, 48],
            State::Failed => [255, 149, 0],
        }
    }

    fn tooltip(self, strings: &'static Strings) -> &'static str {
        match self {
            State::Off => strings.tray_off,
            State::Protected => strings.tray_protected,
            State::Blocked => strings.tray_blocked,
            State::Failed => strings.tray_failed,
        }
    }
}

fn icon(state: State) -> Icon {
    Icon::from_rgba(widgets::shield_rgba(ICON_SIZE, state.color()), ICON_SIZE, ICON_SIZE)
        .expect("icon buffer matches its dimensions")
}

pub fn show_window(shared: &Shared, ctx: &egui::Context) {
    let hwnd = HWND(shared.hwnd.load(Ordering::SeqCst) as *mut _);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
    }
    ctx.request_repaint();
}

pub fn hide_window(shared: &Shared) {
    unsafe {
        let _ = ShowWindow(HWND(shared.hwnd.load(Ordering::SeqCst) as *mut _), SW_HIDE);
    }
}

fn set_cloaked(shared: &Shared, cloaked: bool) {
    let value = BOOL::from(cloaked);
    unsafe {
        let _ = DwmSetWindowAttribute(
            HWND(shared.hwnd.load(Ordering::SeqCst) as *mut _),
            DWMWA_CLOAK,
            (&raw const value).cast(),
            size_of::<BOOL>() as u32,
        );
    }
}

/// eframe always shows the window after its first frame. Cloaking keeps that from being seen.
pub fn begin_hidden_start(shared: &Shared) {
    set_cloaked(shared, true);
}

pub fn finish_hidden_start(shared: &Shared) {
    hide_window(shared);
    set_cloaked(shared, false);
}

fn build(open: &MenuItem, quit: &MenuItem, state: State, strings: &'static Strings) -> Result<TrayIcon, String> {
    let menu = Menu::new();
    menu.append(open).map_err(|error| error.to_string())?;
    menu.append(quit).map_err(|error| error.to_string())?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(state.tooltip(strings))
        .with_icon(icon(state))
        .build()
        .map_err(|error| error.to_string())
}

pub fn run(shared: Arc<Shared>, ctx: egui::Context) {
    let language = |shared: &Shared| shared.config.lock().unwrap().language;
    let mut chosen = language(&shared);
    let mut strings = i18n::strings(chosen);
    let open = MenuItem::new(strings.tray_open, true, None);
    let quit = MenuItem::new(strings.tray_quit, true, None);
    let mut state = State::Off;
    let mut tooltip = String::new();
    let tray = match build(&open, &quit, state, strings) {
        Ok(tray) => tray,
        Err(error) => {
            eprintln!("tray icon unavailable: {error}");
            return;
        }
    };
    loop {
        unsafe {
            MsgWaitForMultipleObjects(None, false, LOOP_TIMEOUT_MS, QS_ALLINPUT);
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == open.id() {
                show_window(&shared, &ctx);
            } else if event.id == quit.id() {
                shared.quit();
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(&shared, &ctx);
            }
        }
        let current_language = language(&shared);
        let (current, exit) = {
            let status = shared.status.lock().unwrap();
            (State::of(&status), status.snapshot.exit_ip.map(|address| (address, status.snapshot.exit_country.clone())))
        };
        if current_language != chosen {
            chosen = current_language;
            strings = i18n::strings(chosen);
            open.set_text(strings.tray_open);
            quit.set_text(strings.tray_quit);
        }
        if current != state {
            state = current;
            let _ = tray.set_icon(Some(icon(state)));
        }
        // As in Kordon's tray window: the state, then the address and its country.
        let text = match &exit {
            Some((address, Some(country))) => format!("{}\n{address} · {}", state.tooltip(strings), (strings.country_name)(country)),
            Some((address, None)) => format!("{}\n{address}", state.tooltip(strings)),
            None => state.tooltip(strings).to_owned(),
        };
        if text != tooltip {
            let _ = tray.set_tooltip(Some(&text));
            tooltip = text;
        }
    }
}
