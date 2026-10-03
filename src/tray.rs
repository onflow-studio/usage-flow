//! The menu bar item: a gauge icon, the usage figures beside it, and the settings menu.

use crate::accounts::Account;
use crate::window::Side;
use crate::{alerts, login, settings, theme, window};
use eframe::egui::{self, Color32, ViewportCommand};
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// What the menu asks the app to do with its accounts.
pub enum Command {
    Refresh,
    /// Take the account with this key off the panel, or put it back.
    ToggleAccount(String),
    AddAccount,
    /// The accounts on this Mac changed.
    Discovered(Vec<Account>),
}

/// What the menu and the panel both read and change.
pub struct Shared {
    pub visible: AtomicBool,
    pub pinned: AtomicBool,
    pub always_on_top: AtomicBool,
    pub menu_bar_usage: AtomicBool,
    pub move_to_next_display: AtomicBool,
    /// Docked against the right edge of the display rather than the left.
    pub right_side: AtomicBool,
    /// The side changed from the menu and the panel has yet to move there.
    pub dock_requested: AtomicBool,
}

impl Shared {
    pub fn load() -> Self {
        Self {
            visible: AtomicBool::new(true),
            pinned: AtomicBool::new(settings::flag(settings::PINNED)),
            always_on_top: AtomicBool::new(!settings::flag(settings::ALWAYS_ON_TOP_OFF)),
            menu_bar_usage: AtomicBool::new(settings::flag(settings::MENU_BAR_USAGE)),
            move_to_next_display: AtomicBool::new(false),
            right_side: AtomicBool::new(settings::flag(settings::SIDE_RIGHT)),
            dock_requested: AtomicBool::new(false),
        }
    }

    pub fn side(&self) -> Side {
        if self.right_side.load(Ordering::SeqCst) { Side::Right } else { Side::Left }
    }

    pub fn set_side(&self, side: Side) {
        self.right_side.store(side == Side::Right, Ordering::SeqCst);
        settings::set_flag(settings::SIDE_RIGHT, side == Side::Right);
    }

