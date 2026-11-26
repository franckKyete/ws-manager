use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::db::{now_ms, ActivityDb};
use super::model::{PresenceBucket, PresenceTotals};
use super::ActivityConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Active,
    Idle,
    Away,
    Locked,
    Asleep,
    Unknown,
}

impl PresenceState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Idle => "idle",
            Self::Away => "away",
            Self::Locked => "locked",
            Self::Asleep => "asleep",
            Self::Unknown => "unknown",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "active" => Self::Active,
            "idle" => Self::Idle,
            "away" => Self::Away,
            "locked" => Self::Locked,
            "asleep" => Self::Asleep,
            _ => Self::Unknown,
        }
    }
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Active | Self::Idle)
    }
}

#[derive(Debug, Clone)]
pub enum PresenceEvent {
    /// User input detected / resumed from idle
    Resumed { ts_ms: i64 },
    /// No input for idle_timeout; ts_ms is approximate last-input time
    Idled { ts_ms: i64 },
    /// Screen locked
    Locked { ts_ms: i64 },
    /// Screen unlocked
    Unlocked { ts_ms: i64 },
    /// System going to sleep
    Sleep { ts_ms: i64 },
    /// System woke up
    Wake { ts_ms: i64 },
    /// Manual state set
    Manual { ts_ms: i64, state: PresenceState },
}

pub struct PresenceMonitor {
    current_state: PresenceState,
    current_source: String,
    current_interval_id: Option<i64>,
    last_input_ts: i64,
    last_transition_ts: i64,
    last_rollup: Instant,
    last_logind_poll: Instant,
    last_logind_locked: Option<bool>,
    pub rx: Option<mpsc::Receiver<PresenceEvent>>,
}

impl PresenceMonitor {
    pub fn new(db: &ActivityDb, cfg: &ActivityConfig) -> Self {
        let mut m = Self {
            current_state: PresenceState::Unknown,
            current_source: "init".to_string(),
            current_interval_id: None,
            last_input_ts: now_ms(),
            last_transition_ts: now_ms(),
            last_rollup: Instant::now() - Duration::from_secs(3600),
            last_logind_poll: Instant::now() - Duration::from_secs(3600),
            last_logind_locked: None,
            rx: None,
        };
        m.repair_on_startup(db, cfg);
        m
    }

    pub fn current_state(&self) -> PresenceState {
        self.current_state
    }

    pub fn current_source(&self) -> &str {
        &self.current_source
    }

    pub fn set_manual_state(&mut self, db: &ActivityDb, ts_ms: i64, state: PresenceState) {
        self.transition(db, ts_ms, state, "manual");
        if state == PresenceState::Active {
            self.last_input_ts = ts_ms;
        }
    }

    fn repair_on_startup(&mut self, db: &ActivityDb, cfg: &ActivityConfig) {
        let now = now_ms();
        let away_ms = (cfg.presence.away_timeout.max(60) * 1000) as i64;
        // Close any open intervals at last known tick or now-away_timeout bound
        if let Ok(Some(open)) = db.current_presence_interval() {
            let last_tick = db.get_meta_i64("presence_last_tick_ms").ok().flatten().unwrap_or(open.start_ts);
            let end = (last_tick + away_ms).min(now).max(open.start_ts);
            let _ = db.close_presence_interval(open.id, end);
        }
        // Also close any other dangling open intervals
        let _ = db.close_open_presence_intervals(now.min(now)); // no-op safety; current should be closed
        // Start fresh as unknown; first tick will resolve
        self.current_state = PresenceState::Unknown;
        self.current_interval_id = None;
        self.last_transition_ts = now;
        self.last_input_ts = now;
    }

    pub fn tick(&mut self, db: &ActivityDb, cfg: &ActivityConfig) {
        let now = now_ms();
        let _ = db.set_meta("presence_last_tick_ms", &now.to_string());

        if !cfg.presence.enabled {
            if self.current_state != PresenceState::Unknown {
                self.transition(db, now, PresenceState::Unknown, "disabled");
            }
            return;
        }

        // Drain external events
        let mut events = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(ev) = rx.try_recv() {
                events.push(ev);
            }
        }
        for ev in events {
            self.handle_event(db, cfg, ev);
        }

