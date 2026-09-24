mod accounts;
mod activity;
mod alerts;
mod api;
mod cache;
mod codex;
mod login;
mod window;

use accounts::{Account, Kind};
use activity::Activity;
use api::Limit;
use chrono::{DateTime, Local, Utc};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2, ViewportCommand};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const POLL_INTERVAL: Duration = Duration::from_secs(10 * 60);
const SCAN_INTERVAL: Duration = Duration::from_secs(10);
const WIDTH: f32 = 320.0;

#[derive(Default)]
struct AccountState {
    limits: Vec<Limit>,
    updated_at: Option<DateTime<Utc>>,
    error: Option<String>,
    checking: bool,
    activity: Option<Activity>,
}

type States = Arc<Mutex<Vec<AccountState>>>;

struct App {
    accounts: Vec<Account>,
    states: States,
    refresh_tx: mpsc::Sender<()>,
    tray: Option<TrayIcon>,
    visible: Arc<AtomicBool>,
    pinned: Arc<AtomicBool>,
    applied_pin: Option<bool>,
    always_on_top: std::cell::Cell<bool>,
    placed: bool,
    display: Option<String>,
    move_to_next_screen: std::cell::Cell<bool>,
    pin_item: CheckMenuItem,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let ctx = cc.egui_ctx.clone();
        ctx.set_visuals(egui::Visuals::dark());
        let accounts = accounts::discover();
        let cached = cache::load();
        let states: States = Arc::new(Mutex::new(
            accounts
                .iter()
                .map(|a| match cached.get(a.key()) {
                    Some(r) => AccountState {
                        limits: r.limits.clone(),
                        updated_at: Some(r.updated_at),
                        ..Default::default()
                    },
                    None => AccountState::default(),
                })
                .collect(),
        ));
        let (refresh_tx, refresh_rx) = mpsc::channel();
        spawn_poller(accounts.clone(), states.clone(), ctx.clone(), refresh_rx);
        spawn_scanner(accounts.clone(), states.clone(), ctx.clone());

        let visible = Arc::new(AtomicBool::new(true));
        let pinned = Arc::new(AtomicBool::new(window::load_pinned()));
        let pin_item = CheckMenuItem::new("Show on all desktops", true, pinned.load(Ordering::SeqCst), None);
        login::sync();
        let login_item = CheckMenuItem::new("Open at login", true, login::enabled(), None);
        let quit_item = MenuItem::new("Quit Claude Usage", true, None);
        let menu = Menu::new();
        let _ = menu.append_items(&[&pin_item, &login_item, &quit_item]);
        let tray = TrayIconBuilder::new()
            .with_title("Claude …")
            .with_tooltip("Claude usage — click to show/hide, right-click for menu")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok();

        let (pin_id, login_id, quit_id) = (
            pin_item.id().clone(),
            login_item.id().clone(),
            quit_item.id().clone(),
        );
        let menu_pinned = pinned.clone();
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == quit_id {
                std::process::exit(0);
            } else if event.id == login_id {
                login::set_enabled(!login::enabled());
            } else if event.id == pin_id {
                menu_pinned.fetch_xor(true, Ordering::SeqCst);
                menu_ctx.request_repaint();
            }
        }));
        let tray_visible = visible.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let show = !tray_visible.fetch_xor(true, Ordering::SeqCst);
                ctx.send_viewport_cmd(ViewportCommand::Visible(show));
                if show {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                ctx.request_repaint();
            }
        }));

        Self {
            accounts,
            states,
            refresh_tx,
            tray,
            visible,
            pinned,
            applied_pin: None,
            always_on_top: std::cell::Cell::new(window::load_always_on_top()),
            placed: false,
            display: None,
            move_to_next_screen: std::cell::Cell::new(false),
            pin_item,
        }
    }

    fn update_tray(&self, states: &[AccountState]) {
        let Some(tray) = &self.tray else { return };
        let pct = |s: &AccountState, primary_session: bool| {
            s.limits
                .iter()
                .filter(|l| l.primary)
                .find(|l| (l.window_secs < 86400) == primary_session)
                .map(|l| format!("{:.0}", l.percent))
                .unwrap_or_else(|| "–".into())
        };
        let title = self
            .accounts
            .iter()
            .zip(states)
            .map(|(a, s)| format!("{} {}·{}", a.tag, pct(s, true), pct(s, false)))
            .collect::<Vec<_>>()
            .join("  ");
        tray.set_title(Some(title));
    }
}

