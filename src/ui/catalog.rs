//! Caches each executable's display name and icon texture.

use super::widgets::Leading;
use crate::platform::apps;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct AppInfo {
    pub name: String,
    icon: Option<TextureHandle>,
}

impl AppInfo {
    pub fn leading(&self) -> Leading {
        let initial = self.name.chars().next().and_then(|letter| letter.to_uppercase().next()).unwrap_or('?');
        self.icon.as_ref().map_or(Leading::Letter(initial), |icon| Leading::Image(icon.id()))
    }
}

#[derive(Default)]
pub struct Catalog(HashMap<PathBuf, Rc<AppInfo>>);

impl Catalog {
    pub fn get(&mut self, ctx: &egui::Context, path: &Path) -> Rc<AppInfo> {
        self.0.entry(path.to_path_buf()).or_insert_with(|| Rc::new(load(ctx, path))).clone()
    }
}

fn load(ctx: &egui::Context, path: &Path) -> AppInfo {
    let icon = apps::icon(path).map(|icon| {
        let image = ColorImage::from_rgba_unmultiplied([icon.width, icon.height], &icon.rgba);
        ctx.load_texture(path.display().to_string(), image, TextureOptions::LINEAR)
    });
    AppInfo { name: apps::display_name(path), icon }
}