        // Poll logind periodically
        if self.last_logind_poll.elapsed() >= Duration::from_secs(5) {
            self.poll_logind(db, cfg, now);
            self.last_logind_poll = Instant::now();
        }

        // Time-based transitions
        self.apply_timeouts(db, cfg, now);

        // Ensure we have an interval if state is known
        if self.current_interval_id.is_none() && self.current_state != PresenceState::Unknown {
            if let Ok(id) = db.open_presence_interval(now, self.current_state.as_str(), &self.current_source) {
                self.current_interval_id = Some(id);
            }
        }

        // Rollup presence buckets periodically
        if self.last_rollup.elapsed() >= Duration::from_secs(15) {
            let _ = rollup_presence(db, cfg, now);
            self.last_rollup = Instant::now();
        }
    }

    fn handle_event(&mut self, db: &ActivityDb, cfg: &ActivityConfig, ev: PresenceEvent) {
        match ev {
            PresenceEvent::Resumed { ts_ms } => {
                self.last_input_ts = ts_ms;
                if self.current_state == PresenceState::Locked || self.current_state == PresenceState::Asleep {
                    return; // unlock/wake events handle these
                }
                if self.current_state != PresenceState::Active {
                    self.transition(db, ts_ms, PresenceState::Active, "wayland");
                }
            }
            PresenceEvent::Idled { ts_ms } => {
                if self.current_state == PresenceState::Active {
                    self.transition(db, ts_ms, PresenceState::Idle, "wayland");
                }
            }
            PresenceEvent::Locked { ts_ms } => {
                self.last_logind_locked = Some(true);
                if cfg.presence.lock_aware && self.current_state != PresenceState::Locked {
                    self.transition(db, ts_ms, PresenceState::Locked, "logind");
                }
            }
            PresenceEvent::Unlocked { ts_ms } => {
                self.last_logind_locked = Some(false);
                if self.current_state == PresenceState::Locked {
                    self.transition(db, ts_ms, PresenceState::Active, "logind");
                    self.last_input_ts = ts_ms;
                }
            }
            PresenceEvent::Sleep { ts_ms } => {
                if cfg.presence.sleep_aware && self.current_state != PresenceState::Asleep {
                    self.transition(db, ts_ms, PresenceState::Asleep, "logind");
                }
            }
            PresenceEvent::Wake { ts_ms } => {
                if self.current_state == PresenceState::Asleep {
                    // After wake, check lock; default to active
                    let locked = self.last_logind_locked.unwrap_or(false);
                    let next = if locked { PresenceState::Locked } else { PresenceState::Active };
                    self.transition(db, ts_ms, next, "logind");
                    if next == PresenceState::Active { self.last_input_ts = ts_ms; }
                }
            }
            PresenceEvent::Manual { ts_ms, state } => {
                self.transition(db, ts_ms, state, "manual");
                if state == PresenceState::Active { self.last_input_ts = ts_ms; }
            }
        }
    }

    fn poll_logind(&mut self, db: &ActivityDb, cfg: &ActivityConfig, now: i64) {
        if !cfg.presence.lock_aware && !cfg.presence.sleep_aware { return; }
        let Some((locked, active)) = query_logind_session() else { return; };
        let was_locked = self.last_logind_locked;
        self.last_logind_locked = Some(locked);

        if cfg.presence.sleep_aware && !active && self.current_state != PresenceState::Asleep && self.current_state != PresenceState::Locked {
            self.transition(db, now, PresenceState::Asleep, "logind");
            return;
        }
        if cfg.presence.lock_aware {
            if locked && self.current_state != PresenceState::Locked && self.current_state != PresenceState::Asleep {
                self.transition(db, now, PresenceState::Locked, "logind");
                return;
            }
            if !locked && was_locked == Some(true) && self.current_state == PresenceState::Locked {
                self.transition(db, now, PresenceState::Active, "logind");
                self.last_input_ts = now;
                return;
            }
        }
        if cfg.presence.sleep_aware && active && self.current_state == PresenceState::Asleep {
            let next = if locked { PresenceState::Locked } else { PresenceState::Active };
            self.transition(db, now, next, "logind");
            if next == PresenceState::Active { self.last_input_ts = now; }
        }
    }

    fn apply_timeouts(&mut self, db: &ActivityDb, cfg: &ActivityConfig, now: i64) {
        let idle_ms = (cfg.presence.idle_timeout.max(30) * 1000) as i64;
        let away_ms = (cfg.presence.away_timeout.max(cfg.presence.idle_timeout.max(30)) * 1000) as i64;

        match self.current_state {
            PresenceState::Active => {
                // If no wayland events are feeding us, fall back to time-based idle detection
                // only if we haven't received input recently. This is a safety net; the
                // primary idle signal is the ext-idle-notify idled event.
                if now - self.last_input_ts >= away_ms {
                    self.transition(db, self.last_input_ts + away_ms, PresenceState::Away, "timeout");
                } else if now - self.last_input_ts >= idle_ms {
                    // Don't auto-transition to idle if wayland source is active; the idled event
                    // is more accurate. Only use timeout fallback when source is not wayland.
                    if self.current_source != "wayland" {
                        self.transition(db, self.last_input_ts + idle_ms, PresenceState::Idle, "timeout");
                    }
                }
            }
            PresenceState::Idle => {
                let idle_since = self.last_transition_ts;
                if now - idle_since >= (away_ms - idle_ms).max(0) {
                    self.transition(db, idle_since + (away_ms - idle_ms).max(0), PresenceState::Away, "timeout");
                }
            }
            PresenceState::Unknown => {
                // Resolve unknown to active as a conservative startup default when presence is enabled
                // but no source has reported yet. Better to be slightly over-present than to miss time;
                // lock/sleep/idle will correct it.
                if cfg.presence.enabled {
                    self.transition(db, now, PresenceState::Active, "init");
                    self.last_input_ts = now;
                }
            }
            _ => {}
        }
    }

    fn transition(&mut self, db: &ActivityDb, ts_ms: i64, new_state: PresenceState, source: &str) {
        if self.current_state == new_state && self.current_interval_id.is_some() {
            return;
        }
        let ts = ts_ms.max(self.last_transition_ts);
        if let Some(id) = self.current_interval_id.take() {
            let _ = db.close_presence_interval(id, ts);
        }
        self.current_state = new_state;
        self.current_source = source.to_string();
        self.last_transition_ts = ts;
        if let Ok(id) = db.open_presence_interval(ts, new_state.as_str(), source) {
            self.current_interval_id = Some(id);
        }
    }
}