fn spawn_poller(accounts: Vec<Account>, states: States, ctx: egui::Context, refresh_rx: mpsc::Receiver<()>) {
    std::thread::spawn(move || {
        let mut alerts = alerts::Alerts::default();
        loop {
            for (i, account) in accounts.iter().enumerate() {
                let Kind::Claude { keychain_service } = &account.kind else { continue };
                states.lock().unwrap()[i].checking = true;
                ctx.request_repaint();
                let result = poll(keychain_service);
                let mut states = states.lock().unwrap();
                let state = &mut states[i];
                match result {
                    Ok(limits) => {
                        alerts.check(&account.label, &limits);
                        let now = Utc::now();
                        cache::save(
                            account.key(),
                            cache::Reading { limits: limits.clone(), updated_at: now },
                        );
                        state.limits = limits;
                        state.updated_at = Some(now);
                        state.error = None;
                    }
                    Err(e) => state.error = Some(e),
                }
                state.checking = false;
                ctx.request_repaint();
            }
            match refresh_rx.recv_timeout(POLL_INTERVAL) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

enum Scanner {
    Claude(activity::Scanner),
    Codex(codex::Scanner),
}

fn spawn_scanner(accounts: Vec<Account>, states: States, ctx: egui::Context) {
    std::thread::spawn(move || {
        let mut scanners: Vec<_> = accounts
            .iter()
            .map(|a| match a.kind {
                Kind::Claude { .. } => Scanner::Claude(activity::Scanner::new(&a.config_dir)),
                Kind::Codex => Scanner::Codex(codex::Scanner::new(&a.config_dir)),
            })
            .collect();
        // Codex limits come from local logs, so its alerts are raised here, not in the poller.
        let mut alerts = alerts::Alerts::default();
        loop {
            for (i, scanner) in scanners.iter_mut().enumerate() {
                match scanner {
                    Scanner::Claude(s) => {
                        let activity = s.scan();
                        states.lock().unwrap()[i].activity = Some(activity);
                    }
                    Scanner::Codex(s) => {
                        let (activity, snapshot) = s.scan();
                        if let Some(snap) = &snapshot {
                            alerts.check(&accounts[i].label, &snap.limits);
                        }
                        let mut states = states.lock().unwrap();
                        let state = &mut states[i];
                        state.activity = Some(activity);
                        match snapshot {
                            Some(snap) => {
                                state.limits = snap.limits;
                                state.updated_at = Some(snap.at);
                                state.error = None;
                            }
                            None => state.error = Some("no Codex usage recorded yet".into()),
                        }
                    }
                }
            }
            ctx.request_repaint();
            std::thread::sleep(SCAN_INTERVAL);
        }
    });
}

/// Reads the token fresh each time so we pick up refreshes done by Claude Code itself.
fn poll(keychain_service: &str) -> Result<Vec<Limit>, String> {
    let token = accounts::read_token(keychain_service)
        .ok_or("no credentials in Keychain")?;
    if token
        .expires_at_ms
        .is_some_and(|exp| exp <= Utc::now().timestamp_millis())
    {
        return Err("token expired — start a claude session on this account, then ↻".into());
    }
    api::fetch(&token.access_token)
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_secs(1));

        if !self.placed {
            window::dock_left(frame, WIDTH);
            self.placed = true;
        }
        if self.move_to_next_screen.take() {
            window::dock_next_screen(frame, WIDTH);
        }
        // Remember the display, whether moved by the button or dragged there.
        let display = window::current_display(frame);
        if display.is_some() && display != self.display {
            if let Some(name) = &display {
                window::save_display(name);
            }
            self.display = display;
        }

        let pinned = self.pinned.load(Ordering::SeqCst);
        if self.applied_pin != Some(pinned) {
            window::set_pinned(frame, pinned);
            window::save_pinned(pinned);
            self.pin_item.set_checked(pinned);
            self.applied_pin = Some(pinned);
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.hide(ctx);
        }

        let states = self.states.lock().unwrap();
        self.update_tray(&states);

        let frame = egui::Frame::central_panel(&ctx.style())
            .fill(Color32::from_rgba_unmultiplied(4, 12, 28, 230))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .stroke(Stroke::new(1.0_f32, Color32::from_gray(60)));
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            self.title_bar(ui);
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.accounts.is_empty() {
                    ui.label("No Claude Code accounts found in Keychain.");
                }
                for (i, (account, state)) in self.accounts.iter().zip(states.iter()).enumerate() {
                    ui.add_space(16.0);
                    draw_account(ui, account, state, ACCENTS[i % ACCENTS.len()]);
                }
            });
        });
    }
}

