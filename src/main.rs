mod accounts;
mod activity;
mod alerts;
mod api;
mod cache;
mod codex;
mod login;
mod settings;
mod theme;
mod tray;
mod window;

use accounts::{Account, Kind};
use activity::Activity;
use api::Limit;
use chrono::{DateTime, Local, Utc};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2, ViewportCommand};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tray::{Shared, Tray};

const POLL_INTERVAL: Duration = Duration::from_secs(10 * 60);
const SCAN_INTERVAL: Duration = Duration::from_secs(10);
const WIDTH: f32 = 320.0;
/// Space between the panel's edge and its content.
const PADDING: f32 = 12.0;
/// Height of an hourly chart when the window has no room to spare.
const CHART_MIN_HEIGHT: f32 = 24.0;
const MIN_ZOOM: f32 = 0.5;
/// Widest a plan badge gets before its text is cut.
const PLAN_MAX_WIDTH: f32 = 96.0;

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
    tray: Option<Tray>,
    shared: Arc<Shared>,
    applied_pin: Option<bool>,
    applied_always_on_top: Option<bool>,
    placed: bool,
    display: Option<String>,
    /// Height added to every hourly chart so the content fills the window.
    chart_extra: std::cell::Cell<f32>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let ctx = cc.egui_ctx.clone();
        theme::install(&ctx);
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
        alerts::ENABLED.store(!settings::flag(settings::ALERTS_OFF), Ordering::SeqCst);
        spawn_poller(accounts.clone(), states.clone(), ctx.clone(), refresh_rx);
        spawn_scanner(accounts.clone(), states.clone(), ctx.clone());
        login::sync();

        let shared = Arc::new(Shared::load());
        let tray = Tray::new(&ctx, shared.clone(), refresh_tx.clone());

        Self {
            accounts,
            states,
            refresh_tx,
            tray,
            shared,
            applied_pin: None,
            applied_always_on_top: None,
            placed: false,
            display: None,
            chart_extra: std::cell::Cell::new(0.0),
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
        let usage = self
            .accounts
            .iter()
            .zip(states)
            .map(|(a, s)| format!("{} {}·{}", a.tag, pct(s, true), pct(s, false)))
            .collect::<Vec<_>>()
            .join("  ");
        let levels: Vec<_> = states
            .iter()
            .map(|s| s.limits.iter().map(|l| l.percent / 100.0).reduce(f32::max))
            .collect();
        let active = active_sessions(states);
        let headline = format!(
            "{} · {}",
            plural(self.accounts.len(), "account"),
            if active > 0 { plural(active, "active session") } else { "idle".into() },
        );
        tray.update(&self.shared, &levels, usage, &headline);
    }
}

fn active_sessions(states: &[AccountState]) -> usize {
    states.iter().filter_map(|s| s.activity.as_ref()).map(|a| a.active_sessions).sum()
}

fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
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
        return Err("token expired. start a claude session on this account, then refresh".into());
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
        if self.shared.move_to_next_display.swap(false, Ordering::SeqCst) {
            window::dock_next_screen(frame, WIDTH);
        }
        window::keep_fitted(frame, WIDTH);
        // Remember the display, whether moved from the menu or dragged there.
        let display = window::current_display(frame);
        if display.is_some() && display != self.display {
            if let Some(name) = &display {
                settings::save_display(name);
            }
            self.display = display;
        }

        let pinned = self.shared.pinned.load(Ordering::SeqCst);
        if self.applied_pin != Some(pinned) {
            window::set_pinned(frame, pinned);
            self.applied_pin = Some(pinned);
        }
        let always_on_top = self.shared.always_on_top.load(Ordering::SeqCst);
        if self.applied_always_on_top != Some(always_on_top) {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(window_level(always_on_top)));
            self.applied_always_on_top = Some(always_on_top);
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.hide(ctx);
        }

        let states = self.states.lock().unwrap();
        self.update_tray(&states);

        // The frame is painted over the panel's outermost pixel, so the content starts inside it.
        let panel = egui::Frame::new()
            .fill(theme::STATUS)
            .inner_margin(egui::Margin::symmetric(PADDING as i8, 1));
        egui::CentralPanel::default().frame(panel).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let (top, available) = (ui.cursor().top(), ui.available_height() - PADDING);
            self.header(ui, active_sessions(&states));
            if self.accounts.is_empty() {
                ui.add_space(PADDING);
                ui.label(RichText::new("no claude code or codex accounts found").size(12.0).color(theme::TEXT_MUTED));
            }
            let chart_height = CHART_MIN_HEIGHT + self.chart_extra.get();
            let mut charts = 0;
            for (i, (account, state)) in self.accounts.iter().zip(states.iter()).enumerate() {
                if i > 0 {
                    rule(ui);
                }
                ui.add_space(PADDING);
                if draw_account(ui, i, account, state, chart_height) {
                    charts += 1;
                }
                ui.add_space(PADDING - ui.spacing().item_spacing.y);
            }
            self.fit_content(ui.ctx(), available, ui.cursor().top() - top, charts);
            let screen = ui.ctx().content_rect();
            theme::frame(&ui.painter().with_clip_rect(screen), screen);
        });
    }
}

