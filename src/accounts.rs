use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

#[derive(Clone, Debug)]
pub struct Account {
    pub label: String,
    /// Short prefix for the menu bar title.
    pub tag: String,
    pub kind: Kind,
    pub config_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub enum Kind {
    /// Limits come from Anthropic's usage endpoint with the CLI's Keychain login.
    Claude { keychain_service: String },
    /// Limits come from the snapshots Codex writes into its own session logs.
    Codex,
}

impl Account {
    /// Stable per account; used to key the on-disk cache.
    pub fn key(&self) -> &str {
        match &self.kind {
            Kind::Claude { keychain_service } => keychain_service,
            Kind::Codex => "codex",
        }
    }
}

#[derive(Deserialize)]
struct Credentials {
    #[serde(rename = "claudeAiOauth")]
    oauth: OAuth,
}

#[derive(Deserialize)]
struct OAuth {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    /// e.g. `default_claude_max_20x`.
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

pub struct Token {
    pub access_token: String,
    pub expires_at_ms: Option<i64>,
}

/// `~/.claude` plus every `~/.claude-*` directory that has credentials in the Keychain.
/// Claude Code stores credentials for a custom `CLAUDE_CONFIG_DIR` under a service name
/// suffixed with the first 8 hex chars of sha256(config dir).
pub fn discover() -> Vec<Account> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let mut accounts = Vec::new();

    if keychain_read(KEYCHAIN_SERVICE).is_some() {
        let label = label_for(&home.join(".claude.json"), KEYCHAIN_SERVICE, "default");
        accounts.push(Account {
            tag: tag_for(&label),
            label,
            kind: Kind::Claude { keychain_service: KEYCHAIN_SERVICE.into() },
            config_dir: home.join(".claude"),
        });
    }

    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&home)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(".claude-"))
        })
        .collect();
    dirs.sort();

    for dir in dirs {
        let service = format!("{KEYCHAIN_SERVICE}-{}", dir_hash(&dir));
        if keychain_read(&service).is_none() {
            continue;
        }
        let fallback = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .trim_start_matches(".claude-")
            .to_string();
        let label = label_for(&dir.join(".claude.json"), &service, &fallback);
        accounts.push(Account {
            tag: tag_for(&label),
            label,
            kind: Kind::Claude { keychain_service: service },
            config_dir: dir,
        });
    }

    accounts.extend(codex(&home));
    accounts
}

/// Codex CLI signed in with ChatGPT (`$CODEX_HOME`, default `~/.codex`).
fn codex(home: &Path) -> Option<Account> {
    use base64::Engine;
    let dir = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("auth.json")).ok()?).ok()?;
    let id_token = auth["tokens"]["id_token"].as_str()?;
    let payload = id_token.split('.').nth(1)?;
    let claims: serde_json::Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .ok()?,
    )
    .ok()?;
    let email = claims["email"].as_str().unwrap_or("Codex");
    let plan = claims["https://api.openai.com/auth"]["chatgpt_plan_type"]
        .as_str()
        .map(|p| format!(" {p}"))
        .unwrap_or_default();
    Some(Account {
        label: format!("{email} · codex{plan}"),
        tag: "Cx".into(),
        kind: Kind::Codex,
        config_dir: dir,
    })
}

pub fn read_token(service: &str) -> Option<Token> {
    let creds: Credentials = serde_json::from_str(&keychain_read(service)?).ok()?;
    Some(Token {
        access_token: creds.oauth.access_token,
        expires_at_ms: creds.oauth.expires_at,
    })
}

fn dir_hash(dir: &Path) -> String {
    let digest = Sha256::digest(dir.to_string_lossy().as_bytes());
    digest.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

fn keychain_read(service: &str) -> Option<String> {
    let out = Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// "max" + "default_claude_max_20x" -> "max 20x".
fn plan_name(subscription: Option<String>, tier: Option<String>) -> Option<String> {
    let multiplier = tier.as_deref().and_then(|t| {
        t.rsplit('_')
            .next()
            .filter(|m| m.ends_with('x') && m[..m.len() - 1].parse::<u32>().is_ok())
            .map(String::from)
    });
    match (subscription, multiplier) {
        (Some(s), Some(m)) => Some(format!("{s} {m}")),
        (s, _) => s,
    }
}

fn tag_for(label: &str) -> String {
    label
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into())
}

fn label_for(claude_json: &Path, service: &str, fallback: &str) -> String {
    let email = std::fs::read_to_string(claude_json)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["oauthAccount"]["emailAddress"].as_str().map(String::from));
    let plan = keychain_read(service)
        .and_then(|s| serde_json::from_str::<Credentials>(&s).ok())
        .and_then(|c| plan_name(c.oauth.subscription_type, c.oauth.rate_limit_tier));
    match (email, plan) {
        (Some(e), Some(p)) => format!("{e} · {p}"),
        (Some(e), None) => e,
        (None, _) => fallback.to_string(),
    }
}
