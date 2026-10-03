//! "Open at login" via a per-user LaunchAgent. On by default; turning it off leaves a marker
//! so the default doesn't re-enable it on the next launch.

use crate::settings;
use std::path::PathBuf;

const LABEL: &str = "dev.f3r.usage-flow";
/// The agent the app installed when it was called claude-usage.
const LEGACY_LABEL: &str = "com.claude-usage.app";

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn plist_path(label: &str) -> PathBuf {
    home().join(format!("Library/LaunchAgents/{label}.plist"))
}

pub fn enabled() -> bool {
    !settings::flag(settings::LOGIN_OFF)
}

/// Writes (or refreshes) the LaunchAgent so it points at the running binary.
pub fn sync() {
    if !enabled() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    // A build run from the source tree never claims the login item.
    if exe.components().any(|c| c.as_os_str() == "target") {
        return;
    }
    let _ = std::fs::remove_file(plist_path(LEGACY_LABEL));
    let exe = exe.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
    );
    let path = plist_path(LABEL);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(plist.as_str()) {
        return;
    }
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(path, plist);
}

pub fn set_enabled(on: bool) {
    settings::set_flag(settings::LOGIN_OFF, !on);
    if on {
        sync();
    } else {
        let _ = std::fs::remove_file(plist_path(LABEL));
    }
}