/// A hairline across the whole panel, edge to edge.
fn rule(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    let screen = ui.ctx().content_rect();
    ui.painter()
        .with_clip_rect(screen)
        .hline(screen.x_range(), rect.center().y, Stroke::new(1.0_f32, theme::hairline()));
}

impl App {
    fn hide(&self, ctx: &egui::Context) {
        self.shared.visible.store(false, Ordering::SeqCst);
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    /// Fits the content to the window instead of scrolling: spare height goes to the hourly
    /// charts, and when it overflows even with the shortest charts, the whole UI zooms out.
    fn fit_content(&self, ctx: &egui::Context, available: f32, used: f32, charts: usize) {
        let extra = self.chart_extra.get();
        let content = used - extra * charts as f32;
        if available < 1.0 || content < 1.0 {
            return;
        }
        let zoom = ctx.zoom_factor();
        let fitted_zoom = (zoom * available / content).clamp(MIN_ZOOM, 1.0);
        let spare = available * zoom / fitted_zoom - content;
        let fitted_extra = if charts > 0 { (spare / charts as f32).max(0.0) } else { 0.0 };
        if (fitted_zoom - zoom).abs() > 0.005 {
            ctx.set_zoom_factor(fitted_zoom);
            ctx.request_repaint();
        }
        if (fitted_extra - extra).abs() > 0.5 {
            self.chart_extra.set(fitted_extra);
            ctx.request_repaint();
        }
    }

    /// The panel's top row, which is also the strip it is dragged by: the gauge, what the
    /// accounts are doing, then refresh and hide.
    fn header(&self, ui: &mut egui::Ui, active: usize) {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click_and_drag());
        if response.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let mut row = ui.new_child(
            egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        row.spacing_mut().item_spacing.x = 8.0;
        let (mark, _) = row.allocate_exact_size(Vec2::new(14.0, 12.0), Sense::hover());
        for (i, level) in [1.0, 0.65, 0.35].into_iter().enumerate() {
            let y = mark.top() + 1.0 + i as f32 * 4.0;
            let bar = Rect::from_min_size(Pos2::new(mark.left(), y), Vec2::new(mark.width() * level, 2.0));
            theme::light_across(row.painter(), bar, theme::ACCENT, theme::light(level));
        }
        row.label(caps("usage", theme::TEXT_DIM));
        let doing = if active > 0 { format!("{active} active") } else { "idle".into() };
        row.label(theme::lit_caps(&doing, 11.0));
        row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if icon_button(ui, Icon::Hide)
                .on_hover_text("hide. click the menu bar gauge to show it again")
                .clicked()
            {
                self.hide(ui.ctx());
            }
            if icon_button(ui, Icon::Refresh).on_hover_text("fetch now. auto every 10 min").clicked() {
                let _ = self.refresh_tx.send(());
            }
        });
        rule(ui);
    }
}

enum Icon {
    Hide,
    Refresh,
}

/// Icons are painted because the font has no glyphs for them.
fn icon_button(ui: &mut egui::Ui, icon: Icon) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            match icon {
                Icon::Hide => "Hide",
                Icon::Refresh => "Fetch now",
            },
        )
    });
    let color = if response.is_pointer_button_down_on() {
        theme::ACCENT
    } else if response.hovered() || response.has_focus() {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    let stroke = Stroke::new(1.2_f32, color);
    let painter = ui.painter();
    let c = rect.center();
    if response.has_focus() {
        painter.rect_stroke(rect.expand(2.0), 2.0, Stroke::new(1.0_f32, theme::ACCENT), egui::StrokeKind::Outside);
    }
    match icon {
        Icon::Hide => {
            let d = 3.5;
            painter.line_segment([c + Vec2::new(-d, -d), c + Vec2::new(d, d)], stroke);
            painter.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
        }
        Icon::Refresh => {
            // Three quarters of a circle, ending in an arrow head at the top.
            let radius = 4.5;
            let at = |turn: f32| {
                let angle = turn * std::f32::consts::TAU;
                c + Vec2::new(angle.sin(), -angle.cos()) * radius
            };
            let arc: Vec<Pos2> = (0..=18).map(|i| at(0.2 + 0.8 * i as f32 / 18.0)).collect();
            painter.add(egui::Shape::line(arc, stroke));
            let tip = at(1.0);
            painter.line_segment([tip, tip + Vec2::new(-3.0, -2.5)], stroke);
            painter.line_segment([tip, tip + Vec2::new(-3.0, 2.5)], stroke);
        }
    }
    response
}

