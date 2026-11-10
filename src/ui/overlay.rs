//! A small notice over a protected app while it is offline, so the missing internet isn't mistaken for a fault in the app.
//!
//! It is a click-through Win32 window on its own thread, because egui stops updating while the main window is hidden.
//! The picture is composed pixel by pixel: GDI alone can't draw smooth corners, a soft shadow or real transparency.

use super::theme::{self, Palette};
use super::widgets;
use crate::i18n;
use crate::protection::monitor::Shared;
use eframe::egui::{Color32, Theme};
use std::path::Path;
use std::sync::Arc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PWSTR, w};

const TICK_MS: u32 = 250;
const BASE_DPI: i32 = 96;
const CARD_OPACITY: f32 = 0.94;
const SHADOW_OPACITY: f32 = 0.28;
const BORDER_OPACITY: f32 = 0.12;
const TITLE_SIZE: i32 = 13;
const REASON_SIZE: i32 = 12;
const MAX_TEXT_WIDTH: i32 = 300;
const MIN_TEXT_WIDTH: i32 = 120;
/// Room around the card for its shadow.
const MARGIN: i32 = 14;
const PADDING_X: i32 = 12;
const PADDING_Y: i32 = 9;
const ICON_SIZE: i32 = 26;
const ICON_GAP: i32 = 10;
const LINE_GAP: i32 = 1;
const TOP_OFFSET: i32 = 6;
const RADIUS: f32 = 14.0;
const SHADOW_DROP: f32 = 2.0;
const FORMAT: DRAW_TEXT_FORMAT = DRAW_TEXT_FORMAT(DT_LEFT.0 | DT_WORDBREAK.0 | DT_NOPREFIX.0);
/// The text is drawn into a mask, one color channel per line, so each line can be tinted afterwards.
const TITLE_MASK: COLORREF = COLORREF(0x0000_00FF);
const REASON_MASK: COLORREF = COLORREF(0x0000_FF00);

fn dp(value: i32, dpi: i32) -> i32 {
    value * dpi / BASE_DPI
}

fn is_dark_mode() -> bool {
    let mut light = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut light).cast()),
            Some(&mut size),
        )
    };
    status != ERROR_SUCCESS || light == 0
}

fn channels(color: Color32) -> [f32; 3] {
    [f32::from(color.r()) / 255.0, f32::from(color.g()) / 255.0, f32::from(color.b()) / 255.0]
}

/// Everything `compose` needs. Sizes are in physical pixels.
struct Scene<'a> {
    width: usize,
    height: usize,
    scale: f32,
    margin: f32,
    palette: Palette,
    dark: bool,
    /// BGRA pixels of the text mask, as GDI left them.
    mask: &'a [u8],
    /// Unmultiplied RGBA pixels of the square icon.
    icon: &'a [u8],
    icon_size: usize,
    icon_x: usize,
    icon_y: usize,
}