impl App {
    fn hide(&self, ctx: &egui::Context) {
        self.visible.store(false, Ordering::SeqCst);
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    /// Drag strip replacing the native title bar.
    fn title_bar(&self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::click_and_drag());
        if response.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(
            egui::Layout::left_to_right(egui::Align::Center),
        ));
        child.label(RichText::new("AI USAGE").weak().size(9.5));
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, Icon::Close, false)
                .on_hover_text("Hide — click the menu bar item to show, right-click it to quit")
                .clicked()
            {
                self.hide(ui.ctx());
            }
            if window::screen_count() > 1
                && icon_button(ui, Icon::NextDisplay, false)
                    .on_hover_text("Move to next display")
                    .clicked()
            {
                self.move_to_next_screen.set(true);
                ui.ctx().request_repaint();
            }
            let pinned = self.pinned.load(Ordering::SeqCst);
            let tip = if pinned {
                "Pinned: shown on every desktop (click to unpin)"
            } else {
                "Pin: show on every desktop"
            };
            if icon_button(ui, Icon::Pin, pinned).on_hover_text(tip).clicked() {
                self.pinned.store(!pinned, Ordering::SeqCst);
            }
            let on_top = self.always_on_top.get();
            let tip = if on_top {
                "Always on top: on (click to allow other windows above)"
            } else {
                "Always on top: off (click to stay above other windows)"
            };
            if icon_button(ui, Icon::AlwaysOnTop, on_top).on_hover_text(tip).clicked() {
                self.always_on_top.set(!on_top);
                window::save_always_on_top(!on_top);
                ui.ctx().send_viewport_cmd(ViewportCommand::WindowLevel(window_level(!on_top)));
                ui.ctx().request_repaint();
            }
            if ui
                .add(egui::Button::new(RichText::new("↻").size(11.0)).frame(false))
                .on_hover_text("Fetch now (auto every 10 min)")
                .clicked()
            {
                let _ = self.refresh_tx.send(());
            }
        });
    }
}

enum Icon {
    Close,
    Pin,
    NextDisplay,
    AlwaysOnTop,
}