fn query_logind_session() -> Option<(bool, bool)> {
    // Returns (locked, active)
    let session_id = std::env::var("XDG_SESSION_ID").ok();
    let mut cmd = std::process::Command::new("loginctl");
    if let Some(sid) = session_id.as_deref() {
        cmd.args(["show-session", sid, "-p", "LockedHint", "-p", "Active"]);
    } else {
        cmd.args(["show-session", "self", "-p", "LockedHint", "-p", "Active"]);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() { return None; }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut locked = false;
    let mut active = true;
    for line in stdout.lines() {
        if let Some(v) = line.strip_prefix("LockedHint=") {
            locked = v.trim() == "yes";
        } else if let Some(v) = line.strip_prefix("Active=") {
            active = v.trim() == "yes";
        }
    }
    Some((locked, active))
}

pub fn rollup_presence(db: &ActivityDb, cfg: &ActivityConfig, now_ms_val: i64) -> Result<(), String> {
    // Recompute last 2 hours by default; on first run or gaps compute wider range.
    let last_rollup_hour = db.get_meta_i64("presence_rollup_hour").ok().flatten().unwrap_or(0);
    let from_hour = if last_rollup_hour > 0 {
        last_rollup_hour - 3600
    } else {
        // backfill from earliest open/closed interval in recent window (7 days)
        let earliest = now_ms_val - 7 * 24 * 3600 * 1000;
        hour_floor(earliest)
    };
    let to_hour = hour_floor(now_ms_val) + 3600;

    let intervals = db.presence_intervals_in_range(from_hour * 1000, to_hour * 1000)?;
    // Build per-hour buckets
    let mut hour = from_hour;
    while hour < to_hour {
        let hour_start = hour * 1000;
        let hour_end = (hour + 3600) * 1000;
        let mut b = PresenceBucket {
            hour,
            ..Default::default()
        };
        // Preserve category fields computed by focus rollup
        if let Ok(existing) = db.presence_buckets_in_range(hour, hour + 3600) {
            if let Some(e) = existing.first() {
                b.work_seconds = e.work_seconds;
                b.active_work_seconds = e.active_work_seconds;
                b.leisure_seconds = e.leisure_seconds;
                b.neutral_seconds = e.neutral_seconds;
                b.unattributed_seconds = e.unattributed_seconds;
                b.unassigned_work_seconds = e.unassigned_work_seconds;
            }
        }
        for iv in &intervals {
            let start = iv.start_ts.max(hour_start);
            let end = iv.end_ts.unwrap_or(now_ms_val).min(hour_end);
            if end <= start { continue; }
            let secs = ((end - start + 999) / 1000).max(0);
            match iv.state.as_str() {
                "active" => b.active_seconds += secs,
                "idle" => b.idle_seconds += secs,
                "away" => b.away_seconds += secs,
                "locked" => b.locked_seconds += secs,
                "asleep" => b.asleep_seconds += secs,
                _ => {}
            }
        }
        // Clamp to hour
        let sum = b.active_seconds + b.idle_seconds + b.away_seconds + b.locked_seconds + b.asleep_seconds;
        if sum > 3600 {
            // Scale down proportionally to avoid rounding overflow
            let scale = 3600.0 / sum as f64;
            b.active_seconds = (b.active_seconds as f64 * scale).round() as i64;
            b.idle_seconds = (b.idle_seconds as f64 * scale).round() as i64;
            b.away_seconds = (b.away_seconds as f64 * scale).round() as i64;
            b.locked_seconds = (b.locked_seconds as f64 * scale).round() as i64;
            b.asleep_seconds = (3600 - b.active_seconds - b.idle_seconds - b.away_seconds - b.locked_seconds).max(0);
        }
        db.upsert_presence_bucket(&b)?;
        hour += 3600;
    }
    let _ = db.set_meta("presence_rollup_hour", &hour_floor(now_ms_val).to_string());
    let _ = cfg; // reserved for future work/leisure categorization
    Ok(())
}

pub fn presence_totals_for_range(db: &ActivityDb, from_hour: i64, to_hour: i64, now_ms_val: i64) -> Result<PresenceTotals, String> {
    // Ensure recent buckets are rolled up for open interval accuracy
    // (caller should have rolled up; this is just a read)
    let mut totals = db.presence_totals(from_hour, to_hour)?;
    // If current hour bucket is missing or stale due to open interval, compute live overlay
    let cur_hour = hour_floor(now_ms_val);
    if cur_hour >= from_hour && cur_hour < to_hour {
        // recompute current hour live from intervals for accuracy
        let hour_start = cur_hour * 1000;
        let hour_end = (cur_hour + 3600) * 1000;
        let intervals = db.presence_intervals_in_range(hour_start, hour_end)?;
        let mut cur = PresenceBucket { hour: cur_hour, ..Default::default() };
        for iv in &intervals {
            let start = iv.start_ts.max(hour_start);
            let end = iv.end_ts.unwrap_or(now_ms_val).min(hour_end);
            if end <= start { continue; }
            let secs = ((end - start + 999) / 1000).max(0);
            match iv.state.as_str() {
                "active" => cur.active_seconds += secs,
                "idle" => cur.idle_seconds += secs,
                "away" => cur.away_seconds += secs,
                "locked" => cur.locked_seconds += secs,
                "asleep" => cur.asleep_seconds += secs,
                _ => {}
            }
        }
        // Replace current hour contribution: subtract stored, add live
        if let Ok(stored) = db.presence_buckets_in_range(cur_hour, cur_hour + 3600) {
            if let Some(s) = stored.first() {
                totals.active_seconds = totals.active_seconds - s.active_seconds + cur.active_seconds;
                totals.idle_seconds = totals.idle_seconds - s.idle_seconds + cur.idle_seconds;
                totals.away_seconds = totals.away_seconds - s.away_seconds + cur.away_seconds;
                totals.locked_seconds = totals.locked_seconds - s.locked_seconds + cur.locked_seconds;
                totals.asleep_seconds = totals.asleep_seconds - s.asleep_seconds + cur.asleep_seconds;
                totals.present_seconds = totals.active_seconds + totals.idle_seconds;
            }
        }
    }
    Ok(totals)
}

fn hour_floor(ts_ms: i64) -> i64 {
    if ts_ms <= 0 { return 0; }
    let secs = ts_ms / 1000;
    (secs / 3600) * 3600
}