/// Signed distance to the rounded card: negative inside, positive outside.
fn card_distance(x: f32, y: f32, scene: &Scene<'_>) -> f32 {
    let radius = RADIUS * scene.scale;
    let half_width = scene.width as f32 / 2.0 - scene.margin;
    let half_height = scene.height as f32 / 2.0 - scene.margin;
    let qx = (x - scene.width as f32 / 2.0).abs() - (half_width - radius);
    let qy = (y - scene.height as f32 / 2.0).abs() - (half_height - radius);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

fn over(pixel: &mut [f32; 4], color: [f32; 3], alpha: f32) {
    for channel in 0..3 {
        pixel[channel] = color[channel] * alpha + pixel[channel] * (1.0 - alpha);
    }
    pixel[3] = alpha + pixel[3] * (1.0 - alpha);
}

fn cross_coverage(dx: f32, dy: f32, arm: f32, half_stroke: f32) -> f32 {
    let (x, y) = (dx.abs(), dy.abs());
    let along = ((x + y) / (2.0 * arm)).clamp(0.0, 1.0);
    (half_stroke + 0.5 - (x - along * arm).hypot(y - along * arm)).clamp(0.0, 1.0)
}

/// Produces premultiplied BGRA pixels, the format layered windows require.
fn compose(scene: &Scene<'_>) -> Vec<u8> {
    let (card, text, secondary) = (channels(scene.palette.card), channels(scene.palette.text), channels(scene.palette.secondary));
    let border = if scene.dark { [1.0; 3] } else { [0.0; 3] };
    let mut output = Vec::with_capacity(scene.width * scene.height * 4);
    for row in 0..scene.height {
        for column in 0..scene.width {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let mut pixel = [0.0f32; 4];

            let shadow_distance = card_distance(x, y - SHADOW_DROP * scene.scale, scene);
            if shadow_distance > -1.0 {
                let falloff = (1.0 - shadow_distance.max(0.0) / scene.margin).clamp(0.0, 1.0);
                over(&mut pixel, [0.0; 3], SHADOW_OPACITY * falloff * falloff);
            }
            let distance = card_distance(x, y, scene);
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            over(&mut pixel, card, coverage * CARD_OPACITY);
            let edge = (1.0 - (distance + 0.5 * scene.scale).abs() / (0.5 * scene.scale)).clamp(0.0, 1.0);
            over(&mut pixel, border, edge * coverage * BORDER_OPACITY);

            if column >= scene.icon_x && row >= scene.icon_y && column < scene.icon_x + scene.icon_size && row < scene.icon_y + scene.icon_size {
                let index = ((row - scene.icon_y) * scene.icon_size + column - scene.icon_x) * 4;
                let alpha = f32::from(scene.icon[index + 3]) / 255.0;
                over(&mut pixel, channels(scene.palette.red), alpha);
                let center = scene.icon_size as f32 / 2.0;
                let (dx, dy) = (x - scene.icon_x as f32 - center, y - scene.icon_y as f32 - center);
                let mark = cross_coverage(dx, dy, scene.icon_size as f32 * 0.14, 1.1 * scene.scale);
                over(&mut pixel, [1.0; 3], mark * alpha);
            }

            let index = (row * scene.width + column) * 4;
            let (title, reason) = (scene.mask[index + 2], scene.mask[index + 1]);
            over(&mut pixel, text, f32::from(title) / 255.0);
            over(&mut pixel, secondary, f32::from(reason) / 255.0);

            let to_byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            output.extend_from_slice(&[to_byte(pixel[2]), to_byte(pixel[1]), to_byte(pixel[0]), to_byte(pixel[3])]);
        }
    }
    output
}

fn font(dpi: i32, size: i32, weight: FONT_WEIGHT) -> HFONT {
    unsafe {
        CreateFontW(
            -dp(size, dpi),
            0,
            0,
            0,
            weight.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            // Grayscale smoothing: ClearType would bleed one line's mask channel into the other.
            ANTIALIASED_QUALITY,
            0,
            w!("Segoe UI"),
        )
    }
}

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

unsafe fn draw_text(dc: HDC, text: &str, font: HFONT, area: &mut RECT, flags: DRAW_TEXT_FORMAT) {
    unsafe {
        let previous = SelectObject(dc, font.into());
        DrawTextW(dc, &mut utf16(text), area, flags);
        SelectObject(dc, previous);
    }
}

/// What is on screen now; the picture is only rebuilt when this changes.
#[derive(Clone, PartialEq)]
struct Content {
    title: String,
    reason: String,
    dark: bool,
    dpi: i32,
    app_width: i32,
}

unsafe fn render(hwnd: HWND, content: &Content) -> Option<(i32, i32)> {
    let dpi = content.dpi;
    let title_font = font(dpi, TITLE_SIZE, FW_SEMIBOLD);
    let reason_font = font(dpi, REASON_SIZE, FW_NORMAL);
    let (margin, padding_x, padding_y, icon, gap, line_gap) =
        (dp(MARGIN, dpi), dp(PADDING_X, dpi), dp(PADDING_Y, dpi), dp(ICON_SIZE, dpi), dp(ICON_GAP, dpi), dp(LINE_GAP, dpi));
    let chrome = 2 * margin + 2 * padding_x + icon + gap;
    let limit = dp(MAX_TEXT_WIDTH, dpi).min(content.app_width - chrome).max(dp(MIN_TEXT_WIDTH, dpi));

    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let mut title_area = RECT { right: limit, ..Default::default() };
        let mut reason_area = RECT { right: limit, ..Default::default() };
        draw_text(memory, &content.title, title_font, &mut title_area, FORMAT | DT_CALCRECT);
        draw_text(memory, &content.reason, reason_font, &mut reason_area, FORMAT | DT_CALCRECT);
        let text_width = title_area.right.max(reason_area.right);
        let text_height = title_area.bottom + line_gap + reason_area.bottom;
        let content_height = icon.max(text_height);
        let (width, height) = (chrome + text_width, 2 * margin + 2 * padding_y + content_height);

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let mut result = None;
        if let Ok(bitmap) = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            let previous = SelectObject(memory, bitmap.into());
            SetBkMode(memory, TRANSPARENT);
            let text_x = margin + padding_x + icon + gap;
            let text_y = margin + padding_y + (content_height - text_height) / 2;
            let mut area = RECT { left: text_x, top: text_y, right: text_x + limit, bottom: height };
            SetTextColor(memory, TITLE_MASK);
            draw_text(memory, &content.title, title_font, &mut area, FORMAT);
            area.top += title_area.bottom + line_gap;
            SetTextColor(memory, REASON_MASK);
            draw_text(memory, &content.reason, reason_font, &mut area, FORMAT);
            let _ = GdiFlush();

            let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), (width * height * 4) as usize);
            let theme = if content.dark { Theme::Dark } else { Theme::Light };
            let palette = theme::palette(theme);
            let red = palette.red;
            let icon_pixels = widgets::shield_rgba(icon as u32, [red.r(), red.g(), red.b()]);
            let scene = Scene {
                width: width as usize,
                height: height as usize,
                scale: dpi as f32 / BASE_DPI as f32,
                margin: margin as f32,
                palette,
                dark: content.dark,
                mask: pixels,
                icon: &icon_pixels,
                icon_size: icon as usize,
                icon_x: (margin + padding_x) as usize,
                icon_y: (margin + padding_y + (content_height - icon) / 2) as usize,
            };
            let composed = compose(&scene);
            pixels.copy_from_slice(&composed);

            let size = SIZE { cx: width, cy: height };
            let origin = POINT::default();
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            if UpdateLayeredWindow(hwnd, Some(screen), None, Some(&size), Some(memory), Some(&origin), COLORREF(0), Some(&blend), ULW_ALPHA).is_ok() {
                result = Some((width, height));
            }
            SelectObject(memory, previous);
            let _ = DeleteObject(bitmap.into());
        }
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        let _ = DeleteObject(title_font.into());
        let _ = DeleteObject(reason_font.into());
        result
    }
}

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size);
        let _ = CloseHandle(process);
        result.ok()?;
        Some(String::from_utf16_lossy(&buffer[..size as usize]))
    }
}

