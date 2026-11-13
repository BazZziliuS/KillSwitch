//! What protection has done over time: blocks, recoveries and switching it on or off. Kept on disk so the record
//! survives restarts, and shown with a few counts on the History tab.

use super::rules::Reason;
use crate::config::data_path;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Enough for weeks of ordinary use without the file growing without bound.
const MAX_EVENTS: usize = 400;
const DAY: u64 = 24 * 60 * 60;
pub const STATS_DAYS: u64 = 7;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    Enabled,
    Disabled,
    Blocked { reasons: Vec<Reason> },
    /// `seconds` is how long the apps were offline.
    Restored { seconds: u64 },
    Failed { error: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Seconds since the Unix epoch.
    pub at: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Clone, Default)]
pub struct History {
    /// Oldest first.
    pub events: Vec<Event>,
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Stats {
    pub blocks: usize,
    /// Blocks caused by the VPN switching off.
    pub vpn_drops: usize,
    pub offline: Duration,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

impl History {
    pub fn load() -> Self {
        let events = std::fs::read(data_path("history.json"))
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        Self { events }
    }

    fn save(&self) {
        let path = data_path("history.json");
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(data) = serde_json::to_vec(&self.events) {
            let _ = std::fs::write(path, data);
        }
    }

    pub fn record(&mut self, kind: EventKind) {
        self.push(Event { at: now(), kind });
        self.save();
    }

    fn push(&mut self, event: Event) {
        self.events.push(event);
        if self.events.len() > MAX_EVENTS {
            self.events.drain(..self.events.len() - MAX_EVENTS);
        }
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.save();
    }

    /// Counts for the last `STATS_DAYS` days. A block still in progress counts up to `now`.
    pub fn stats(&self, now: u64) -> Stats {
        let since = now.saturating_sub(STATS_DAYS * DAY);
        let mut stats = Stats::default();
        let mut open_block = None;
        for event in &self.events {
            match &event.kind {
                EventKind::Blocked { reasons } => {
                    open_block = Some(event.at);
                    if event.at >= since {
                        stats.blocks += 1;
                        stats.vpn_drops += usize::from(reasons.contains(&Reason::VpnOff));
                    }
                }
                EventKind::Restored { seconds } => {
                    open_block = None;
                    let start = event.at.saturating_sub(*seconds).max(since);
                    stats.offline += Duration::from_secs(event.at.saturating_sub(start));
                }
                _ => {}
            }
        }
        if let Some(start) = open_block {
            stats.offline += Duration::from_secs(now.saturating_sub(start.max(since)));
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::{DAY, Event, EventKind, History, Stats};
    use crate::protection::rules::Reason;
    use std::time::Duration;

    fn history(events: &[(u64, EventKind)]) -> History {
        History { events: events.iter().map(|(at, kind)| Event { at: *at, kind: kind.clone() }).collect() }
    }

    #[test]
    fn counts_recent_blocks_and_offline_time() {
        let now = 30 * DAY;
        let history = history(&[
            (now - 20 * DAY, EventKind::Blocked { reasons: vec![Reason::VpnOff] }),
            (now - 20 * DAY + 50, EventKind::Restored { seconds: 50 }),
            (now - DAY, EventKind::Blocked { reasons: vec![Reason::VpnOff] }),
            (now - DAY + 30, EventKind::Restored { seconds: 30 }),
            (now - 100, EventKind::Blocked { reasons: vec![Reason::NoNetwork] }),
        ]);
        assert_eq!(history.stats(now), Stats { blocks: 2, vpn_drops: 1, offline: Duration::from_secs(130) });
    }

    #[test]
    fn events_round_trip_through_json() {
        let events = history(&[
            (1, EventKind::Blocked { reasons: vec![Reason::ForeignServer("203.0.113.1".parse().unwrap())] }),
            (2, EventKind::Restored { seconds: 1 }),
            (3, EventKind::Failed { error: "x".into() }),
        ])
        .events;
        let json = serde_json::to_string(&events).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Event>>(&json).unwrap(), events);
    }

    #[test]
    fn keeps_only_the_latest_events() {
        let mut history = History::default();
        for at in 0..super::MAX_EVENTS as u64 + 10 {
            history.push(Event { at, kind: EventKind::Enabled });
        }
        assert_eq!(history.events.len(), super::MAX_EVENTS);
        assert_eq!(history.events[0].at, 10);
    }
}