/// Icons are painted because the bundled font lacks these glyphs.
fn icon_button(ui: &mut egui::Ui, icon: Icon, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::selected(
        egui::WidgetType::Button, ui.is_enabled(), active,
        match icon {
            Icon::Close => "Hide",
            Icon::Pin => "Show on every desktop",
            Icon::NextDisplay => "Move to next display",
            Icon::AlwaysOnTop => "Always on top",
        },
    ));
    let color = if response.is_pointer_button_down_on() {
        Color32::WHITE
    } else if response.hovered() || response.has_focus() {
        Color32::from_gray(230)
    } else if active {
        Color32::from_rgb(90, 190, 120)
    } else {
        Color32::from_gray(150)
    };
    let stroke = Stroke::new(1.4_f32, color);
    let painter = ui.painter();
    let c = rect.center();
    if response.has_focus() {
        painter.rect_stroke(rect.expand(2.0), 2.0, Stroke::new(2.0_f32, color), egui::StrokeKind::Outside);
    }
    match icon {
        Icon::AlwaysOnTop => {
            let back = egui::Rect::from_center_size(c + Vec2::new(-2.0, 2.0), Vec2::splat(8.0));
            let front = egui::Rect::from_center_size(c + Vec2::new(2.0, -2.0), Vec2::splat(8.0));
            painter.line_segment([back.left_top(), back.left_bottom()], stroke);
            painter.line_segment([back.left_bottom(), back.right_bottom()], stroke);
            if active {
                painter.rect_filled(front, 0.0, color);
            } else {
                painter.rect_stroke(front, 0.0, stroke, egui::StrokeKind::Middle);
            }
        }
        Icon::Close => {
            let d = 3.5;
            painter.line_segment([c + Vec2::new(-d, -d), c + Vec2::new(d, d)], stroke);
            painter.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
        }
        Icon::NextDisplay => {
            // A small monitor with an arrow pointing right.
            let screen = egui::Rect::from_center_size(c + Vec2::new(-1.5, -1.0), Vec2::new(9.0, 7.0));
            painter.rect_stroke(screen, 0.0, stroke, egui::StrokeKind::Middle);
            painter.line_segment([c + Vec2::new(-3.5, 4.5), c + Vec2::new(0.5, 4.5)], stroke);
            let tip = c + Vec2::new(6.5, -1.0);
            painter.line_segment([c + Vec2::new(1.5, -1.0), tip], stroke);
            painter.line_segment([tip, tip + Vec2::new(-2.5, -2.5)], stroke);
            painter.line_segment([tip, tip + Vec2::new(-2.5, 2.5)], stroke);
        }
        Icon::Pin => {
            let head = c + Vec2::new(0.0, -2.0);
            if active {
                painter.circle_filled(head, 3.2, color);
            } else {
                painter.circle_stroke(head, 3.0, stroke);
            }
            painter.line_segment([c + Vec2::new(0.0, 1.2), c + Vec2::new(0.0, 6.0)], stroke);
        }
    }
    response
}

const CARD: Color32 = Color32::from_gray(30);
const MUTED: Color32 = Color32::from_gray(140);
/// One colour per account so the cards are easy to tell apart.
const ACCENTS: [Color32; 4] = [
    Color32::from_rgb(110, 150, 235),
    Color32::from_rgb(190, 130, 235),
    Color32::from_rgb(80, 190, 180),
    Color32::from_rgb(230, 160, 90),
];

fn draw_account(ui: &mut egui::Ui, account: &Account, state: &AccountState, accent: Color32) {
    let card = egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, accent.gamma_multiply(0.35)))
        .inner_margin(egui::Margin { left: 16, right: 12, top: 12, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            let (name, plan) = account
                .label
                .split_once(" · ")
                .unwrap_or((account.label.as_str(), ""));
            ui.horizontal(|ui| {
                ui.scope(|ui| {
                    ui.set_max_width(ui.available_width() - 80.0);
                    ui.add(egui::Label::new(RichText::new(name).strong().size(14.0).color(accent)).truncate())
                        .on_hover_text(name);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !plan.is_empty() {
                        egui::Frame::new()
                            .fill(Color32::from_gray(50))
                            .inner_margin(egui::Margin::symmetric(5, 1))
                            .show(ui, |ui| {
                                ui.label(RichText::new(plan.to_uppercase()).size(9.5).color(MUTED));
                            });
                    }
                    if state.checking {
                        ui.add(egui::Spinner::new().size(11.0));
                    }
                });
            });

            if let Some(a) = &state.activity {
                draw_activity(ui, a);
            }
            ui.add_space(6.0);

            if state.limits.is_empty() && state.error.is_none() {
                ui.label(RichText::new("loading…").color(MUTED).size(12.0));
            }
            for limit in &state.limits {
                draw_limit(ui, limit);
            }
            if let Some(err) = &state.error {
                ui.label(RichText::new(err).color(Color32::from_rgb(230, 120, 90)).size(11.5));
            }

            if let Some(a) = &state.activity {
                ui.add_space(4.0);
                draw_hourly(ui, a, accent);
            }

            if let Some(t) = state.updated_at {
                let ago = (Utc::now() - t).num_seconds().max(0);
                let text = match account.kind {
                    Kind::Codex => format!("limits as of last Codex reply, {}", ago_text(ago)),
                    Kind::Claude { .. } => format!("updated {}", ago_text(ago)),
                };
                ui.label(RichText::new(text).color(Color32::from_gray(100)).size(10.0));
            }
        });
    let r = card.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_max(r.left_top(), egui::pos2(r.left() + 4.0, r.bottom())),
        0.0,
        accent,
    );
}

