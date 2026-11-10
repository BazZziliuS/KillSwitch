use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Theme};
use std::sync::Arc;

const SEMIBOLD: &str = "semibold";

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color32,
    pub card: Color32,
    pub text: Color32,
    pub secondary: Color32,
    pub tertiary: Color32,
    pub separator: Color32,
    pub hover: Color32,
    pub switch_off: Color32,
    pub accent: Color32,
    pub green: Color32,
    pub red: Color32,
    pub orange: Color32,
}

pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Light => Palette {
            background: Color32::from_rgb(242, 242, 247),
            card: Color32::WHITE,
            text: Color32::from_rgb(28, 28, 30),
            secondary: Color32::from_rgb(110, 110, 115),
            tertiary: Color32::from_rgb(174, 174, 178),
            separator: Color32::from_rgb(229, 229, 234),
            hover: Color32::from_rgb(238, 238, 243),
            switch_off: Color32::from_rgb(217, 217, 222),
            accent: Color32::from_rgb(0, 122, 255),
            green: Color32::from_rgb(52, 199, 89),
            red: Color32::from_rgb(255, 59, 48),
            orange: Color32::from_rgb(255, 149, 0),
        },
        Theme::Dark => Palette {
            background: Color32::from_rgb(16, 16, 18),
            card: Color32::from_rgb(32, 32, 35),
            text: Color32::from_rgb(245, 245, 247),
            secondary: Color32::from_rgb(152, 152, 159),
            tertiary: Color32::from_rgb(99, 99, 102),
            separator: Color32::from_rgb(56, 56, 58),
            hover: Color32::from_rgb(44, 44, 48),
            switch_off: Color32::from_rgb(72, 72, 76),
            accent: Color32::from_rgb(10, 132, 255),
            green: Color32::from_rgb(48, 209, 88),
            red: Color32::from_rgb(255, 69, 58),
            orange: Color32::from_rgb(255, 159, 10),
        },
    }
}

pub fn body() -> FontId {
    FontId::proportional(15.0)
}

pub fn callout() -> FontId {
    FontId::proportional(14.0)
}

pub fn footnote() -> FontId {
    FontId::proportional(12.5)
}

pub fn tab_label() -> FontId {
    FontId::proportional(11.0)
}

pub fn headline() -> FontId {
    FontId::new(15.0, FontFamily::Name(SEMIBOLD.into()))
}

pub fn title() -> FontId {
    FontId::new(23.0, FontFamily::Name(SEMIBOLD.into()))
}

fn system_font(file: &str) -> Option<Arc<FontData>> {
    let root = std::env::var_os("SystemRoot")?;
    let data = std::fs::read(std::path::Path::new(&root).join("Fonts").join(file)).ok()?;
    Some(Arc::new(FontData::from_owned(data)))
}

/// Uses the Windows system typeface; egui's bundled fonts remain as the fallback.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(font) = system_font("segoeui.ttf") {
        fonts.font_data.insert("system".into(), font);
        proportional.insert(0, "system".into());
    }
    let mut semibold = proportional.clone();
    if let Some(font) = system_font("seguisb.ttf") {
        fonts.font_data.insert(SEMIBOLD.into(), font);
        semibold.insert(0, SEMIBOLD.into());
    }
    fonts.families.insert(FontFamily::Proportional, proportional);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    ctx.set_fonts(fonts);

    for theme in [Theme::Light, Theme::Dark] {
        let colors = palette(theme);
        ctx.style_mut_of(theme, |style| {
            style.visuals.panel_fill = colors.background;
            style.visuals.window_fill = colors.card;
            style.visuals.extreme_bg_color = colors.card;
            style.visuals.selection.bg_fill = colors.accent.gamma_multiply(0.35);
            style.visuals.selection.stroke.color = colors.accent;
            style.visuals.widgets.noninteractive.fg_stroke.color = colors.text;
            style.spacing.item_spacing.y = 0.0;
            style.spacing.scroll = egui::style::ScrollStyle {
                dormant_handle_opacity: 0.0,
                dormant_background_opacity: 0.0,
                active_background_opacity: 0.0,
                interact_background_opacity: 0.0,
                ..egui::style::ScrollStyle::floating()
            };
        });
    }
}
