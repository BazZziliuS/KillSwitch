use super::theme::{self, Palette};
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{self, Color32, CornerRadius, Frame, Id, Pos2, Rect, RichText, Sense, Stroke, TextureId, Ui, Vec2, pos2, vec2};

const CARD_RADIUS: u8 = 12;
const ROW_HEIGHT: f32 = 44.0;
const ROW_HEIGHT_TWO_LINES: f32 = 56.0;
const ROW_PADDING: f32 = 16.0;
const ICON_SIZE: f32 = 28.0;
/// Every leading mark takes the same column, so titles start on one line in every list.
const LEADING_WIDTH: f32 = ICON_SIZE + 12.0;
const SWITCH_SIZE: Vec2 = vec2(44.0, 24.0);
const SWITCH_ANIMATION: f32 = 0.16;
const NAV_HEIGHT: f32 = 44.0;
const TAB_BAR_HEIGHT: f32 = 60.0;
const TAB_BAR_INSET: f32 = 16.0;
const TAB_BAR_GAP: f32 = 12.0;
const TAB_BAR_PADDING: f32 = 4.0;
const TAB_ANIMATION: f32 = 0.2;
const TAB_FADE: f32 = 32.0;
const TAB_FADE_STRENGTH: f32 = 0.94;
/// Empty space at the end of a tab page, so its last row can scroll clear of the floating bar.
pub const TAB_BAR_CLEARANCE: f32 = TAB_BAR_HEIGHT + TAB_BAR_GAP + 16.0;

#[derive(Clone, Copy)]
pub enum Leading {
    None,
    Image(TextureId),
    Letter(char),
    /// A setup step: a filled check once done, an empty ring until then.
    Step(bool),
    /// A small colored mark, as for an event in the history.
    Dot(Color32),
}

#[derive(Clone, Copy)]
pub enum Trailing<'a> {
    None,
    Switch { on: bool, enabled: bool },
    Remove,
    Chevron { value: Option<(&'a str, Color32)> },
    Check(bool),
}

pub struct Row<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub title_color: Option<Color32>,
    pub subtitle: Option<(&'a str, Color32)>,
    pub leading: Leading,
    pub trailing: Trailing<'a>,
    pub action: bool,
}

impl<'a> Row<'a> {
    pub fn new(id: &'a str, title: &'a str, trailing: Trailing<'a>) -> Self {
        Self { id, title, title_color: None, subtitle: None, leading: Leading::None, trailing, action: false }
    }

    pub fn subtitle(mut self, text: &'a str, color: Color32) -> Self {
        self.subtitle = Some((text, color));
        self
    }

    pub fn leading(mut self, leading: Leading) -> Self {
        self.leading = leading;
        self
    }

    /// Makes the whole row respond to clicks, apart from its trailing control.
    pub fn tappable(mut self) -> Self {
        self.action = true;
        self
    }

    /// A row that is a button in its own right, drawn in the accent color.
    pub fn action(mut self, color: Color32) -> Self {
        self.title_color = Some(color);
        self.action = true;
        self
    }
}

#[derive(Default)]
pub struct RowOutput {
    pub clicked: bool,
    pub trailing_clicked: bool,
}

pub struct Group<'u> {
    ui: &'u mut Ui,
    palette: Palette,
    first: bool,
}

