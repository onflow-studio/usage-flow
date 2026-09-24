//! "Open at login" via a per-user LaunchAgent. On by default; turning it off leaves a marker
//! so the default doesn't re-enable it on the next launch.

use std::path::PathBuf;

const LABEL: &str = "com.claude-usage.app";

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn plist_path() -> PathBuf {
    home().join(format!("Library/LaunchAgents/{LABEL}.plist"))
}

fn disabled_marker() -> PathBuf {
    home().join("Library/Application Support/claude-usage/login-disabled")
}

pub fn enabled() -> bool {
    !disabled_marker().exists()
}

/// Writes (or refreshes) the LaunchAgent so it points at the running binary.
pub fn sync() {
    if !enabled() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
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
    let path = plist_path();
    if std::fs::read_to_string(&path).ok().as_deref() == Some(plist.as_str()) {
        return;
    }
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(path, plist);
}

pub fn set_enabled(on: bool) {
    let marker = disabled_marker();
    if on {
        let _ = std::fs::remove_file(marker);
        sync();
    } else {
        let _ = std::fs::create_dir_all(marker.parent().unwrap());
        let _ = std::fs::write(marker, "");
        let _ = std::fs::remove_file(plist_path());
    }
}
