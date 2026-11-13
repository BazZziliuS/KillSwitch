//! The protection log and its counts, after Kordon's history and statistics screens.

use super::widgets::{self, Leading, Row, Trailing, section};
use super::{App, View};
use crate::protection::history::{self, Event, EventKind};
use eframe::egui::{self, Color32, Ui};
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

/// Older entries stay in the file but aren't drawn, so the page stays light.
const SHOWN_EVENTS: usize = 150;
/// 100 ns intervals between 1601-01-01 and 1970-01-01.
const EPOCH_DIFFERENCE: u64 = 116_444_736_000_000_000;

impl App {
    pub(super) fn history_page(&mut self, ui: &mut Ui, view: View<'_>) {
        let (palette, text) = (&view.palette, view.text);
        widgets::page_title(ui, palette, text.history);
        let history = self.shared.history.lock().unwrap().clone();
        let now = history::now();
        egui::ScrollArea::vertical().id_salt("history").auto_shrink(false).show(ui, |ui| {
            let stats = history.stats(now);
            let blocks_color = if stats.blocks == 0 { palette.green } else { palette.text };
            widgets::stat_card(
                ui,
                palette,
                text.history_stats,
                &[
                    (stats.blocks.to_string(), text.stat_blocks, blocks_color),
                    (stats.vpn_drops.to_string(), text.stat_vpn_drops, palette.text),
                    ((text.duration)(stats.offline.as_secs()), text.stat_offline, palette.text),
                ],
            );

            if history.events.is_empty() {
                widgets::footnote(ui, text.history_empty, palette.secondary);
            }
            let today = local_time(now).map(|time| date_key(&time));
            let mut day: Vec<&Event> = Vec::new();
            let mut day_key = None;
            for event in history.events.iter().rev().take(SHOWN_EVENTS) {
                let key = local_time(event.at).map(|time| date_key(&time));
                if key != day_key && !day.is_empty() {
                    day_section(ui, view, day_key, today, &day);
                    day.clear();
                }
                day_key = key;
                day.push(event);
            }
            if !day.is_empty() {
                day_section(ui, view, day_key, today, &day);
            }

            if !history.events.is_empty() {
                let mut clear = false;
                section(ui, palette, None, None, |group| {
                    clear = group.row(Row::new("clear-history", text.history_clear, Trailing::None).action(palette.red)).clicked;
                });
                if clear {
                    self.shared.history.lock().unwrap().clear();
                }
            }
            ui.add_space(widgets::TAB_BAR_CLEARANCE);
        });
    }
}

fn day_section(ui: &mut Ui, view: View<'_>, key: Option<(u16, u16, u16)>, today: Option<(u16, u16, u16)>, events: &[&Event]) {
    let (palette, text) = (&view.palette, view.text);
    let header = match (key, today) {
        (Some(day), Some(today)) if day == today => text.today.to_owned(),
        (Some(day), Some(today)) if is_previous_day(day, today) => text.yesterday.to_owned(),
        (Some((year, month, day)), _) => format!("{day:02}.{month:02}.{year}"),
        (None, _) => String::new(),
    };
    section(ui, palette, Some(&header), None, |group| {
        for (index, event) in events.iter().enumerate() {
            let (title, detail, color) = describe(view, event);
            let clock = local_time(event.at).map_or_else(String::new, |time| format!("{:02}:{:02}", time.wHour, time.wMinute));
            let subtitle = if detail.is_empty() { clock } else { format!("{clock} · {detail}") };
            let id = format!("event-{}-{index}", event.at);
            group.row(Row::new(&id, title, Trailing::None).leading(Leading::Dot(color)).subtitle(&subtitle, palette.secondary));
        }
    });
}

fn describe(view: View<'_>, event: &Event) -> (&'static str, String, Color32) {
    let (palette, text) = (&view.palette, view.text);
    match &event.kind {
        EventKind::Enabled => (text.event_enabled, String::new(), palette.accent),
        EventKind::Disabled => (text.event_disabled, String::new(), palette.tertiary),
        EventKind::Blocked { reasons } => {
            let detail = reasons.iter().map(|reason| text.explain(reason)).collect::<Vec<_>>().join(" ");
            (text.event_blocked, detail, palette.red)
        }
        EventKind::Restored { seconds } => (text.event_restored, (text.event_offline_for)(&(text.duration)(*seconds)), palette.green),
        EventKind::Failed { error } => (text.event_failed, error.clone(), palette.orange),
    }
}

fn date_key(time: &SYSTEMTIME) -> (u16, u16, u16) {
    (time.wYear, time.wMonth, time.wDay)
}

fn is_previous_day(day: (u16, u16, u16), today: (u16, u16, u16)) -> bool {
    let days = |(year, month, day): (u16, u16, u16)| {
        // Days since a fixed origin, from the civil calendar; only differences matter here.
        let (year, month) = if month <= 2 { (i64::from(year) - 1, i64::from(month) + 12) } else { (i64::from(year), i64::from(month)) };
        365 * year + year / 4 - year / 100 + year / 400 + (153 * (month - 3) + 2) / 5 + i64::from(day)
    };
    days(today) - days(day) == 1
}

/// Unix seconds as wall-clock time in the Windows time zone.
fn local_time(unix: u64) -> Option<SYSTEMTIME> {
    let ticks = unix * 10_000_000 + EPOCH_DIFFERENCE;
    let file_time = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&file_time, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(local)
}

#[cfg(test)]
mod tests {
    use super::is_previous_day;

    #[test]
    fn recognizes_yesterday_across_month_and_year_ends() {
        assert!(is_previous_day((2026, 11, 12), (2026, 11, 13)));
        assert!(is_previous_day((2026, 2, 28), (2026, 3, 1)));
        assert!(is_previous_day((2025, 12, 31), (2026, 1, 1)));
        assert!(!is_previous_day((2026, 11, 11), (2026, 11, 13)));
    }
}
