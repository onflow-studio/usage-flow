//! Codex CLI activity and limits from its session logs (`<codex home>/sessions/YYYY/MM/DD/*.jsonl`).
//! After each model response Codex writes a `token_count` event carrying that response's token
//! usage and a snapshot of the plan's rate limits, so no network calls are needed.

use crate::activity::{self, Activity, Clock, SessionCount};
use crate::api::Limit;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Deserialize)]
struct Line {
    timestamp: Option<DateTime<Utc>>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: String,
    info: Option<Info>,
    rate_limits: Option<RateLimits>,
}

#[derive(Deserialize)]
struct Info {
    total_token_usage: Usage,
    last_token_usage: Usage,
}

#[derive(Deserialize)]
struct Usage {
    total_tokens: u64,
}

#[derive(Deserialize, Clone)]
struct RateLimits {
    primary: Option<Window>,
    secondary: Option<Window>,
}

#[derive(Deserialize, Clone)]
struct Window {
    used_percent: f32,
    window_minutes: i64,
    resets_at: Option<i64>,
}

pub struct Snapshot {
    pub limits: Vec<Limit>,
    pub at: DateTime<Utc>,
}

pub struct Scanner {
    sessions: PathBuf,
    offsets: HashMap<PathBuf, u64>,
    entries: Vec<(DateTime<Utc>, u64)>,
    /// Running total per file; Codex repeats the event when nothing new was used.
    totals: HashMap<PathBuf, u64>,
    latest: Option<(DateTime<Utc>, RateLimits)>,
}

impl Scanner {
    pub fn new(codex_home: &Path) -> Self {
        Self {
            sessions: codex_home.join("sessions"),
            offsets: HashMap::new(),
            entries: Vec::new(),
            totals: HashMap::new(),
            latest: None,
        }
    }

    pub fn scan(&mut self) -> (Activity, Option<Snapshot>) {
        let clock = Clock::now();
        let mut sessions = SessionCount::default();
        let mut files = Vec::new();
        collect_jsonl(&self.sessions, 4, &mut files);
        for (path, modified) in files {
            // Older files still matter until we've seen one limits snapshot.
            if modified < clock.since() && self.latest.is_some() {
                continue;
            }
            if modified >= clock.since() {
                sessions.add(&clock, modified);
            }
            let mut offsets = std::mem::take(&mut self.offsets);
            activity::read_appended(&mut offsets, &path, |line| self.ingest(&path, line));
            self.offsets = offsets;
        }

        self.entries.retain(|(at, _)| *at >= clock.cutoff);
        let activity = activity::summarize(&clock, sessions, self.entries.iter().copied());
        let snapshot = self.latest.as_ref().map(|(at, limits)| Snapshot {
            limits: to_limits(limits),
            at: *at,
        });
        (activity, snapshot)
    }

    fn ingest(&mut self, path: &Path, line: &[u8]) {
        if !activity::contains(line, b"\"token_count\"") {
            return;
        }
        let Ok(Line { timestamp: Some(at), payload: Some(p) }) = serde_json::from_slice::<Line>(line)
        else {
            return;
        };
        if p.kind != "token_count" {
            return;
        }
        if let Some(info) = p.info {
            let previous = self.totals.insert(path.to_path_buf(), info.total_token_usage.total_tokens);
            if previous != Some(info.total_token_usage.total_tokens) {
                self.entries.push((at, info.last_token_usage.total_tokens));
            }
        }
        if let Some(limits) = p.rate_limits
            && self.latest.as_ref().is_none_or(|(t, _)| at >= *t)
        {
            self.latest = Some((at, limits));
        }
    }
}

fn to_limits(r: &RateLimits) -> Vec<Limit> {
    let now = Utc::now();
    [&r.primary, &r.secondary]
        .into_iter()
        .flatten()
        .map(|w| {
            let resets_at = w.resets_at.and_then(|s| DateTime::from_timestamp(s, 0));
            // The snapshot predates a reset: the window has started over since.
            let expired = resets_at.is_some_and(|t| t <= now);
            let name = match w.window_minutes {
                300 => "Session (5h)".to_string(),
                10080 => "Week".to_string(),
                m if m % 1440 == 0 => format!("{}-day window", m / 1440),
                m => format!("{}h window", m / 60),
            };
            Limit {
                name,
                percent: if expired { 0.0 } else { w.used_percent },
                resets_at: if expired { None } else { resets_at },
                window_secs: w.window_minutes * 60,
                primary: true,
            }
        })
        .collect()
}

fn collect_jsonl(dir: &Path, depth: u8, out: &mut Vec<(PathBuf, SystemTime)>) {
    for path in activity::read_dir(dir) {
        if path.is_dir() {
            if depth > 0 {
                collect_jsonl(&path, depth - 1, out);
            }
        } else if path.extension().is_some_and(|e| e == "jsonl")
            && let Ok(modified) = path.metadata().and_then(|m| m.modified())
        {
            out.push((path, modified));
        }
    }
}