/// 11px uppercase with wide tracking: the panel's section voice.
fn caps(text: &str, color: Color32) -> RichText {
    RichText::new(text.to_uppercase())
        .font(theme::semibold(11.0))
        .extra_letter_spacing(theme::TRACKING)
        .color(color)
}

fn small(text: impl Into<String>, color: Color32) -> RichText {
    RichText::new(text).size(11.0).color(color)
}

/// Returns whether the account includes an hourly chart.
fn draw_account(
    ui: &mut egui::Ui,
    index: usize,
    account: &Account,
    state: &AccountState,
    chart_height: f32,
) -> bool {
    ui.spacing_mut().item_spacing.y = 4.0;
    let (name, plan) = account
        .label
        .split_once(" · ")
        .unwrap_or((account.label.as_str(), ""));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(small(format!("{:02}", index + 1), theme::TEXT_DIM));
        let (square, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
        ui.painter().rect_filled(square, 0.0, theme::ACCOUNT_HUES[index % theme::ACCOUNT_HUES.len()]);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !plan.is_empty() {
                egui::Frame::new()
                    .fill(theme::SURFACE_RAISED)
                    .corner_radius(2)
                    .inner_margin(egui::Margin::symmetric(4, 1))
                    .show(ui, |ui| {
                        // A long plan name gives way to the account's.
                        ui.set_max_width(PLAN_MAX_WIDTH);
                        let plan = plan.to_lowercase().replace('_', " ");
                        ui.add(egui::Label::new(small(&plan, theme::TEXT_MUTED)).truncate()).on_hover_text(plan);
                    });
            }
            if state.checking {
                ui.add(egui::Spinner::new().size(10.0).color(theme::ACCENT));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(name).font(theme::semibold(12.0)).color(theme::TEXT)).truncate())
                    .on_hover_text(name);
            });
        });
    });

    if let Some(a) = &state.activity {
        draw_activity(ui, a);
    }
    ui.add_space(8.0);

    if state.limits.is_empty() && state.error.is_none() {
        ui.label(small("loading", theme::TEXT_MUTED));
    }
    for limit in &state.limits {
        draw_limit(ui, limit);
    }
    if let Some(err) = &state.error {
        ui.label(small(err, theme::DANGER));
    }

    let mut has_chart = false;
    if let Some(a) = &state.activity {
        ui.add_space(4.0);
        has_chart = draw_hourly(ui, a, chart_height);
    }

    if let Some(t) = state.updated_at {
        let ago = (Utc::now() - t).num_seconds().max(0);
        let text = match account.kind {
            Kind::Codex => format!("limits as of last codex reply, {}", ago_text(ago)),
            Kind::Claude { .. } => format!("updated {}", ago_text(ago)),
        };
        ui.label(small(text, theme::TEXT_DIM));
    }
    has_chart
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
        ui.spacing_mut().item_spacing.x = 8.0;
        // Lines up under the account square.
        ui.add_space(22.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
        let label = if a.active_sessions > 0 {
            ui.painter().circle_filled(rect.center(), 3.0, theme::ACCENT);
            plural(a.active_sessions, "active session")
        } else {
            ui.painter().circle_stroke(rect.center(), 2.5, Stroke::new(1.0_f32, theme::TEXT_DIM));
            "idle".to_string()
        };
        ui.label(small(label, theme::TEXT_MUTED));
    });

    ui.add_space(8.0);
    ui.columns(3, |cols| {
        stat(&mut cols[0], "last hour", &tokens(a.hour_tokens), "tokens in the last hour");
        stat(&mut cols[1], "today", &tokens(a.today_tokens), "tokens since midnight");
        stat(&mut cols[2], "sessions", &a.sessions_today.to_string(), "sessions active today");
    });
}

