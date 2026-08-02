//! The morphing hand cursor (UI spec §4): a live glyph mirroring the
//! recognized pose, so the user always sees what the system sees. Drawn in
//! the overlay's foreground from recognizer state — never raw landmarks.

use crate::theme;
use pmos_abi::HandPose;

pub struct HandCursor {
    pub pose: HandPose,
    pub pinch: f32,
    pub pos: Option<[f32; 2]>,
    pub tracking: bool,
    pub hands: u8,
    pub camera_enabled: bool,
    pub camera_reason: String,
    /// Time of the last pinch onset — drives the click ripple (UI spec §6).
    pub last_pinch: f64,
}

impl HandCursor {
    pub fn new() -> Self {
        Self {
            pose: HandPose::Rest,
            pinch: 0.0,
            pos: None,
            tracking: false,
            hands: 0,
            camera_enabled: false,
            camera_reason: String::new(),
            last_pinch: -10.0,
        }
    }

    /// Tray indicator (Hand Gestures spec §7: camera status always visible).
    pub fn tray_text(&self) -> String {
        if !self.camera_enabled {
            "📷 off".to_string()
        } else if !self.tracking {
            "📷 on · no hands".to_string()
        } else {
            format!(
                "📷 tracking · {} hand{}",
                self.hands,
                if self.hands == 1 { "" } else { "s" }
            )
        }
    }

    pub fn draw(&mut self, ctx: &egui::Context) {
        // ONE cursor for every source (user decision 2026-08-02): the OS
        // arrow is hidden, and this glyph follows whichever device last
        // moved the shared pointer — morphing by hand pose while the hand
        // drives, and by button state while the mouse does.
        ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::None);
        let latest = ctx.pointer_latest_pos();
        let Some(center) = latest.or_else(|| self.pos.map(|[x, y]| egui::pos2(x, y))) else {
            return;
        };
        let hand_driving = self.camera_enabled
            && self.tracking
            && self
                .pos
                .is_some_and(|[x, y]| (x - center.x).abs() + (y - center.y).abs() < 2.0);
        let (mouse_down, mouse_pressed) = ctx.input(|i| {
            (i.pointer.primary_down(), i.pointer.primary_pressed())
        });
        if !hand_driving && mouse_pressed {
            self.last_pinch = ctx.input(|i| i.time); // mouse clicks ripple too
        }
        let t = ctx.input(|i| i.time) as f32;

        egui::Area::new(egui::Id::new("hand-cursor"))
            .fixed_pos(egui::pos2(0.0, 0.0))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                let p = ui.painter();
                // Click ripple: expanding fading ring after a pinch/click.
                let since = (ctx.input(|i| i.time) - self.last_pinch) as f32;
                if (0.0..0.35).contains(&since) {
                    let k = since / 0.35;
                    p.circle_stroke(
                        center,
                        10.0 + 42.0 * k,
                        egui::Stroke::new(2.0 * (1.0 - k), theme::accent_a().gamma_multiply(1.0 - k)),
                    );
                }
                if !hand_driving {
                    // Mouse-driven (or idle): the same PMOS ring; a held
                    // button tightens it into the "pinched" solid dot.
                    if mouse_down {
                        p.circle_filled(center, 6.0, theme::accent_a());
                        p.circle_stroke(
                            center,
                            10.0,
                            egui::Stroke::new(1.0, theme::accent_a().gamma_multiply(0.35)),
                        );
                    } else {
                        p.circle_stroke(
                            center,
                            11.0,
                            egui::Stroke::new(2.0, theme::accent_a().gamma_multiply(0.85)),
                        );
                        p.circle_filled(center, 2.2, theme::INK);
                    }
                    return;
                }
                match self.pose {
                    HandPose::Pinch => {
                        // Closed: solid dot with a soft halo.
                        p.circle_filled(center, 6.0, theme::accent_a());
                        p.circle_stroke(
                            center,
                            10.0,
                            egui::Stroke::new(1.0, theme::accent_a().gamma_multiply(0.35)),
                        );
                    }
                    HandPose::MiddlePinch => {
                        p.circle_filled(center, 6.0, theme::accent_b());
                        p.circle_stroke(
                            center,
                            10.0,
                            egui::Stroke::new(1.0, theme::accent_b().gamma_multiply(0.35)),
                        );
                    }
                    HandPose::Grab => glyph(p, center, "✊", 26.0),
                    HandPose::OpenPalm => {
                        // Palm bloom: radial pulse (RECORD sign counting up).
                        let pulse = 12.0 + 6.0 * (t * 3.0).sin().abs();
                        p.circle_stroke(
                            center,
                            pulse,
                            egui::Stroke::new(2.0, theme::accent_a().gamma_multiply(0.5)),
                        );
                        glyph(p, center, "✋", 24.0);
                    }
                    HandPose::CallSign => {
                        // Voice anchor: accent-B ring with a pulsing core.
                        let a = 0.5 + 0.5 * (t * 4.0).sin().abs();
                        p.circle_stroke(center, 12.0, egui::Stroke::new(2.0, theme::accent_b()));
                        p.circle_filled(
                            center,
                            4.0,
                            egui::Color32::from_rgb(0xff, 0x5c, 0x7a).gamma_multiply(a),
                        );
                    }
                    HandPose::ThumbsUp => glyph(p, center, "👍", 24.0),
                    HandPose::ThumbsDown => glyph(p, center, "👎", 24.0),
                    // Rest / Point / TwoFinger: the ring — tightening as a
                    // pinch forms (pre-touch feedback, spec §4).
                    _ => {
                        let r = 11.0 - 6.0 * self.pinch;
                        p.circle_stroke(
                            center,
                            r,
                            egui::Stroke::new(2.0, theme::accent_a().gamma_multiply(0.85)),
                        );
                        p.circle_filled(center, 2.2, theme::INK);
                    }
                }
            });
    }
}

impl Default for HandCursor {
    fn default() -> Self {
        Self::new()
    }
}

fn glyph(p: &egui::Painter, center: egui::Pos2, s: &str, size: f32) {
    p.text(
        center,
        egui::Align2::CENTER_CENTER,
        s,
        egui::FontId::proportional(size),
        theme::INK,
    );
}
