<p align="center">
  <img src="docs/hero.png" alt="Usage Flow: a panel hanging from the macOS menu bar, showing Claude Code and Codex limits for two accounts" width="800">
</p>

<h1 align="center">Usage Flow</h1>

<p align="center">
  <b>Claude Code and Codex limits at a glance, in your menu bar.</b><br>
  No dashboard to open, no <code>/usage</code> to type, no surprise at 100%. Look up and stay in flow.
</p>

<p align="center">
  <a href="https://github.com/onflow-studio/usage-flow/releases/latest"><b>Download for macOS (Apple Silicon)</b></a>
  ·
  <a href="#linux">Linux</a>
  ·
  <a href="#build-it-yourself">Build from source</a>
</p>

---

## Why

Limits only show up when you hit them. The session window runs out in the middle of a refactor, the weekly one two days before it resets, and the only way to see either coming is to stop and ask. Usage Flow keeps every account's limits where a glance finds them: a small gauge in the menu bar, and a panel with the detail when you want it.

## What you get

- **Every account, side by side.** Each Claude Code login on your Mac (`~/.claude` and any `~/.claude-*` config folder) plus Codex, found on their own. Take one off the panel or sign a new one in from the menu.
- **Limits with a pace.** Session and weekly windows as bars, with a tick for how much of the window has passed and a projection: `pace: ~72% at reset`, or the time you would hit 100%.
- **An icon that shows what's on.** One bar per account in the menu bar, filled as far as its fullest limit. It turns amber at 70% and red at 90%.
- **What you are burning.** Tokens in the last hour and today, sessions running right now, and a 12-hour chart per account, read from local transcripts.
- **A heads-up before the wall.** A notification at 80, 90 and 100% of a limit, and when a session or week resets.
- **Sticks to a side.** The panel docks to the left or right edge of a display, full height, and sizes its content to the room it has. Drag it anywhere and it snaps to the nearer side.
- **Room of its own.** Tick **Keep Windows Clear of the Panel** and other apps' windows are moved out from under it, as if the panel were part of the screen's edge.
- **Settings in the menu.** Click the gauge for the panel, accounts, menu bar figures, alerts, always on top, all desktops, side and open at login.
- **Quiet by design.** No account, no telemetry, no Dock icon. It only talks to the services you already use.

## Install

