use chrono::{DateTime, Utc};
use serde::Deserialize;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct Limit {
    pub name: String,
    pub percent: f32,
    pub resets_at: Option<DateTime<Utc>>,
    /// Length of the rolling window, used to compute pace.
    pub window_secs: i64,
    /// Only the headline limits (session, all-models week) trigger reset notifications.
    pub primary: bool,
}

const SESSION_SECS: i64 = 5 * 3600;
const WEEK_SECS: i64 = 7 * 86400;

#[derive(Deserialize)]
struct UsageResponse {
    #[serde(default)]
    limits: Vec<RawLimit>,
    five_hour: Option<Window>,
    seven_day: Option<Window>,
}

#[derive(Deserialize)]
struct RawLimit {
    kind: String,
    percent: f32,
    resets_at: Option<DateTime<Utc>>,
    scope: Option<Scope>,
}

#[derive(Deserialize)]
struct Scope {
    model: Option<Model>,
}

#[derive(Deserialize)]
struct Model {
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct Window {
    utilization: f32,
    resets_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct ApiError {
    error: ApiErrorBody,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    message: String,
}

pub fn fetch(access_token: &str) -> Result<Vec<Limit>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .into();
    let mut resp = agent
        .get(USAGE_URL)
        .header("Authorization", &format!("Bearer {access_token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .call()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;

    if status == 429 {
        return Err("rate limited by Anthropic — showing last reading, retrying in 10 min".into());
    }
    if !status.is_success() {
        let msg = serde_json::from_str::<ApiError>(&body)
            .map(|e| e.error.message)
            .unwrap_or_else(|_| format!("HTTP {status}"));
        return Err(msg);
    }

    let usage: UsageResponse = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(to_limits(usage))
}

fn to_limits(usage: UsageResponse) -> Vec<Limit> {
    if !usage.limits.is_empty() {
        return usage
            .limits
            .into_iter()
            .map(|l| {
                let model = l
                    .scope
                    .and_then(|s| s.model)
                    .and_then(|m| m.display_name);
                let name = match (l.kind.as_str(), model) {
                    ("session", _) => "Session (5h)".to_string(),
                    ("weekly_all", _) => "Week".to_string(),
                    (_, Some(m)) => format!("Week · {m}"),
                    (kind, None) => kind.replace('_', " "),
                };
                let is_session = l.kind == "session";
                Limit {
                    name,
                    percent: l.percent,
                    resets_at: l.resets_at,
                    window_secs: if is_session { SESSION_SECS } else { WEEK_SECS },
                    primary: is_session || l.kind == "weekly_all",
                }
            })
            .collect();
    }

    [
        ("Session (5h)", usage.five_hour, SESSION_SECS),
        ("Week", usage.seven_day, WEEK_SECS),
    ]
    .into_iter()
    .filter_map(|(name, w, window_secs)| {
        w.map(|w| Limit {
            name: name.into(),
            percent: w.utilization,
            resets_at: w.resets_at,
            window_secs,
            primary: true,
        })
    })
        .collect()
}
