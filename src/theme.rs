//! The onflow.studio look, as written down in DESIGN.md: black panel, JetBrains Mono, and one
//! light running from cyan to periwinkle.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Color32, FontFamily, FontId, Pos2, Rect, Stroke};
use std::sync::Arc;

pub const STATUS: Color32 = Color32::BLACK;
pub const SURFACE_RAISED: Color32 = Color32::from_rgb(0x11, 0x20, 0x2A);
pub const SURFACE_TOP: Color32 = Color32::from_rgb(0x17, 0x2A, 0x36);
pub const BORDER: Color32 = Color32::from_rgb(0x13, 0x26, 0x31);
pub const TEXT: Color32 = Color32::from_rgb(0xE6, 0xFB, 0xFF);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x7F, 0xB2, 0xC2);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x3F, 0x66, 0x74);
pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0xE1, 0xFF);
pub const INFO: Color32 = Color32::from_rgb(0x8C, 0x9E, 0xFF);
pub const WARNING: Color32 = Color32::from_rgb(0xFF, 0xB0, 0x00);
pub const DANGER: Color32 = Color32::from_rgb(0xFF, 0x4D, 0x4D);

/// One hue per account, in the order accounts take them.
pub const ACCOUNT_HUES: [Color32; 8] = [
    Color32::from_rgb(0x39, 0xFF, 0x9E),
    Color32::from_rgb(0xED, 0xE9, 0x5C),
    Color32::from_rgb(0xC7, 0x92, 0xEA),
    Color32::from_rgb(0xA6, 0xE2, 0x2E),
    Color32::from_rgb(0xF5, 0xA9, 0x7F),
    Color32::from_rgb(0xFF, 0x9C, 0xC2),
    Color32::from_rgb(0xD8, 0xC8, 0xA0),
    Color32::from_rgb(0xA9, 0xB8, 0xC2),
];

const SEMIBOLD: &str = "semibold";

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let faces = [
        ("JetBrainsMono-400", &include_bytes!("../assets/fonts/JetBrainsMono-400.ttf")[..]),
        ("JetBrainsMono-600", &include_bytes!("../assets/fonts/JetBrainsMono-600.ttf")[..]),
    ];
    for (name, bytes) in faces {
        fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
    }
    // egui's own fonts stay behind ours for the glyphs the Latin subset lacks.
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().insert(0, "JetBrainsMono-400".into());
    }
    let mut semibold = vec!["JetBrainsMono-600".to_string()];
    semibold.extend(fallback);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    ctx.set_fonts(fonts);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = STATUS;
    visuals.window_fill = SURFACE_TOP;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = 4.into();
    visuals.menu_corner_radius = 4.into();
    visuals.popup_shadow = egui::Shadow::NONE;
    visuals.window_shadow = egui::Shadow::NONE;
    visuals.override_text_color = Some(TEXT);
    ctx.set_visuals(visuals);
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(channel(from.r(), to.r()), channel(from.g(), to.g()), channel(from.b(), to.b()))
}

/// The light at `t`: `--accent` at 0, `--info` at 1.
pub fn light(t: f32) -> Color32 {
    mix(ACCENT, INFO, t)
}

fn quad(painter: &egui::Painter, rect: Rect, colors: [Color32; 4]) {
    let mut mesh = egui::Mesh::default();
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    for (pos, color) in corners.into_iter().zip(colors) {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// A fill lit left to right.
pub fn light_across(painter: &egui::Painter, rect: Rect, from: Color32, to: Color32) {
    quad(painter, rect, [from, to, to, from]);
}

/// A fill lit bottom to top.
pub fn light_up(painter: &egui::Painter, rect: Rect, foot: Color32, top: Color32) {
    quad(painter, rect, [top, top, foot, foot]);
}

/// The panel's one edge, 1px on the side that faces the screen: accent at both ends, info in
/// the middle. The other three sides meet the display's own edges.
pub fn edge(painter: &egui::Painter, rect: Rect, at_right: bool) {
    let x = if at_right { rect.right() - 1.0 } else { rect.left() };
    let mid = rect.center().y;
    let upper = Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(x + 1.0, mid));
    let lower = Rect::from_min_max(Pos2::new(x, mid), Pos2::new(x + 1.0, rect.bottom()));
    light_up(painter, upper, INFO, ACCENT);
    light_up(painter, lower, ACCENT, INFO);
}

/// The wash behind an account's name: the light at a fraction of its strength, under a full line.
pub fn band(painter: &egui::Painter, rect: Rect) {
    light_across(painter, rect, ACCENT.gamma_multiply(0.24), INFO.gamma_multiply(0.14));
    let line = Rect::from_min_max(rect.left_top(), Pos2::new(rect.right(), rect.top() + 1.0));
    light_across(painter, line, ACCENT, INFO);
}

/// A hairline tied to the accent, as between the radio's rows.
pub fn hairline() -> Color32 {
    ACCENT.gamma_multiply(0.3)
}

/// Uppercase, semibold and widely tracked, with the light running across the letters.
pub fn lit_caps(text: &str, size: f32) -> LayoutJob {
    let text = text.to_uppercase();
    let last = text.chars().count().saturating_sub(1).max(1) as f32;
    let mut job = LayoutJob::default();
    for (i, c) in text.chars().enumerate() {
        job.append(
            c.encode_utf8(&mut [0; 4]),
            0.0,
            TextFormat {
                font_id: semibold(size),
                color: light(i as f32 / last),
                extra_letter_spacing: TRACKING,
                ..Default::default()
            },
        );
    }
    job
}

/// Letter spacing of uppercase labels.
pub const TRACKING: f32 = 1.6;