1. Download `Usage.Flow-*-arm64-mac.zip` from the [latest release](https://github.com/onflow-studio/usage-flow/releases/latest) and unzip it.
2. Move **Usage Flow.app** to `/Applications`.
3. The app isn't notarized by Apple, so macOS blocks the first launch. Clear the download flag once:

   ```sh
   xattr -dr com.apple.quarantine "/Applications/Usage Flow.app"
   ```

4. Open it. The gauge appears in your menu bar and the panel on the left edge of your screen. It opens at login from then on; click the gauge to turn that off.

## Linux

Usage Flow also runs on Linux desktops with X11 and a system tray. There is no download yet, so it is built from source. On Ubuntu, with Rust 1.85+:

```sh
sudo apt install build-essential pkg-config libgtk-3-dev libayatana-appindicator3-dev libxdo-dev
git clone https://github.com/onflow-studio/usage-flow.git
cd usage-flow
make install-app    # ~/.local/bin/usage-flow, with its icon and its entry among your apps
```

Open **Usage Flow** from your apps. It works as on macOS, with these differences:

- **The gauge is a tray icon.** GNOME shows it through the AppIndicator extension, which Ubuntu ships turned on.
- **Logins come from a file.** Claude Code on Linux keeps each login in `.credentials.json` inside its config folder, and that is where Usage Flow reads it.
- **Keep Windows Clear of the Panel asks for nothing.** The panel reserves its strip with the window manager, as a dock does, so maximized and tiled windows stop at its edge. While it is on, the panel shows on every workspace and is moved from the menu rather than dragged. A panel with another display beyond its edge has no strip to reserve.
- **Settings live in `~/.local/share/usage-flow/`**, and **Open at Login** is an entry in `~/.config/autostart/`.

Tested on Ubuntu 24.04 with GNOME on X11. Under Wayland it runs through XWayland, which has not been tried yet.

## Use

| Do this | To |
| --- | --- |
| Click the gauge | Open the menu |
| **Show Panel** / **Hide Panel** | Show or hide the panel |
| Drag the panel's top row | Move it. It snaps to the nearer side of the display it lands on |
| ↻ in the panel, or **Refresh Now** | Fetch limits now instead of waiting 10 minutes |
| ✕ in the panel | Hide it. **Show Panel** brings it back |
| **Usage in Menu Bar** | Show the figures beside the gauge (`session·week` per account). Off to start with |
| **Alerts at 80, 90 and 100%** | Turn notifications on or off |
| **Always on Top** / **Show on All Desktops** | Keep the panel above other windows, and on every Space |
| **Accounts**, then an account | Take it off the panel, or put it back |
| **Accounts**, then **Add Claude Code Account…** | Name a new login and sign it in from Terminal. It joins the panel once signed in |
| **Move to Right Side** / **Move to Left Side** | Dock the panel against the other edge |
| **Move to Next Display** | Dock the panel on the next display |
| **Keep Windows Clear of the Panel** | Move other windows out from under the panel. The first time on macOS, it opens System Settings to allow Usage Flow under Accessibility |
| **Open at Login** / **Quit Usage Flow** | The usual |

## Stack

```text
>_ usage-flow --stack
lang       Rust 99%
stack      egui · native macOS menu bar and Accessibility · X11 and a tray icon on Linux
talks to   Anthropic's usage endpoint, with the login Claude Code already keeps
reads      Claude Code and Codex transcripts on your Mac
stores     settings and the last reading, in Application Support
runs on    macOS, Apple Silicon · Linux, X11 · no server, no account, no telemetry
```

## How it works

Usage Flow is a small native app written in Rust with [egui](https://github.com/emilk/egui). It has no server and no account of its own.

Claude Code limits come from Anthropic's usage endpoint, asked every 10 minutes with the login Claude Code already keeps in your Keychain. The token is read fresh each time and never stored or sent anywhere else. Codex limits come from the snapshots Codex writes into its own session logs, so they are as fresh as its last reply. Token counts, sessions and the hourly chart are read from the transcripts both tools keep on your Mac, every 10 seconds.

macOS has no way for an app to reserve a strip of the screen, so **Keep Windows Clear of the Panel** does it by hand: with the Accessibility permission, it moves and narrows the windows that overlap the panel. It is off until you tick it, and it leaves full-screen and minimized windows alone. The app isn't signed with a developer ID, so macOS asks for the permission again after each update.

On Linux the login is read from `.credentials.json` in each Claude Code config folder, and X11 does have a way to reserve a strip of the screen, so the panel asks the window manager for it and no window is moved by hand.

Settings and the last reading are saved to `~/Library/Application Support/Usage Flow/`, or `~/.local/share/usage-flow/` on Linux. That folder is the only thing it stores, and it never leaves your Mac.

## Build it yourself

Needs Rust 1.85+ and the Xcode command line tools, or on Linux the packages listed under [Linux](#linux).

```sh
git clone https://github.com/onflow-studio/usage-flow.git
cd usage-flow
make start          # build and launch from source
make install-app    # package and copy to /Applications/Usage Flow.app (~/.local on Linux)
make release        # package a distributable zip into release/ (a tarball on Linux)
```

Colors, type and spacing come from [`DESIGN.md`](DESIGN.md) and live in [`src/theme.rs`](src/theme.rs).

## Origin

Usage Flow started as claude-usage, a personal monitor for a couple of Claude Code accounts. Codex joined, the panel stayed open all day next to [Radio Flow](https://github.com/onflow-studio/radio-flow), and it took the same look to sit beside it.

## License

[MIT](LICENSE). JetBrains Mono is bundled under the [SIL Open Font License](assets/fonts/OFL.txt).