fn stat(ui: &mut egui::Ui, label: &str, value: &str, hover: &str) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.label(small(label, theme::TEXT_DIM));
    ui.label(RichText::new(value).font(theme::semibold(15.0)).color(theme::TEXT))
        .on_hover_text(format!("{hover}, cache reads included. from local transcripts on this mac"));
}

fn draw_limit(ui: &mut egui::Ui, limit: &Limit) {
    let level = tray::level_color(limit.percent);
    ui.horizontal(|ui| {
        ui.label(RichText::new(limit.name.to_lowercase()).size(12.0).color(theme::TEXT_MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{:.0}%", limit.percent))
                    .font(theme::semibold(15.0))
                    .color(level.unwrap_or(theme::TEXT)),
            );
        });
    });

    let pace = pace(limit);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, theme::SURFACE_RAISED);
    let frac = (limit.percent / 100.0).clamp(0.0, 1.0);
    let mut filled = rect;
    filled.set_width(rect.width() * frac);
    match level {
        Some(color) => {
            painter.rect_filled(filled, 0.0, color);
        }
        // The light spans the whole track, and a shorter fill shows only its near end.
        None => theme::light_across(painter, filled, theme::ACCENT, theme::light(frac)),
    }
    if let Some(p) = &pace {
        let x = rect.left() + rect.width() * p.elapsed;
        painter.line_segment(
            [egui::pos2(x, rect.top() - 3.0), egui::pos2(x, rect.bottom() + 3.0)],
            Stroke::new(2.0_f32, theme::TEXT),
        );
    }

    ui.horizontal(|ui| {
        if let Some(t) = limit.resets_at {
            ui.label(small(format!("resets in {}", countdown(t)), theme::TEXT_DIM));
        }
        if let Some(Pace { text: Some((text, warn)), .. }) = pace {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let color = if warn { theme::WARNING } else { theme::TEXT_MUTED };
                ui.label(small(text, color))
                    .on_hover_text("average pace since the window started, projected to its reset. the tick marks elapsed time");
            });
        }
    });
    ui.add_space(4.0);
}

/// Returns whether a chart was drawn, i.e. there was usage to plot.
fn draw_hourly(ui: &mut egui::Ui, a: &Activity, height: f32) -> bool {
    ui.label(small("tokens per hour", theme::TEXT_DIM));
    let max = a.hourly.iter().copied().max().unwrap_or(0);
    if max == 0 {
        ui.label(small(format!("no usage in the last {}h", a.hourly.len()), theme::TEXT_DIM));
        return false;
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0_f32, theme::BORDER));
    let slot = rect.width() / a.hourly.len() as f32;
    for (i, &count) in a.hourly.iter().enumerate() {
        let level = (count as f64 / max as f64) as f32;
        let h = (level * (height - 4.0)).max(if count > 0 { 1.0 } else { 0.0 });
        let x = rect.left() + slot * i as f32;
        let bar = Rect::from_min_max(egui::pos2(x + 1.0, rect.bottom() - h), egui::pos2(x + slot - 1.0, rect.bottom()));
        // Past hours sit back; the hour in progress is lit in full.
        let dim = if i == a.hourly.len() - 1 { 1.0 } else { 0.55 };
        theme::light_up(
            painter,
            bar,
            theme::ACCENT.gamma_multiply(dim),
            theme::light(level).gamma_multiply(dim),
        );
        let hover = Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(x + slot, rect.bottom()));
        let hour = a.hourly_start + chrono::Duration::hours(i as i64);
        ui.interact(hover, ui.id().with(("hour", i)), Sense::hover())
            .on_hover_text(format!("{} · {} tokens", hour.format("%H:00"), tokens(count)));
    }
    ui.horizontal(|ui| {
        ui.label(small(a.hourly_start.format("%H:00").to_string(), theme::TEXT_DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(small(format!("now · peak {}/h", tokens(max)), theme::TEXT_DIM));
        });
    });
    true
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
            Some((format!("pace: 100% {when}"), true))
        } else {
            Some((format!("pace: ~{projected:.0}% at reset"), false))
        }
    };
    Some(Pace { elapsed: frac, text })
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

    settings::migrate();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Usage Flow")
            .with_inner_size([WIDTH, 300.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(window_level(!settings::flag(settings::ALWAYS_ON_TOP_OFF))),
        // Menu bar app: no Dock icon, no Cmd-Tab entry.
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_activation_policy(ActivationPolicy::Accessory);
        })),
        ..Default::default()
    };
    eframe::run_native(
        "Usage Flow",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
