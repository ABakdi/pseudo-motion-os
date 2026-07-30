//! The PMOS egui theme — mirrors the design tokens in `pmos-web/index.html`
//! (UI spec §6: dark, high-contrast, projector-friendly).

use egui::{Color32, CornerRadius, Stroke};

pub const BG: Color32 = Color32::from_rgb(0x07, 0x08, 0x0f);
pub const BG_RAISE: Color32 = Color32::from_rgb(0x0d, 0x0f, 0x1c);
pub const INK: Color32 = Color32::from_rgb(0xe8, 0xec, 0xf4);
pub const INK_DIM: Color32 = Color32::from_rgb(0x8b, 0x93, 0xa9);
pub const ACCENT_A: Color32 = Color32::from_rgb(0x6e, 0xe7, 0xff);
pub const ACCENT_B: Color32 = Color32::from_rgb(0xc0, 0x84, 0xfc);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = Some(INK);
        v.panel_fill = Color32::from_rgba_unmultiplied(0x0d, 0x0f, 0x1c, 0xd8);
        v.window_fill = Color32::from_rgba_unmultiplied(0x0d, 0x0f, 0x1c, 0xea);
        v.window_stroke =
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x8b, 0x93, 0xa9, 0x38));
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(10);
        v.selection.bg_fill = Color32::from_rgba_unmultiplied(0x6e, 0xe7, 0xff, 0x50);
        v.hyperlink_color = ACCENT_A;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_A);
        v.widgets.active.bg_stroke = Stroke::new(1.2, ACCENT_B);
    });
}