impl Group<'_> {
    pub fn row(&mut self, row: Row<'_>) -> RowOutput {
        let palette = self.palette;
        let ui = &mut *self.ui;
        let clickable = row.action || matches!(row.trailing, Trailing::Chevron { .. } | Trailing::Check(_));
        let height = if row.subtitle.is_some() { ROW_HEIGHT_TWO_LINES } else { ROW_HEIGHT };
        let sense = if clickable { Sense::click() } else { Sense::hover() };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
        let painter = ui.painter_at(rect);

        if clickable && response.hovered() {
            painter.rect_filled(rect.shrink(4.0), CornerRadius::same(8), palette.hover);
        }

        let mut left = rect.left() + ROW_PADDING;
        let icon_rect = Rect::from_center_size(pos2(left + ICON_SIZE / 2.0, rect.center().y), Vec2::splat(ICON_SIZE));
        match row.leading {
            Leading::None => {}
            Leading::Image(texture) => {
                let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
                painter.image(texture, icon_rect, uv, Color32::WHITE);
                left += LEADING_WIDTH;
            }
            Leading::Letter(letter) => {
                painter.rect_filled(icon_rect, CornerRadius::same(8), palette.separator);
                painter.text(icon_rect.center(), egui::Align2::CENTER_CENTER, letter, theme::headline(), palette.secondary);
                left += LEADING_WIDTH;
            }
            Leading::Step(done) => {
                let center = icon_rect.center();
                if done {
                    painter.circle_filled(center, 10.0, palette.green);
                    polyline(&painter, &[center + vec2(-4.5, 0.0), center + vec2(-1.5, 3.5), center + vec2(4.5, -3.5)], 2.0, Color32::WHITE);
                } else {
                    painter.circle_stroke(center, 9.5, Stroke::new(1.5, palette.tertiary));
                }
                left += LEADING_WIDTH;
            }
            Leading::Dot(color) => {
                painter.circle_filled(pos2(left + 4.0, rect.center().y), 4.0, color);
                left += 20.0;
            }
        }

        if !self.first {
            // Snapped to a physical pixel: a fractional hairline fades in and out between rows.
            let pixel = 1.0 / ui.pixels_per_point();
            let y = (rect.top() / pixel).round() * pixel + pixel / 2.0;
            painter.hline(left..=rect.right(), y, Stroke::new(pixel, palette.separator));
        }
        self.first = false;

        let trailing_width = paint_width(&row.trailing, ui);
        let text_width = (rect.right() - ROW_PADDING - trailing_width - left - 8.0).max(20.0);
        let title_color = row.title_color.unwrap_or(palette.text);
        let title = truncated(ui, row.title, theme::body(), title_color, text_width);
        match row.subtitle {
            Some((text, color)) => {
                let subtitle = truncated(ui, text, theme::footnote(), color, text_width);
                let top = rect.center().y - (title.size().y + subtitle.size().y + 1.0) / 2.0;
                let subtitle_top = top + title.size().y + 1.0;
                painter.galley(pos2(left, top), title, title_color);
                painter.galley(pos2(left, subtitle_top), subtitle, color);
            }
            None => {
                let top = rect.center().y - title.size().y / 2.0;
                painter.galley(pos2(left, top), title, title_color);
            }
        }

        let right = rect.right() - ROW_PADDING;
        let id = ui.scope_id().with(row.id);
        let mut output = RowOutput { clicked: response.clicked(), trailing_clicked: false };
        match row.trailing {
            Trailing::None => {}
            Trailing::Switch { on, enabled } => {
                let switch = Rect::from_center_size(
                    pos2(right - SWITCH_SIZE.x / 2.0, rect.center().y),
                    SWITCH_SIZE,
                );
                output.trailing_clicked = paint_switch(ui, switch, id, on, enabled, &palette);
            }
            Trailing::Remove => {
                let target = Rect::from_center_size(pos2(right - 10.0, rect.center().y), Vec2::splat(28.0));
                let hit = ui.interact(target, id, Sense::click());
                let color = if hit.hovered() { palette.red } else { palette.tertiary };
                let c = target.center();
                let r = 5.0;
                let stroke = Stroke::new(1.6, color);
                painter.line_segment([c + vec2(-r, -r), c + vec2(r, r)], stroke);
                painter.line_segment([c + vec2(r, -r), c + vec2(-r, r)], stroke);
                output.trailing_clicked = hit.clicked();
            }
            Trailing::Chevron { value } => {
                let c = pos2(right - 4.0, rect.center().y);
                polyline(&painter, &[c + vec2(-3.5, -6.0), c + vec2(2.5, 0.0), c + vec2(-3.5, 6.0)], 1.8, palette.tertiary);
                if let Some((text, color)) = value {
                    painter.text(
                        pos2(right - 18.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        text,
                        theme::callout(),
                        color,
                    );
                }
            }
            Trailing::Check(on) => {
                if on {
                    let c = pos2(right - 8.0, rect.center().y);
                    polyline(&painter, &[c + vec2(-6.5, 0.5), c + vec2(-1.5, 5.5), c + vec2(7.0, -5.5)], 2.2, palette.accent);
                }
            }
        }
        output
    }
}

