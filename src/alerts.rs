use crate::api::Limit;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::process::Command;

const THRESHOLDS: [f32; 3] = [80.0, 90.0, 100.0];

struct Seen {
    resets_at: Option<DateTime<Utc>>,
    level: usize,
}

/// Notifies when a limit crosses 80/90/100% and when a primary window resets.
/// The first reading of each limit only records state, so launching the app is silent.
#[derive(Default)]
pub struct Alerts {
    seen: HashMap<(String, String), Seen>,
}

impl Alerts {
    pub fn check(&mut self, account: &str, limits: &[Limit]) {
        for limit in limits {
            let level = THRESHOLDS.iter().filter(|t| limit.percent >= **t).count();
            let key = (account.to_string(), limit.name.clone());
            let Some(seen) = self.seen.get_mut(&key) else {
                self.seen.insert(
                    key,
                    Seen {
                        resets_at: limit.resets_at,
                        level,
                    },
                );
                continue;
            };

            // resets_at jitters by milliseconds between calls, so only a real move counts.
            let reset = match (seen.resets_at, limit.resets_at) {
                (Some(old), Some(new)) => (new - old).num_seconds() > 60,
                _ => false,
            };
            if reset && limit.primary {
                notify(&format!("{} reset", limit.name), account);
            } else if level > seen.level {
                notify(
                    &format!("{} at {:.0}%", limit.name, limit.percent),
                    account,
                );
            }
            seen.resets_at = limit.resets_at;
            seen.level = level;
        }
    }
}

fn notify(title: &str, body: &str) {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"Claude Usage\" subtitle \"{}\"",
        esc(body),
        esc(title)
    );
    let _ = Command::new("osascript").args(["-e", &script]).spawn();
}