    pub fn toggle_panel(&self, ctx: &egui::Context) {
        let show = !self.visible.fetch_xor(true, Ordering::SeqCst);
        ctx.send_viewport_cmd(ViewportCommand::Visible(show));
        if show {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        ctx.request_repaint();
    }
}

pub struct Tray {
    icon: TrayIcon,
    headline: MenuItem,
    panel: MenuItem,
    next_display: MenuItem,
    side: MenuItem,
    accounts: Submenu,
    /// The submenu's account rows, and which account each one's id stands for.
    account_items: RefCell<Vec<CheckMenuItem>>,
    account_ids: Arc<Mutex<Vec<(MenuId, String)>>>,
    /// What the menu bar shows now, so it is redrawn only when it changes.
    shown: RefCell<Option<(Vec<Option<u8>>, Option<String>)>>,
}

impl Tray {
    pub fn new(ctx: &egui::Context, shared: Arc<Shared>, commands: mpsc::Sender<Command>) -> Option<Self> {
        let checked = |flag: &AtomicBool| flag.load(Ordering::SeqCst);
        let headline = MenuItem::new("Usage Flow", false, None);
        let panel = MenuItem::new("Hide Panel", true, None);
        let refresh = MenuItem::new("Refresh Now", true, None);
        let menu_bar_usage = CheckMenuItem::new("Usage in Menu Bar", true, checked(&shared.menu_bar_usage), None);
        let alerts = CheckMenuItem::new("Alerts at 80, 90 and 100%", true, !settings::flag(settings::ALERTS_OFF), None);
        let always_on_top = CheckMenuItem::new("Always on Top", true, checked(&shared.always_on_top), None);
        let pinned = CheckMenuItem::new("Show on All Desktops", true, checked(&shared.pinned), None);
        let side = MenuItem::new("Move to Right Side", true, None);
        let next_display = MenuItem::new("Move to Next Display", window::screen_count() > 1, None);
        let add_account = MenuItem::new("Add Claude Code Account…", true, None);
        let accounts = Submenu::new("Accounts", true);
        let _ = accounts.append_items(&[&PredefinedMenuItem::separator(), &add_account]);
        let open_at_login = CheckMenuItem::new("Open at Login", true, login::enabled(), None);
        let quit = MenuItem::new("Quit Usage Flow", true, None);

        let menu = Menu::new();
        let _ = menu.append_items(&[
            &headline,
            &panel,
            &refresh,
            &PredefinedMenuItem::separator(),
            &accounts,
            &PredefinedMenuItem::separator(),
            &menu_bar_usage,
            &alerts,
            &always_on_top,
            &pinned,
            &side,
            &next_display,
            &PredefinedMenuItem::separator(),
            &open_at_login,
            &quit,
        ]);
        let icon = TrayIconBuilder::new()
            .with_icon(gauge(&[]))
            .with_icon_as_template(false)
            .with_tooltip("Usage Flow")
            .with_menu(Box::new(menu))
            .build()
            .ok()?;

        let ids = (
            panel.id().clone(),
            refresh.id().clone(),
            menu_bar_usage.id().clone(),
            alerts.id().clone(),
            always_on_top.id().clone(),
            pinned.id().clone(),
            next_display.id().clone(),
            side.id().clone(),
            add_account.id().clone(),
            open_at_login.id().clone(),
            quit.id().clone(),
        );
        let (menu_shared, menu_ctx) = (shared, ctx.clone());
        let account_ids: Arc<Mutex<Vec<(MenuId, String)>>> = Arc::default();
        let menu_account_ids = account_ids.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let (panel, refresh, menu_bar_usage, alerts, always_on_top, pinned, next_display, side, add_account, open_at_login, quit) =
                &ids;
            let shared = &menu_shared;
            let id = &event.id;
            if id == quit {
                std::process::exit(0);
            } else if id == panel {
                shared.toggle_panel(&menu_ctx);
            } else if id == refresh {
                let _ = commands.send(Command::Refresh);
            } else if id == menu_bar_usage {
                let on = !shared.menu_bar_usage.fetch_xor(true, Ordering::SeqCst);
                settings::set_flag(settings::MENU_BAR_USAGE, on);
            } else if id == alerts {
                let on = !alerts::ENABLED.fetch_xor(true, Ordering::SeqCst);
                settings::set_flag(settings::ALERTS_OFF, !on);
            } else if id == always_on_top {
                let on = !shared.always_on_top.fetch_xor(true, Ordering::SeqCst);
                settings::set_flag(settings::ALWAYS_ON_TOP_OFF, !on);
            } else if id == pinned {
                let on = !shared.pinned.fetch_xor(true, Ordering::SeqCst);
                settings::set_flag(settings::PINNED, on);
            } else if id == next_display {
                shared.move_to_next_display.store(true, Ordering::SeqCst);
            } else if id == side {
                shared.set_side(if shared.side() == Side::Left { Side::Right } else { Side::Left });
                shared.dock_requested.store(true, Ordering::SeqCst);
            } else if id == add_account {
                let _ = commands.send(Command::AddAccount);
            } else if id == open_at_login {
                login::set_enabled(!login::enabled());
            } else if let Some((_, key)) = menu_account_ids.lock().unwrap().iter().find(|(item, _)| item == id) {
                let _ = commands.send(Command::ToggleAccount(key.clone()));
            }
            menu_ctx.request_repaint();
        }));

        Some(Self {
            icon,
            headline,
            panel,
            next_display,
            side,
            accounts,
            account_items: RefCell::default(),
            account_ids,
            shown: RefCell::new(None),
        })
    }

    /// Lists every account found on this Mac in the Accounts submenu, ticked while on the panel.
    pub fn set_accounts(&self, all: &[Account], hidden: &HashSet<String>) {
        let mut items = self.account_items.borrow_mut();
        for item in items.drain(..) {
            let _ = self.accounts.remove(&item);
        }
        let mut ids = self.account_ids.lock().unwrap();
        ids.clear();
        for (at, account) in all.iter().enumerate() {
            let item = CheckMenuItem::new(&account.label, true, !hidden.contains(account.key()), None);
            let _ = self.accounts.insert(&item, at);
            ids.push((item.id().clone(), account.key().to_string()));
            items.push(item);
        }
    }

    /// `levels` holds each account's fullest limit as a fraction, `None` until it has a reading.
    pub fn update(&self, shared: &Shared, levels: &[Option<f32>], usage: String, headline: &str) {
        self.headline.set_text(headline);
        let visible = shared.visible.load(Ordering::SeqCst);
        self.panel.set_text(if visible { "Hide Panel" } else { "Show Panel" });
        self.next_display.set_enabled(window::screen_count() > 1);
        self.side.set_text(match shared.side() {
            Side::Left => "Move to Right Side",
            Side::Right => "Move to Left Side",
        });

        // Whole percents, so the icon is not rebuilt for changes it cannot show.
        let steps = levels.iter().map(|l| l.map(|l| (l.clamp(0.0, 1.0) * 100.0) as u8)).collect();
        let title = shared.menu_bar_usage.load(Ordering::SeqCst).then_some(usage);
        let next = Some((steps, title));
        if *self.shown.borrow() == next {
            return;
        }
        let _ = self.icon.set_icon(Some(gauge(levels)));
        self.icon.set_title(next.as_ref().and_then(|(_, title)| title.as_deref()));
        *self.shown.borrow_mut() = next;
    }
}