fn is_listed(path: &str, apps: &[impl AsRef<Path>]) -> bool {
    let path = path.to_lowercase();
    apps.iter().any(|app| app.as_ref().to_string_lossy().to_lowercase() == path)
}

/// The offline app in front of the user, and the two lines to tell them about it.
fn target(shared: &Shared) -> Option<(RECT, String, String)> {
    let config = shared.config.lock().unwrap().clone();
    if !config.overlay {
        return None;
    }
    let reason = {
        let status = shared.status.lock().unwrap();
        if !status.blocked {
            return None;
        }
        status.reasons.first().cloned()?
    };
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() || IsIconic(window).as_bool() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if !is_listed(&process_path(pid)?, &config.apps) {
            return None;
        }
        let mut rect = RECT::default();
        GetWindowRect(window, &mut rect).ok()?;
        let text = i18n::strings(config.language);
        Some((rect, text.overlay_title.to_owned(), text.explain(&reason)))
    }
}

struct Banner {
    hwnd: HWND,
    shown: Option<(Content, (i32, i32))>,
}

impl Banner {
    fn show(&mut self, app: RECT, title: String, reason: String) {
        let dpi = (unsafe { GetDpiForWindow(self.hwnd) } as i32).max(BASE_DPI);
        let content = Content { title, reason, dark: is_dark_mode(), dpi, app_width: app.right - app.left };
        let size = match &self.shown {
            Some((shown, size)) if *shown == content => Some(*size),
            _ => unsafe { render(self.hwnd, &content) },
        };
        let Some((width, height)) = size else { return };
        self.shown = Some((content, (width, height)));
        let x = app.left + (app.right - app.left - width) / 2;
        let y = app.top + dp(TOP_OFFSET, dpi);
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    }

    fn hide(&mut self) {
        if self.shown.take().is_some() {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

pub fn run(shared: Arc<Shared>) {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let instance = HINSTANCE(module.0);
        let class = w!("KillSwitchOverlay");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance, lpszClassName: class, ..Default::default() });
        let style = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        let Ok(hwnd) = CreateWindowExW(style, class, w!("KillSwitch"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None) else {
            return;
        };
        SetTimer(Some(hwnd), 1, TICK_MS, None);
        let mut banner = Banner { hwnd, shown: None };
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            if message.message == WM_TIMER && message.hwnd == hwnd {
                match target(&shared) {
                    Some((rect, title, reason)) => banner.show(rect, title, reason),
                    None => banner.hide(),
                }
                continue;
            }
            DispatchMessageW(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn composed(mask: &[u8], width: usize, height: usize) -> Vec<u8> {
        compose(&Scene {
            width,
            height,
            scale: 1.0,
            margin: 14.0,
            palette: theme::palette(Theme::Dark),
            dark: true,
            mask,
            icon: &[],
            icon_size: 0,
            icon_x: 0,
            icon_y: 0,
        })
    }

    #[test]
    fn card_is_solid_in_the_middle_and_clear_at_the_corners() {
        let (width, height) = (200, 80);
        let pixels = composed(&vec![0; width * height * 4], width, height);
        assert_eq!(pixels[3], 0);
        let center = (height / 2 * width + width / 2) * 4;
        assert!(pixels[center + 3] >= 235);
    }

    #[test]
    fn pixels_are_premultiplied() {
        let (width, height) = (200, 80);
        let pixels = composed(&vec![255; width * height * 4], width, height);
        assert!(pixels.chunks(4).all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3])));
    }

    #[test]
    fn the_cross_is_centered_and_limited_to_its_arms() {
        assert_eq!(cross_coverage(0.0, 0.0, 4.0, 1.0), 1.0);
        assert_eq!(cross_coverage(4.0, -4.0, 4.0, 1.0), 1.0);
        assert_eq!(cross_coverage(4.0, 0.0, 4.0, 1.0), 0.0);
        assert_eq!(cross_coverage(9.0, 9.0, 4.0, 1.0), 0.0);
    }
}