fn ago_text(secs: i64) -> String {
    match secs {
        s if s < 60 => format!("{s}s ago"),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86400 => format!("{}h {}m ago", s / 3600, s % 3600 / 60),
        s => format!("{}d ago", s / 86400),
    }
}

fn draw_activity(ui: &mut egui::Ui, a: &Activity) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        let label = if a.active_sessions > 0 {
            ui.painter().circle_filled(rect.center(), 4.0, Color32::from_rgb(90, 190, 120));
            format!("{} active session{}", a.active_sessions, if a.active_sessions == 1 { "" } else { "s" })
        } else {
            ui.painter().circle_stroke(rect.center(), 3.5, Stroke::new(1.0_f32, Color32::from_gray(110)));
            "idle".to_string()
        };
        ui.label(RichText::new(label).color(MUTED).size(12.0));
    });

    ui.add_space(4.0);
    ui.columns(3, |cols| {
        stat(&mut cols[0], "LAST HOUR", &tokens(a.hour_tokens), "Tokens in the last hour");
        stat(&mut cols[1], "TODAY", &tokens(a.today_tokens), "Tokens since midnight");
        stat(&mut cols[2], "SESSIONS", &a.sessions_today.to_string(), "Sessions active today");
    });
}

fn stat(ui: &mut egui::Ui, label: &str, value: &str, hover: &str) {
    ui.spacing_mut().item_spacing.y = 1.0;
    ui.label(RichText::new(label).size(9.5).color(MUTED));
    ui.label(RichText::new(value).size(18.0).strong())
        .on_hover_text(format!("{hover}, including cache reads · from local transcripts on this Mac"));
}

fn draw_limit(ui: &mut egui::Ui, limit: &Limit) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(&limit.name).size(12.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let color = if limit.percent >= 70.0 { bar_color(limit.percent) } else { Color32::WHITE };
            ui.label(RichText::new(format!("{:.0}%", limit.percent)).strong().size(18.0).color(color));
        });
    });

    let pace = pace(limit);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, Color32::from_gray(50));
    let frac = (limit.percent / 100.0).clamp(0.0, 1.0);
    let mut filled = rect;
    filled.set_width(rect.width() * frac);
    painter.rect_filled(filled, 0.0, bar_color(limit.percent));
    if let Some(p) = &pace {
        let x = rect.left() + rect.width() * p.elapsed;
        painter.line_segment(
            [egui::pos2(x, rect.top() - 3.0), egui::pos2(x, rect.bottom() + 3.0)],
            Stroke::new(2.0_f32, Color32::from_gray(225)),
        );
    }

    ui.horizontal(|ui| {
        if let Some(t) = limit.resets_at {
            ui.label(RichText::new(format!("resets in {}", countdown(t))).color(MUTED).size(10.5));
        }
        if let Some(Pace { text: Some((text, warn)), .. }) = pace {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let color = if warn { Color32::from_rgb(235, 150, 90) } else { MUTED };
                ui.label(RichText::new(text).color(color).size(10.5))
                    .on_hover_text("Average pace since the window started, projected to reset. The white tick marks elapsed time.");
            });
        }
    });
    ui.add_space(6.0);
}

