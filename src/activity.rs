//! Live activity from Claude Code's local transcripts (`<config>/projects/**/*.jsonl`).
//! Files are read incrementally: each scan only parses bytes appended since the last one.

use chrono::{DateTime, Local, Timelike, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const ACTIVE_WINDOW: Duration = Duration::from_secs(5 * 60);
pub const HOURS: usize = 12;

#[derive(Clone, Debug)]
pub struct Activity {
    pub active_sessions: usize,
    /// Sessions with any activity since local midnight.
    pub sessions_today: usize,
    pub hour_tokens: u64,
    pub today_tokens: u64,
    /// Tokens per clock hour for the last `HOURS` hours, oldest first; the last is the current hour.
    pub hourly: Vec<u64>,
    /// Local start of the oldest bucket in `hourly`.
    pub hourly_start: DateTime<Local>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    timestamp: Option<DateTime<Utc>>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

struct Entry {
    at: DateTime<Utc>,
    tokens: u64,
}

pub struct Scanner {
    projects: PathBuf,
    offsets: HashMap<PathBuf, u64>,
    /// Keyed by message id: Claude Code writes one line per content block with the same
    /// usage, so the last write wins.
    entries: HashMap<String, Entry>,
}

impl Scanner {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            projects: config_dir.join("projects"),
            offsets: HashMap::new(),
            entries: HashMap::new(),
        }
    }

    pub fn scan(&mut self) -> Activity {
        let clock = Clock::now();
        let mut sessions = SessionCount::default();
        for (path, modified, is_subagent) in transcripts(&self.projects) {
            if modified < clock.since() {
                continue;
            }
            if !is_subagent {
                sessions.add(&clock, modified);
            }
            self.read_new(&path);
        }

        self.entries.retain(|_, e| e.at >= clock.cutoff);
        self.offsets.retain(|p, _| p.exists());
        summarize(&clock, sessions, self.entries.values().map(|e| (e.at, e.tokens)))
    }

    fn read_new(&mut self, path: &Path) {
        let mut offsets = std::mem::take(&mut self.offsets);
        read_appended(&mut offsets, path, |line| self.ingest(line));
        self.offsets = offsets;
    }

    fn ingest(&mut self, line: &[u8]) {
        if !contains(line, b"\"usage\"") {
            return;
        }
        let Ok(parsed) = serde_json::from_slice::<Line>(line) else { return };
        if parsed.kind != "assistant" {
            return;
        }
        let (Some(at), Some(msg)) = (parsed.timestamp, parsed.message) else { return };
        let (Some(id), Some(usage)) = (msg.id, msg.usage) else { return };
        self.entries.insert(
            id,
            Entry {
                at,
                tokens: usage.input_tokens
                    + usage.output_tokens
                    + usage.cache_read_input_tokens
                    + usage.cache_creation_input_tokens,
            },
        );
    }
}

/// Time boundaries for one scan.
pub struct Clock {
    pub now: SystemTime,
    pub midnight: DateTime<Utc>,
    pub hourly_start: DateTime<Local>,
    /// Oldest timestamp any scan needs to keep.
    pub cutoff: DateTime<Utc>,
}

impl Clock {
    pub fn now() -> Self {
        let midnight = Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|t| t.and_local_timezone(Local).earliest())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let hourly_start = Local::now()
            .with_minute(0)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .unwrap_or_else(Local::now)
            - chrono::Duration::hours(HOURS as i64 - 1);
        Self {
            now: SystemTime::now(),
            midnight,
            hourly_start,
            cutoff: midnight.min(hourly_start.with_timezone(&Utc)),
        }
    }

    /// Files last modified before this can't contain anything we keep.
    pub fn since(&self) -> SystemTime {
        self.cutoff.into()
    }
}

#[derive(Default)]
pub struct SessionCount {
    active: usize,
    today: usize,
}

impl SessionCount {
    pub fn add(&mut self, clock: &Clock, modified: SystemTime) {
        if modified >= SystemTime::from(clock.midnight) {
            self.today += 1;
        }
        if clock.now.duration_since(modified).unwrap_or_default() < ACTIVE_WINDOW {
            self.active += 1;
        }
    }
}

pub fn summarize(
    clock: &Clock,
    sessions: SessionCount,
    entries: impl Iterator<Item = (DateTime<Utc>, u64)>,
) -> Activity {
    let hour_ago = Utc::now() - chrono::Duration::hours(1);
    let hourly_start = clock.hourly_start.with_timezone(&Utc);
    let mut activity = Activity {
        active_sessions: sessions.active,
        sessions_today: sessions.today,
        hour_tokens: 0,
        today_tokens: 0,
        hourly: vec![0; HOURS],
        hourly_start: clock.hourly_start,
    };
    for (at, tokens) in entries {
        let bucket = (at - hourly_start).num_hours();
        if (0..HOURS as i64).contains(&bucket) {
            activity.hourly[bucket as usize] += tokens;
        }
        if at >= hour_ago {
            activity.hour_tokens += tokens;
        }
        if at >= clock.midnight {
            activity.today_tokens += tokens;
        }
    }
    activity
}

/// Reads bytes appended to `path` since `offsets[path]`, calling `ingest` per complete line.
/// A partially written last line is left for the next scan.
pub fn read_appended(offsets: &mut HashMap<PathBuf, u64>, path: &Path, mut ingest: impl FnMut(&[u8])) {
    let Ok(mut file) = File::open(path) else { return };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let offset = offsets.get(path).copied().unwrap_or(0);
    let offset = if len < offset { 0 } else { offset };
    if len == offset || file.seek(SeekFrom::Start(offset)).is_err() {
        return;
    }
    let mut buf = Vec::with_capacity((len - offset) as usize);
    if file.read_to_end(&mut buf).is_err() {
        return;
    }
    let Some(end) = buf.iter().rposition(|&b| b == b'\n') else { return };
    for line in buf[..end].split(|&b| b == b'\n') {
        ingest(line);
    }
    offsets.insert(path.to_path_buf(), offset + end as u64 + 1);
}

/// Top-level session transcripts plus subagent transcripts in `<session>/subagents/`.
fn transcripts(projects: &Path) -> Vec<(PathBuf, SystemTime, bool)> {
    let mut out = Vec::new();
    for project in read_dir(projects) {
        for entry in read_dir(&project) {
            if entry.is_dir() {
                for sub in read_dir(&entry.join("subagents")) {
                    push_jsonl(&mut out, sub, true);
                }
            } else {
                push_jsonl(&mut out, entry, false);
            }
        }
    }
    out
}

fn push_jsonl(out: &mut Vec<(PathBuf, SystemTime, bool)>, path: PathBuf, is_subagent: bool) {
    if path.extension().is_some_and(|e| e == "jsonl")
        && let Ok(modified) = path.metadata().and_then(|m| m.modified())
    {
        out.push((path, modified, is_subagent));
    }
}

pub fn read_dir(dir: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