pub fn text_field(ui: &mut Ui, palette: &Palette, value: &mut String, hint: &str) -> egui::Response {
    Frame::new()
        .fill(palette.card)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text(RichText::new(hint).color(palette.tertiary))
                    .font(theme::body())
                    .text_color(palette.text)
                    .frame(Frame::NONE)
                    .desired_width(f32::INFINITY),
            )
        })
        .inner
}

fn paint_width(trailing: &Trailing<'_>, ui: &Ui) -> f32 {
    match trailing {
        Trailing::None => 0.0,
        Trailing::Switch { .. } => SWITCH_SIZE.x,
        Trailing::Remove | Trailing::Check(_) => 26.0,
        Trailing::Chevron { value: None } => 12.0,
        Trailing::Chevron { value: Some((text, _)) } => {
            18.0 + ui.painter().layout_no_wrap((*text).to_owned(), theme::callout(), Color32::WHITE).size().x
        }
    }
}

fn truncated(ui: &Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(width);
    ui.painter().layout_job(job)
}

fn paint_switch(ui: &mut Ui, rect: Rect, id: Id, on: bool, enabled: bool, palette: &Palette) -> bool {
    let response = ui.interact(rect.expand(4.0), id, if enabled { Sense::click() } else { Sense::hover() });
    let t = ui.ctx().animate_bool_with_time(id, on, SWITCH_ANIMATION);
    let opacity = if enabled { 1.0 } else { 0.45 };
    let track = palette.switch_off.lerp_to_gamma(palette.green, t).gamma_multiply(opacity);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same((rect.height() / 2.0) as u8), track);
    let radius = rect.height() / 2.0 - 2.0;
    let left = rect.left() + rect.height() / 2.0;
    let right = rect.right() - rect.height() / 2.0;
    let knob = pos2(egui::lerp(left..=right, t), rect.center().y);
    painter.circle_filled(knob + vec2(0.0, 1.0), radius + 0.5, Color32::from_black_alpha(40));
    painter.circle_filled(knob, radius, Color32::WHITE.gamma_multiply(if enabled { 1.0 } else { 0.8 }));
    response.clicked()
}

/// Lines with round joins and caps; egui's own strokes end square.
fn polyline(painter: &egui::Painter, points: &[Pos2], width: f32, color: Color32) {
    let stroke = Stroke::new(width, color);
    for pair in points.windows(2) {
        painter.line_segment([pair[0], pair[1]], stroke);
    }
    for point in points {
        painter.circle_filled(*point, width / 2.0, color);
    }
}

pub fn section(ui: &mut Ui, palette: &Palette, header: Option<&str>, footer: Option<&str>, add: impl FnOnce(&mut Group<'_>)) {
    ui.add_space(20.0);
    if let Some(header) = header {
        ui.horizontal(|ui| {
            ui.add_space(ROW_PADDING);
            ui.label(RichText::new(header).font(theme::footnote()).color(palette.secondary));
        });
        ui.add_space(8.0);
    }
    Frame::new().fill(palette.card).corner_radius(CornerRadius::same(CARD_RADIUS)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut group = Group { ui, palette: *palette, first: true };
        add(&mut group);
    });
    if let Some(footer) = footer {
        footnote(ui, footer, palette.secondary);
    }
}

