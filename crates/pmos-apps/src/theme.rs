//! The PMOS egui theme — mirrors the design tokens in `pmos-web/index.html`
//! (UI spec §6: dark, high-contrast, projector-friendly).
//!
//! Color schemes (UI spec §6.1): a small set of accent palettes selectable in
//! Settings → Appearance. The active scheme lives in an atomic so the cursor,
//! dock, and palette all pick up changes immediately without re-plumbing.

use egui::{Color32, CornerRadius, Stroke};
use std::sync::atomic::{AtomicU8, Ordering};

pub const BG: Color32 = Color32::from_rgb(0x07, 0x08, 0x0f);
pub const BG_RAISE: Color32 = Color32::from_rgb(0x0d, 0x0f, 0x1c);
pub const INK: Color32 = Color32::from_rgb(0xe8, 0xec, 0xf4);
pub const INK_DIM: Color32 = Color32::from_rgb(0x8b, 0x93, 0xa9);

/// (name, accent A, accent B) — accent A is the primary highlight (cursor
/// ring, hyperlinks, hover), accent B the secondary (active, middle-pinch).
pub const SCHEMES: &[(&str, Color32, Color32)] = &[
    (
        "Ion",
        Color32::from_rgb(0x6e, 0xe7, 0xff),
        Color32::from_rgb(0xc0, 0x84, 0xfc),
    ),
    (
        "Ember",
        Color32::from_rgb(0xff, 0xb3, 0x5c),
        Color32::from_rgb(0xff, 0x6e, 0x8a),
    ),
    (
        "Verdant",
        Color32::from_rgb(0x5c, 0xf2, 0xa6),
        Color32::from_rgb(0x4f, 0xd8, 0xff),
    ),
    (
        "Rose",
        Color32::from_rgb(0xff, 0x7a, 0xb8),
        Color32::from_rgb(0xc0, 0x84, 0xfc),
    ),
];

static SCHEME: AtomicU8 = AtomicU8::new(0);

pub fn scheme() -> u8 {
    SCHEME.load(Ordering::Relaxed).min(SCHEMES.len() as u8 - 1)
}

pub fn accent_a() -> Color32 {
    SCHEMES[scheme() as usize].1
}

pub fn accent_b() -> Color32 {
    SCHEMES[scheme() as usize].2
}

/// Switch the color scheme and restyle every egui style at once.
pub fn set_scheme(ctx: &egui::Context, id: u8) {
    SCHEME.store(id.min(SCHEMES.len() as u8 - 1), Ordering::Relaxed);
    apply(ctx);
}

pub fn apply(ctx: &egui::Context) {
    let (a, b) = (accent_a(), accent_b());
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        // Text is selectable everywhere (copy works via Ctrl+C — the platform
        // mirrors egui copies to the system clipboard).
        style.interaction.selectable_labels = true;
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = Some(INK);
        v.panel_fill = Color32::from_rgba_unmultiplied(0x0d, 0x0f, 0x1c, 0xd8);
        v.window_fill = Color32::from_rgba_unmultiplied(0x0d, 0x0f, 0x1c, 0xea);
        v.window_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x8b, 0x93, 0xa9, 0x38));
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(10);
        v.selection.bg_fill = a.gamma_multiply(0.32);
        v.hyperlink_color = a;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, a);
        v.widgets.active.bg_stroke = Stroke::new(1.2, b);
    });
}