const ICON_PX: usize = 36;
const TRACK_PX: usize = 28;
/// More accounts than this share the menu bar's height badly, so the rest stay in the panel.
const MAX_BARS: usize = 4;

/// The tray icon: one bar per account, filled as far as its fullest limit, lit like the panel's
/// bars. Drawn at twice the menu bar's size, so every measure is even and halves cleanly.
fn gauge(levels: &[Option<f32>]) -> Icon {
    let placeholder = [None; 3];
    let levels = if levels.is_empty() { &placeholder[..] } else { &levels[..levels.len().min(MAX_BARS)] };
    let (bar, gap) = match levels.len() {
        1 => (10, 0),
        2 => (8, 6),
        3 => (6, 4),
        _ => (4, 4),
    };
    let left = (ICON_PX - TRACK_PX) / 2;
    let top = (ICON_PX - (levels.len() * bar + (levels.len() - 1) * gap)) / 2;
    let mut rgba = vec![0u8; ICON_PX * ICON_PX * 4];
    for (i, level) in levels.iter().enumerate() {
        let level = level.unwrap_or(0.0).clamp(0.0, 1.0);
        let filled = match (level * TRACK_PX as f32).round() as usize {
            0 if level > 0.0 => 2,
            px => px,
        };
        for x in 0..TRACK_PX {
            let color = if x >= filled {
                theme::TEXT_DIM.gamma_multiply(0.6)
            } else {
                level_color(level * 100.0).unwrap_or_else(|| theme::light(x as f32 / (TRACK_PX - 1) as f32))
            };
            let y0 = top + i * (bar + gap);
            for y in y0..y0 + bar {
                let at = (y * ICON_PX + left + x) * 4;
                rgba[at..at + 4].copy_from_slice(&color.to_srgba_unmultiplied());
            }
        }
    }
    Icon::from_rgba(rgba, ICON_PX as u32, ICON_PX as u32).expect("icon buffer matches its size")
}

/// Amber from 70%, red from 90%; below that a bar keeps the light.
pub fn level_color(percent: f32) -> Option<Color32> {
    match percent {
        p if p >= 90.0 => Some(theme::DANGER),
        p if p >= 70.0 => Some(theme::WARNING),
        _ => None,
    }
}