/// A card of large figures with captions beneath, side by side.
pub fn stat_card(ui: &mut Ui, palette: &Palette, header: &str, stats: &[(String, &str, Color32)]) {
    ui.add_space(20.0);
    ui.horizontal(|ui| {
        ui.add_space(ROW_PADDING);
        ui.label(RichText::new(header).font(theme::footnote()).color(palette.secondary));
    });
    ui.add_space(8.0);
    Frame::new().fill(palette.card).corner_radius(CornerRadius::same(CARD_RADIUS)).inner_margin(egui::Margin::symmetric(4, 16)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.columns(stats.len(), |columns| {
            for (column, (value, caption, color)) in columns.iter_mut().zip(stats) {
                column.vertical_centered(|ui| {
                    ui.label(RichText::new(value).font(theme::title_small()).color(*color));
                    ui.add_space(4.0);
                    ui.label(RichText::new(*caption).font(theme::footnote()).color(palette.secondary));
                });
            }
        });
    });
}

pub fn footnote(ui: &mut Ui, text: &str, color: Color32) {
    ui.add_space(8.0);
    Frame::new().inner_margin(egui::Margin::symmetric(ROW_PADDING as i8, 0)).show(ui, |ui| {
        ui.label(RichText::new(text).font(theme::footnote()).color(color));
    });
}

pub struct NavOutput {
    pub back: bool,
    pub done: bool,
}

pub fn nav_bar(ui: &mut Ui, palette: &Palette, title: &str, back_label: &str, done_label: Option<&str>) -> NavOutput {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), NAV_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, title, theme::headline(), palette.text);

    let back_rect = Rect::from_min_size(rect.min, vec2(90.0, NAV_HEIGHT));
    let back = ui.interact(back_rect, ui.scope_id().with("nav-back"), Sense::click());
    let color = if back.hovered() { palette.accent.gamma_multiply(0.7) } else { palette.accent };
    let c = pos2(rect.left() + 8.0, rect.center().y);
    polyline(&painter, &[c + vec2(4.0, -7.0), c + vec2(-3.0, 0.0), c + vec2(4.0, 7.0)], 2.0, color);
    painter.text(pos2(c.x + 12.0, c.y), egui::Align2::LEFT_CENTER, back_label, theme::body(), color);

    let mut done_clicked = false;
    if let Some(label) = done_label {
        let done_rect = Rect::from_min_size(pos2(rect.right() - 80.0, rect.top()), vec2(80.0, NAV_HEIGHT));
        let response = ui.interact(done_rect, ui.scope_id().with("nav-done"), Sense::click());
        let color = if response.hovered() { palette.accent.gamma_multiply(0.7) } else { palette.accent };
        painter.text(pos2(rect.right() - 4.0, c.y), egui::Align2::RIGHT_CENTER, label, theme::headline(), color);
        done_clicked = response.clicked();
    }
    NavOutput { back: back.clicked(), done: done_clicked }
}

pub fn page_title(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        ui.add_space(ROW_PADDING);
        ui.label(RichText::new(title).font(theme::title()).color(palette.text));
    });
}

#[derive(Clone, Copy)]
pub enum TabIcon {
    Shield,
    Globe,
    Lock,
    Clock,
    Gear,
}

