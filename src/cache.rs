//! Last good usage reading per account, so a restart shows data immediately.

use crate::api::Limit;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Reading {
    pub limits: Vec<Limit>,
    pub updated_at: DateTime<Utc>,
}

fn path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Library/Application Support/claude-usage/last-usage.json")
}

/// Keyed by Keychain service name, which is stable per account.
pub fn load() -> HashMap<String, Reading> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(key: &str, reading: Reading) {
    let mut all = load();
    all.insert(key.to_string(), reading);
    let path = path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    if let Ok(json) = serde_json::to_string(&all) {
        let _ = std::fs::write(path, json);
    }
}
