//! The PMOS egui theme — mirrors the design tokens in `pmos-web/index.html`
//! (UI spec §6: dark, high-contrast, projector-friendly).
//!
//! Color schemes (UI spec §6.1): a small set of accent palettes selectable in
//! Settings → Appearance. The active scheme lives in an atomic so the cursor,
//! dock, and palette all pick up changes immediately without re-plumbing.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle};
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

/// Register the PMOS typefaces: Inter for UI text (with Inter Medium for
/// headings) and JetBrains Mono for code — egui's built-in fonts stay as
/// fallbacks so emoji and symbols keep rendering. Call once at boot.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.ttf")).into(),
    );
    fonts.font_data.insert(
        "InterMedium".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-Medium.ttf")).into(),
    );
    fonts.font_data.insert(
        "JetBrainsMono".into(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/JetBrainsMono-Regular.ttf"
        ))
        .into(),
    );
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "Inter".into());
    // Heading family: Inter Medium first, then the same fallback chain.
    let heading_chain: Vec<String> = std::iter::once("InterMedium".to_string())
        .chain(prop.clone())
        .collect();
    fonts
        .families
        .insert(FontFamily::Name("heading".into()), heading_chain);
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrainsMono".into());
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    let (a, b) = (accent_a(), accent_b());
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        // Type scale (UI spec §6): Inter with a Medium cut for headings,
        // JetBrains Mono for code, comfortable sizes for projector reading.
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(17.0, FontFamily::Name("heading".into())),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
            (
                TextStyle::Button,
                FontId::new(13.5, FontFamily::Proportional),
            ),
            (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        ]
        .into();

        // Breathing room: the difference between clunky and calm.
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(8.0, 7.0);
        s.button_padding = egui::vec2(12.0, 5.0);
        s.interact_size.y = 28.0;
        s.window_margin = Margin::same(14);
        s.menu_margin = Margin::same(10);
        s.indent = 20.0;

        // Text is selectable everywhere (copy works via Ctrl+C — the platform
        // mirrors egui copies to the system clipboard).
        style.interaction.selectable_labels = true;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = Some(INK);
        v.panel_fill = Color32::from_rgba_unmultiplied(0x0d, 0x0f, 0x1c, 0xd8);
        v.window_fill = Color32::from_rgba_unmultiplied(0x0d, 0x0f, 0x1c, 0xee);
        v.window_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x8b, 0x93, 0xa9, 0x30));
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(10);
        v.window_shadow = Shadow {
            offset: [0, 10],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(120),
        };
        v.popup_shadow = Shadow {
            offset: [0, 6],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(100),
        };
        v.extreme_bg_color = Color32::from_rgb(0x09, 0x0b, 0x14); // text edits
        v.faint_bg_color = Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0x06);
        v.selection.bg_fill = a.gamma_multiply(0.32);
        v.hyperlink_color = a;

        // Soft, consistent widget chrome: quiet fills, hairline strokes,
        // accent only on interaction.
        let raise = Color32::from_rgba_unmultiplied(0x1a, 0x1e, 0x33, 0xcc);
        v.widgets.noninteractive.bg_stroke =
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x8b, 0x93, 0xa9, 0x20));
        v.widgets.inactive.bg_fill = raise;
        v.widgets.inactive.weak_bg_fill = raise;
        v.widgets.hovered.bg_fill = Color32::from_rgba_unmultiplied(0x24, 0x2a, 0x45, 0xdd);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgba_unmultiplied(0x24, 0x2a, 0x45, 0xdd);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, a.gamma_multiply(0.85));
        v.widgets.active.bg_stroke = Stroke::new(1.2, b);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = CornerRadius::same(8);
        }
    });
}