/// Line icons, one stroke for every tab, drawn in the label's color.
fn paint_tab_icon(painter: &egui::Painter, icon: TabIcon, center: Pos2, color: Color32, background: Color32) {
    const RADIUS: f32 = 9.0;
    const STROKE: f32 = 1.5;
    let stroke = Stroke::new(STROKE, color);
    match icon {
        TabIcon::Shield => {
            let points = shield_outline().into_iter().map(|(x, y)| center + vec2(x, y) * RADIUS * 1.1).collect();
            painter.add(egui::Shape::closed_line(points, stroke));
        }
        TabIcon::Globe => {
            painter.circle_stroke(center, RADIUS, stroke);
            painter.add(egui::Shape::ellipse_stroke(center, vec2(RADIUS * 0.45, RADIUS), stroke));
            painter.line_segment([center - vec2(RADIUS, 0.0), center + vec2(RADIUS, 0.0)], stroke);
        }
        TabIcon::Lock => {
            let body = Rect::from_min_size(center + vec2(-7.0, -1.0), vec2(14.0, 10.0));
            painter.rect_stroke(body, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
            let shackle: Vec<Pos2> = (0..=16)
                .map(|step| {
                    let angle = std::f32::consts::PI * (1.0 + step as f32 / 16.0);
                    center + vec2(angle.cos() * 4.0, angle.sin() * 4.0 - 5.0)
                })
                .collect();
            polyline(painter, &shackle, STROKE, color);
            polyline(painter, &[center + vec2(-4.0, -5.0), center + vec2(-4.0, -1.0)], STROKE, color);
            polyline(painter, &[center + vec2(4.0, -5.0), center + vec2(4.0, -1.0)], STROKE, color);
        }
        TabIcon::Clock => {
            painter.circle_stroke(center, RADIUS, stroke);
            polyline(painter, &[center + vec2(0.0, -5.0), center, center + vec2(3.5, 2.0)], STROKE, color);
        }
        TabIcon::Gear => {
            // Sliders: reads as settings at this size, where a line-drawn gear turns into a sun.
            for (y, knob) in [(-5.0, 3.0), (0.0, -3.5), (5.0, 1.0)] {
                polyline(painter, &[center + vec2(-RADIUS, y), center + vec2(RADIUS, y)], STROKE, color);
                let knob = center + vec2(knob, y);
                painter.circle_filled(knob, 2.5, background);
                painter.circle_stroke(knob, 2.5, stroke);
            }
        }
    }
}

/// Vertical gradient from clear to `color`.
fn paint_fade(painter: &egui::Painter, rect: Rect, color: Color32) {
    let (clear, solid) = (Color32::TRANSPARENT, color.gamma_multiply(TAB_FADE_STRENGTH));
    let mut mesh = egui::Mesh::default();
    for (point, color) in [(rect.left_top(), clear), (rect.right_top(), clear), (rect.right_bottom(), solid), (rect.left_bottom(), solid)] {
        mesh.colored_vertex(point, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Floating tab bar: a raised surface with a hairline, the selected tab one surface step up. Returns the tab clicked.
pub fn tab_bar<T: Copy + PartialEq>(ui: &mut Ui, palette: &Palette, area: Rect, selected: T, tabs: &[(T, TabIcon, &str)]) -> Option<T> {
    let rect = Rect::from_min_max(
        pos2(area.left() + TAB_BAR_INSET, area.bottom() - TAB_BAR_GAP - TAB_BAR_HEIGHT),
        pos2(area.right() - TAB_BAR_INSET, area.bottom() - TAB_BAR_GAP),
    );
    let painter = ui.painter_at(area);
    let dark = palette.background.r() < 128;
    // The page dissolves under the bar, so rows never end in a hard cut against it.
    paint_fade(&painter, Rect::from_min_max(pos2(area.left(), rect.top() - TAB_FADE), area.max), palette.background);
    let corner = CornerRadius::same((TAB_BAR_HEIGHT / 2.0) as u8);
    if !dark {
        painter.add(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(20) }.as_shape(rect, corner));
    }
    painter.rect_filled(rect, corner, palette.card);
    painter.rect_stroke(rect, corner, Stroke::new(1.0, palette.text.gamma_multiply(0.08)), egui::StrokeKind::Inside);

    let width = (rect.width() - 2.0 * TAB_BAR_PADDING) / tabs.len() as f32;
    let cell_at = |position: f32| {
        Rect::from_min_size(
            pos2(rect.left() + TAB_BAR_PADDING + width * position, rect.top() + TAB_BAR_PADDING),
            vec2(width, rect.height() - 2.0 * TAB_BAR_PADDING),
        )
    };
    let current = tabs.iter().position(|(tab, ..)| *tab == selected).unwrap_or(0) as f32;
    let position = ui.ctx().animate_value_with_time(ui.scope_id().with("tab-highlight"), current, TAB_ANIMATION);
    let lens = cell_at(position);
    // Nested radius: the bar's corner less its padding.
    painter.rect_filled(lens, CornerRadius::same((TAB_BAR_HEIGHT / 2.0 - TAB_BAR_PADDING) as u8), palette.hover);

    let mut clicked = None;
    for (index, (tab, icon, label)) in tabs.iter().enumerate() {
        let cell = cell_at(index as f32);
        let response = ui.interact(cell, ui.scope_id().with(("tab", index)), Sense::click());
        let is_selected = *tab == selected;
        let color = if is_selected { palette.accent } else { palette.secondary };
        let fill = if is_selected { palette.hover } else { palette.card };
        paint_tab_icon(&painter, *icon, pos2(cell.center().x, cell.top() + 18.0), color, fill);
        painter.text(pos2(cell.center().x, cell.top() + 40.0), egui::Align2::CENTER_CENTER, label, theme::footnote(), color);
        if response.clicked() {
            clicked = Some(*tab);
        }
    }
    clicked
}

#[derive(Clone, Copy)]
pub enum Glyph {
    Check,
    Cross,
    Dash,
    Exclamation,
}

/// Unit-square outline of the shield; convex, so it can be filled directly.
fn shield_outline() -> Vec<(f32, f32)> {
    const STEPS: usize = 14;
    let quadratic = |a: (f32, f32), c: (f32, f32), b: (f32, f32), t: f32| {
        let u = 1.0 - t;
        (u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0, u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1)
    };
    let mut right = Vec::new();
    for step in 0..=STEPS {
        right.push(quadratic((0.0, -0.94), (0.42, -0.84), (0.76, -0.62), step as f32 / STEPS as f32));
    }
    for step in 0..=STEPS {
        right.push(quadratic((0.76, 0.0), (0.74, 0.62), (0.0, 0.96), step as f32 / STEPS as f32));
    }
    let left = right.iter().rev().skip(1).take(right.len() - 2).map(|(x, y)| (-x, *y)).collect::<Vec<_>>();
    right.into_iter().chain(left).collect()
}

pub fn shield(ui: &Ui, rect: Rect, color: Color32, glyph: Glyph) {
    let painter = ui.painter();
    let center = rect.center();
    let radius = rect.width() / 2.0;
    let points = shield_outline().into_iter().map(|(x, y)| center + vec2(x, y) * radius).collect();
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));

    let at = |x: f32, y: f32| center + vec2(x, y) * radius;
    let width = radius * 0.13;
    match glyph {
        Glyph::Check => polyline(painter, &[at(-0.32, 0.0), at(-0.08, 0.26), at(0.36, -0.26)], width, Color32::WHITE),
        Glyph::Cross => {
            polyline(painter, &[at(-0.27, -0.27), at(0.27, 0.27)], width, Color32::WHITE);
            polyline(painter, &[at(0.27, -0.27), at(-0.27, 0.27)], width, Color32::WHITE);
        }
        Glyph::Dash => polyline(painter, &[at(-0.28, 0.0), at(0.28, 0.0)], width, Color32::WHITE),
        Glyph::Exclamation => {
            polyline(painter, &[at(0.0, -0.42), at(0.0, 0.1)], width, Color32::WHITE);
            painter.circle_filled(at(0.0, 0.38), width * 0.65, Color32::WHITE);
        }
    }
}

fn inside(outline: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut result = false;
    let mut previous = outline[outline.len() - 1];
    for &point in outline {
        if (point.1 > y) != (previous.1 > y) && x < (previous.0 - point.0) * (y - point.1) / (previous.1 - point.1) + point.0 {
            result = !result;
        }
        previous = point;
    }
    result
}

/// RGBA shield for the tray and the title bar, antialiased by supersampling.
pub fn shield_rgba(size: u32, color: [u8; 3]) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let outline = shield_outline();
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut hits = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / size as f32 * 2.0 - 1.0;
                    let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / size as f32 * 2.0 - 1.0;
                    hits += u32::from(inside(&outline, x, y));
                }
            }
            rgba.extend_from_slice(&[color[0], color[1], color[2], (hits * 255 / (SAMPLES * SAMPLES)) as u8]);
        }
    }
    rgba
}
