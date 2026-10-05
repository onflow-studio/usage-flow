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
    /// Limits come from Anthropic's usage endpoint with the CLI's own login. The Keychain
    /// service it is kept under on macOS names the account everywhere.
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

/// `~/.claude` plus every `~/.claude-*` directory that has credentials. On macOS Claude Code
/// keeps them in the Keychain, those of a custom `CLAUDE_CONFIG_DIR` under a service name
/// suffixed with the first 8 hex chars of sha256(config dir). On Linux they are a file in the
/// config directory.
pub fn discover() -> Vec<Account> {
    let home = crate::settings::home();
    let mut accounts = Vec::new();

    if credentials(KEYCHAIN_SERVICE, &home.join(".claude")).is_some() {
        let label = label_for(&home.join(".claude.json"), KEYCHAIN_SERVICE, &home.join(".claude"), "default");
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
        if credentials(&service, &dir).is_none() {
            continue;
        }
        let fallback = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .trim_start_matches(".claude-")
            .to_string();
        let label = label_for(&dir.join(".claude.json"), &service, &dir, &fallback);
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

pub fn read_token(service: &str, config_dir: &Path) -> Option<Token> {
    let creds: Credentials = serde_json::from_str(&credentials(service, config_dir)?).ok()?;
    Some(Token {
        access_token: creds.oauth.access_token,
        expires_at_ms: creds.oauth.expires_at,
    })
}

fn dir_hash(dir: &Path) -> String {
    let digest = Sha256::digest(dir.to_string_lossy().as_bytes());
    digest.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// The login Claude Code keeps for an account, as JSON.
#[cfg(target_os = "macos")]
fn credentials(service: &str, _config_dir: &Path) -> Option<String> {
    let out = Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The login Claude Code keeps for an account, as JSON.
#[cfg(target_os = "linux")]
fn credentials(_service: &str, config_dir: &Path) -> Option<String> {
    std::fs::read_to_string(config_dir.join(".credentials.json")).ok()
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

fn label_for(claude_json: &Path, service: &str, config_dir: &Path, fallback: &str) -> String {
    let email = std::fs::read_to_string(claude_json)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["oauthAccount"]["emailAddress"].as_str().map(String::from));
    let plan = credentials(service, config_dir)
        .and_then(|s| serde_json::from_str::<Credentials>(&s).ok())
        .and_then(|c| plan_name(c.oauth.subscription_type, c.oauth.rate_limit_tier));
    match (email, plan) {
        (Some(e), Some(p)) => format!("{e} · {p}"),
        (Some(e), None) => e,
        (None, _) => fallback.to_string(),
    }
}

/// Starts a new Claude Code login: asks for a name, then opens a terminal on `claude` with a
/// config folder of its own. The account shows up by itself once it has signed in.
pub fn add_claude() {
    std::thread::spawn(|| {
        let Some(answer) = ask_name() else { return };
        let name: String = answer
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_whitespace() { '-' } else { c })
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if name.is_empty() {
            return;
        }
        let dir = crate::settings::home().join(format!(".claude-{name}"));
        let launcher = crate::settings::dir().join(LAUNCHER);
        let lines = [
            SHEBANG.to_string(),
            format!("export CLAUDE_CONFIG_DIR=\"{}\"", dir.display()),
            "echo \"Usage Flow: sign in with /login, then /exit. The account appears in the panel within a minute.\"".to_string(),
            RUN_CLAUDE.to_string(),
        ];
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::create_dir_all(crate::settings::dir());
        if std::fs::write(&launcher, lines.join("\n") + "\n").is_err() {
            return;
        }
        let _ = Command::new("chmod").arg("+x").arg(&launcher).status();
        open_terminal(&launcher);
    });
}

const ASK_NAME: &str = "Name for the new Claude Code account, e.g. work. Terminal opens to sign it in, and it shows up here once it has.";

#[cfg(target_os = "macos")]
const LAUNCHER: &str = "sign-in.command";
#[cfg(target_os = "macos")]
const SHEBANG: &str = "#!/bin/zsh -li";
#[cfg(target_os = "macos")]
const RUN_CLAUDE: &str = "exec claude";

#[cfg(target_os = "macos")]
fn ask_name() -> Option<String> {
    let script = format!(
        "display dialog \"{ASK_NAME}\" default answer \"\" with title \"Usage Flow\" buttons {{\"Cancel\", \"Open Terminal\"}} default button 2"
    );
    let out = Command::new("osascript").args(["-e", &script]).output().ok()?;
    let answer = String::from_utf8_lossy(&out.stdout);
    let (_, name) = answer.trim().split_once("text returned:")?;
    Some(name.to_string())
}

#[cfg(target_os = "macos")]
fn open_terminal(launcher: &Path) {
    let _ = Command::new("open").arg(launcher).spawn();
}

#[cfg(target_os = "linux")]
const LAUNCHER: &str = "sign-in.sh";
#[cfg(target_os = "linux")]
const SHEBANG: &str = "#!/bin/sh";
/// Through the login shell, as that is where `claude` is on the path.
#[cfg(target_os = "linux")]
const RUN_CLAUDE: &str = "exec \"${SHELL:-/bin/sh}\" -lic \"exec claude\"";

#[cfg(target_os = "linux")]
fn ask_name() -> Option<String> {
    let zenity = ["--entry", "--title", "Usage Flow", "--ok-label", "Open Terminal", "--width", "420", "--text", ASK_NAME];
    let kdialog = ["--title", "Usage Flow", "--inputbox", ASK_NAME];
    let asked = Command::new("zenity").args(zenity).output().or_else(|_| Command::new("kdialog").args(kdialog).output());
    let out = asked.ok().filter(|out| out.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The desktop's own terminal where it names one, then the usual ones.
#[cfg(target_os = "linux")]
fn open_terminal(launcher: &Path) {
    let terminals = [("x-terminal-emulator", "-e"), ("gnome-terminal", "--"), ("konsole", "-e"), ("xterm", "-e")];
    for (terminal, run) in terminals {
        if Command::new(terminal).arg(run).arg(launcher).spawn().is_ok() {
            return;
        }
    }
}