fn draw_hourly(ui: &mut egui::Ui, a: &Activity, accent: Color32) {
    ui.label(RichText::new("TOKENS PER HOUR").size(9.5).color(MUTED));
    let max = a.hourly.iter().copied().max().unwrap_or(0);
    if max == 0 {
        ui.label(RichText::new(format!("no usage in the last {}h", a.hourly.len())).color(Color32::from_gray(100)).size(11.0));
        return;
    }
    let height = 56.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0_f32, Color32::from_gray(60)),
    );
    let slot = rect.width() / a.hourly.len() as f32;
    for (i, &count) in a.hourly.iter().enumerate() {
        let h = (count as f64 / max as f64) as f32 * (height - 4.0);
        let x = rect.left() + slot * i as f32;
        let bar = egui::Rect::from_min_max(
            egui::pos2(x + 1.5, rect.bottom() - h.max(if count > 0 { 1.0 } else { 0.0 })),
            egui::pos2(x + slot - 1.5, rect.bottom()),
        );
        let current = i == a.hourly.len() - 1;
        painter.rect_filled(bar, 0.0, if current { accent } else { accent.gamma_multiply(0.6) });
        let hover = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(x + slot, rect.bottom()));
        let hour = a.hourly_start + chrono::Duration::hours(i as i64);
        ui.interact(hover, ui.id().with(("hour", i)), Sense::hover())
            .on_hover_text(format!("{} · {} tokens", hour.format("%H:00"), tokens(count)));
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(a.hourly_start.format("%H:00").to_string()).size(9.5).color(Color32::from_gray(100)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("now · peak {}/h", tokens(max))).size(9.5).color(Color32::from_gray(100)));
        });
    });
}

struct Pace {
    /// Fraction of the window elapsed (position of the tick on the bar).
    elapsed: f32,
    /// Projection text and whether it predicts hitting the limit before reset.
    text: Option<(String, bool)>,
}

/// Projects the average rate since the window started forward to the reset time.
fn pace(limit: &Limit) -> Option<Pace> {
    let resets_at = limit.resets_at?;
    let window = limit.window_secs as f64;
    let remaining = (resets_at - Utc::now()).num_seconds().clamp(0, limit.window_secs) as f64;
    let elapsed = window - remaining;
    let frac = (elapsed / window) as f32;
    let percent = limit.percent as f64;

    let text = if percent >= 100.0 {
        Some(("limit reached".to_string(), true))
    } else if elapsed < window * 0.02 || percent <= 0.0 {
        None
    } else {
        let rate = percent / elapsed;
        let projected = percent + rate * remaining;
        if projected >= 100.0 {
            let secs = ((100.0 - percent) / rate) as i64;
            let at = Utc::now() + chrono::Duration::seconds(secs);
            let when = if secs < 86400 {
                format!("in {}", countdown(at))
            } else {
                at.with_timezone(&Local).format("%a %H:%M").to_string()
            };
            Some((format!("at this pace: 100% {when}"), true))
        } else {
            Some((format!("at this pace: ~{projected:.0}% at reset"), false))
        }
    };
    Some(Pace { elapsed: frac, text })
}

fn bar_color(percent: f32) -> Color32 {
    match percent {
        p if p >= 90.0 => Color32::from_rgb(230, 80, 70),
        p if p >= 70.0 => Color32::from_rgb(235, 170, 60),
        _ => Color32::from_rgb(90, 190, 120),
    }
}

fn tokens(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.2}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.0}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

fn countdown(t: DateTime<Utc>) -> String {
    let secs = (t - Utc::now()).num_seconds().max(0);
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

fn window_level(on_top: bool) -> egui::WindowLevel {
    if on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal }
}

fn main() -> eframe::Result {
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Claude Usage")
            .with_inner_size([WIDTH, 300.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(window_level(window::load_always_on_top())),
        // Menu bar app: no Dock icon, no Cmd-Tab entry.
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_activation_policy(ActivationPolicy::Accessory);
        })),
        ..Default::default()
    };
    eframe::run_native(
        "Claude Usage",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
