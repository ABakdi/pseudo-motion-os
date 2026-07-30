//! The Hand Tracker app (Hand Gestures spec §8): live camera viewer with a
//! landmark overlay, privacy mode (landmarks on black, no pixels), and
//! gesture-detection tuning. Doubles as the gesture debugging surface.
//!
//! Preview pixels arrive as an egui texture straight from the platform —
//! they never pass through the kernel. Landmarks arrive as capability-gated
//! `RawHands` events; tuning flows back through `HandsTune` syscalls.

use crate::theme;
use pmos_abi::{HandsTuning, KernelApi, Pid, Syscall};

/// MediaPipe hand skeleton (landmark index pairs).
const BONES: [(usize, usize); 21] = [
    (0, 1),
    (1, 2),
    (2, 3),
    (3, 4),
    (0, 5),
    (5, 6),
    (6, 7),
    (7, 8),
    (5, 9),
    (9, 10),
    (10, 11),
    (11, 12),
    (9, 13),
    (13, 14),
    (14, 15),
    (15, 16),
    (13, 17),
    (17, 18),
    (18, 19),
    (19, 20),
    (0, 17),
];

pub struct HandTrackerState {
    pub show_feed: bool,
    pub show_marks: bool,
    pub tuning: HandsTuning,
    sent_tuning: HandsTuning,
    sent_viewer: Option<(bool, bool)>,
}

impl HandTrackerState {
    pub fn new() -> Self {
        Self {
            show_feed: true,
            show_marks: true,
            tuning: HandsTuning::default(),
            sent_tuning: HandsTuning::default(),
            sent_viewer: None,
        }
    }

    /// Window closed: stop landmark events and the pixel stream.
    pub fn on_close(&mut self, kernel: &mut dyn KernelApi, pid: Pid) {
        let _ = kernel.syscall(
            pid,
            Syscall::HandsViewer {
                open: false,
                stream_feed: false,
            },
        );
        self.sent_viewer = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        kernel: &mut dyn KernelApi,
        pid: Pid,
        feed: Option<egui::TextureId>,
        raw: &(Vec<f32>, u8),
        camera_enabled: bool,
        camera_reason: &str,
        tracking: bool,
        pose_label: &str,
    ) {
        // Keep the platform in sync: viewer open; pixels only when the feed
        // is shown (privacy mode = landmarks only, zero pixels streamed).
        let want = (true, self.show_feed && camera_enabled);
        if self.sent_viewer != Some(want)
            && kernel
                .syscall(
                    pid,
                    Syscall::HandsViewer {
                        open: want.0,
                        stream_feed: want.1,
                    },
                )
                .is_ok()
        {
            self.sent_viewer = Some(want);
        }

        // ---------- viewer ----------
        let w = ui.available_width().clamp(240.0, 480.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, w * 0.75), egui::Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 8.0, egui::Color32::BLACK);

        if !camera_enabled {
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "camera is off",
                egui::FontId::proportional(14.0),
                theme::INK_DIM,
            );
        } else {
            if self.show_feed {
                match feed {
                    Some(tex) => {
                        p.image(
                            tex,
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    None => {
                        p.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "starting feed…",
                            egui::FontId::proportional(13.0),
                            theme::INK_DIM,
                        );
                    }
                }
            }
            if self.show_marks {
                for h in 0..raw.1 as usize {
                    let lm = &raw.0[h * 63..(h + 1) * 63];
                    let color = if h == 0 {
                        theme::ACCENT_A
                    } else {
                        theme::ACCENT_B
                    };
                    // Mirror x to match the selfie-view preview.
                    let map = |i: usize| {
                        egui::pos2(
                            rect.left() + (1.0 - lm[i * 3]) * rect.width(),
                            rect.top() + lm[i * 3 + 1] * rect.height(),
                        )
                    };
                    for (a, b) in BONES {
                        p.line_segment(
                            [map(a), map(b)],
                            egui::Stroke::new(1.5, color.gamma_multiply(0.55)),
                        );
                    }
                    for i in 0..21 {
                        p.circle_filled(map(i), 2.5, color);
                    }
                }
            }
        }
        p.rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0, theme::INK_DIM.gamma_multiply(0.4)),
            egui::StrokeKind::Inside,
        );

        // ---------- status ----------
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if !camera_enabled {
                ui.weak("status: off");
                if ui.button("Enable camera").clicked() {
                    let _ = kernel.syscall(pid, Syscall::CameraStart);
                }
            } else if tracking {
                ui.colored_label(theme::ACCENT_A, format!("● {}", pose_label));
                ui.weak(format!(
                    "· {} hand{}",
                    raw.1,
                    if raw.1 == 1 { "" } else { "s" }
                ));
            } else {
                ui.weak("status: on · no hands in view");
            }
        });
        if !camera_enabled && !camera_reason.is_empty() {
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(camera_reason)
                    .size(12.0)
                    .color(egui::Color32::from_rgb(0xff, 0x9d, 0x6b)),
            );
        }

        ui.add_space(4.0);
        ui.separator();

        // ---------- display toggles ----------
        ui.checkbox(&mut self.show_feed, "Show camera feed")
            .on_hover_text("Off = landmarks on black — nothing from the camera is displayed");
        ui.checkbox(&mut self.show_marks, "Show hand landmarks");

        // ---------- detection settings ----------
        ui.add_space(4.0);
        egui::CollapsingHeader::new("Detection settings")
            .default_open(true)
            .show(ui, |ui| {
                let t = &mut self.tuning;
                egui::Grid::new("ht-tuning")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Hands");
                        let mut hands = t.num_hands as u32;
                        ui.add(egui::Slider::new(&mut hands, 1..=2));
                        t.num_hands = hands as u8;
                        ui.end_row();

                        ui.label("Detection conf.");
                        ui.add(egui::Slider::new(&mut t.det_conf, 0.1..=0.9));
                        ui.end_row();

                        ui.label("Tracking conf.");
                        ui.add(egui::Slider::new(&mut t.track_conf, 0.1..=0.9));
                        ui.end_row();

                        ui.label("Smoothing");
                        egui::ComboBox::from_id_salt("ht-smooth")
                            .selected_text(
                                ["Precise", "Balanced", "Smooth"][t.smoothing.min(2) as usize],
                            )
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut t.smoothing, 0, "Precise");
                                ui.selectable_value(&mut t.smoothing, 1, "Balanced");
                                ui.selectable_value(&mut t.smoothing, 2, "Smooth");
                            });
                        ui.end_row();

                        ui.label("Pinch enter");
                        ui.add(egui::Slider::new(&mut t.pinch_enter, 0.15..=0.5));
                        ui.end_row();

                        ui.label("Pinch exit");
                        ui.add(egui::Slider::new(&mut t.pinch_exit, 0.3..=0.9));
                        ui.end_row();
                    });
                if ui.button("Reset to defaults").clicked() {
                    *t = HandsTuning::default();
                }
            });

        // Push tuning changes through the ABI only once the user releases the
        // control — sending mid-drag would rebuild the landmarker for every
        // intermediate slider value and interrupt tracking dozens of times.
        if self.tuning != self.sent_tuning
            && !ui.ctx().input(|i| i.pointer.any_down())
            && kernel.syscall(pid, Syscall::HandsTune(self.tuning)).is_ok()
        {
            self.sent_tuning = self.tuning;
        }
    }
}

impl Default for HandTrackerState {
    fn default() -> Self {
        Self::new()
    }
}
