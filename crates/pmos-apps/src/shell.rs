//! The shell process (UI spec §2): desktop with floating stage icons, dock,
//! and window management. Talks to the kernel strictly through the ABI.

use crate::apps::{AppKind, AppState, ALL};
use crate::theme;
use pmos_abi::{KernelApi, Pid, Reply, Syscall, WinDesc, WinId};

/// What the shell needs to place UI inside the 3D stage: the camera transform
/// and viewport, provided by the platform glue each frame. This is a drawing
/// aid, not a kernel API — apps never see it.
pub struct StageView {
    /// Column-major view-projection matrix.
    pub view_proj: [f32; 16],
    /// Viewport size in egui points.
    pub viewport: [f32; 2],
    /// Seconds since boot (drives the icon bobbing).
    pub time: f32,
}

impl StageView {
    /// Project a world position to screen points. Returns the position and a
    /// perspective scale factor (1.0 at ~13 units — the default camera dist).
    fn project(&self, world: [f32; 3]) -> Option<(egui::Pos2, f32)> {
        let m = &self.view_proj;
        let (x, y, z) = (world[0], world[1], world[2]);
        let cx = m[0] * x + m[4] * y + m[8] * z + m[12];
        let cy = m[1] * x + m[5] * y + m[9] * z + m[13];
        let cw = m[3] * x + m[7] * y + m[11] * z + m[15];
        if cw < 0.5 {
            return None; // behind or too close to the camera
        }
        let ndc_x = cx / cw;
        let ndc_y = cy / cw;
        let px = (ndc_x + 1.0) * 0.5 * self.viewport[0];
        let py = (1.0 - ndc_y) * 0.5 * self.viewport[1];
        Some((egui::pos2(px, py), (13.0 / cw).clamp(0.35, 2.2)))
    }
}

struct OpenApp {
    pid: Pid,
    win: WinId,
    state: AppState,
    open: bool,
    focus: bool,
}

pub struct Shell {
    pid: Pid,
    open_apps: Vec<OpenApp>,
    icon_slots: Vec<(AppKind, [f32; 3])>,
    themed: bool,
}

impl Shell {
    pub fn new(kernel: &mut dyn KernelApi) -> Self {
        // The kernel guarantees the first registered process is the shell
        // (pid 1) and grants it the shell capability set.
        let pid = match kernel.syscall(Pid(0), Syscall::ProcRegister { name: "shell".into() }) {
            Ok(Reply::Pid(pid)) => pid,
            other => {
                log::error!("shell registration failed: {other:?}");
                Pid(1)
            }
        };
        // Floating icons on an arc between the default camera and the origin
        // (UI spec §2.8). Angles are from the +z axis, radius 5.5, eye height.
        let icon_slots = ALL
            .iter()
            .enumerate()
            .map(|(i, kind)| {
                let a = (i as f32 - (ALL.len() - 1) as f32 / 2.0) * 0.42;
                (*kind, [5.5 * a.sin(), 2.1, 5.5 * a.cos()])
            })
            .collect();
        Self { pid, open_apps: Vec::new(), icon_slots, themed: false }
    }

    pub fn update(&mut self, ctx: &egui::Context, kernel: &mut dyn KernelApi, stage: &StageView) {
        if !self.themed {
            theme::apply(ctx);
            self.themed = true;
        }
        for ev in kernel.poll_events(self.pid) {
            log::debug!("shell event: {ev:?}");
        }
        self.stage_icons(ctx, kernel, stage);
        self.windows(ctx, kernel);
        self.dock(ctx, kernel);
        self.help_hint(ctx);
    }

    fn is_open(&self, kind: AppKind) -> bool {
        self.open_apps.iter().any(|a| a.state.kind == kind)
    }

    fn launch(&mut self, kernel: &mut dyn KernelApi, kind: AppKind) {
        // Refocus if already open (single-instance apps in v1).
        if let Some(app) = self.open_apps.iter_mut().find(|a| a.state.kind == kind) {
            app.focus = true;
            return;
        }
        // Each app is a real kernel process: register, then open its window
        // via capability-checked syscalls (the ABI's permanent smoke test).
        let Ok(Reply::Pid(pid)) =
            kernel.syscall(self.pid, Syscall::ProcRegister { name: kind.title().into() })
        else {
            return;
        };
        let desc =
            WinDesc { title: kind.title().into(), size: kind.default_size(), resizable: true };
        let Ok(Reply::Win(win)) = kernel.syscall(pid, Syscall::WinCreate(desc)) else {
            return;
        };
        self.open_apps.push(OpenApp {
            pid,
            win,
            state: AppState::new(kind),
            open: true,
            focus: true,
        });
    }

    fn close(&mut self, kernel: &mut dyn KernelApi, idx: usize) {
        let app = self.open_apps.remove(idx);
        let _ = kernel.syscall(app.pid, Syscall::WinClose(app.win));
        let _ = kernel.syscall(self.pid, Syscall::ProcKill(app.pid));
    }

    /// Floating app icons living in the 3D stage (UI spec §2.8): world
    /// positions projected to the overlay each frame, so they orbit, bob and
    /// scale exactly like stage objects.
    fn stage_icons(&mut self, ctx: &egui::Context, kernel: &mut dyn KernelApi, stage: &StageView) {
        let mut clicked: Option<AppKind> = None;
        for (i, (kind, base)) in self.icon_slots.iter().enumerate() {
            let bob = (stage.time * 0.9 + i as f32 * 1.3).sin() * 0.14;
            let world = [base[0], base[1] + bob, base[2]];
            let Some((pos, scale)) = stage.project(world) else { continue };
            let size = 58.0 * scale;
            let open = self.is_open(*kind);

            egui::Area::new(egui::Id::new(("stage-icon", i)))
                .fixed_pos(pos - egui::vec2(size / 2.0, size / 2.0))
                .order(egui::Order::Background)
                .show(ctx, |ui| {
                    let (rect, resp) = ui
                        .allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
                    let hovered = resp.hovered();
                    let p = ui.painter();
                    let center = rect.center();
                    let glow = if hovered { 0.55 } else { 0.28 };
                    p.circle_filled(
                        center,
                        size * 0.52,
                        theme::BG_RAISE.gamma_multiply(if hovered { 0.95 } else { 0.75 }),
                    );
                    p.circle_stroke(
                        center,
                        size * 0.52,
                        egui::Stroke::new(
                            if hovered { 2.0 } else { 1.2 },
                            theme::ACCENT_A.gamma_multiply(glow),
                        ),
                    );
                    if open {
                        p.circle_filled(
                            center + egui::vec2(0.0, size * 0.62),
                            2.5,
                            theme::ACCENT_A,
                        );
                    }
                    p.text(
                        center,
                        egui::Align2::CENTER_CENTER,
                        kind.icon(),
                        egui::FontId::proportional(size * 0.52),
                        theme::INK,
                    );
                    if hovered {
                        p.text(
                            center + egui::vec2(0.0, size * 0.78),
                            egui::Align2::CENTER_TOP,
                            kind.title(),
                            egui::FontId::proportional(13.0),
                            theme::INK,
                        );
                    }
                    if resp.clicked() {
                        clicked = Some(*kind);
                    }
                });
        }
        if let Some(kind) = clicked {
            self.launch(kernel, kind);
        }
    }

    /// Window manager v1 (UI spec §2.1): egui windows own drag/resize; the
    /// shell owns lifecycle, which flows through syscalls.
    fn windows(&mut self, ctx: &egui::Context, kernel: &mut dyn KernelApi) {
        let mut to_close: Vec<usize> = Vec::new();
        for (i, app) in self.open_apps.iter_mut().enumerate() {
            let mut open = app.open;
            let win = egui::Window::new(format!(
                "{}  {}",
                app.state.kind.icon(),
                app.state.kind.title()
            ))
            .id(egui::Id::new(("app-window", app.win)))
            .default_size(app.state.kind.default_size())
            .resizable(true)
            .collapsible(true)
            .open(&mut open);
            let win = if app.focus {
                app.focus = false;
                win.default_pos(egui::pos2(
                    120.0 + i as f32 * 36.0,
                    90.0 + i as f32 * 30.0,
                ))
            } else {
                win
            };
            win.show(ctx, |ui| app.state.ui(ui));
            if !open {
                to_close.push(i);
            }
        }
        for i in to_close.into_iter().rev() {
            self.close(kernel, i);
        }
    }

    /// The dock (UI spec §2.2): flat, fast access mirroring the stage icons.
    fn dock(&mut self, ctx: &egui::Context, kernel: &mut dyn KernelApi) {
        let mut clicked: Option<AppKind> = None;
        egui::Area::new(egui::Id::new("dock"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -14.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::window(ui.style())
                    .corner_radius(egui::CornerRadius::same(16))
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for kind in ALL {
                                let open = self.is_open(kind);
                                let label = egui::RichText::new(kind.icon()).size(24.0);
                                let btn = ui
                                    .add(egui::Button::new(label).frame(false))
                                    .on_hover_text(kind.title());
                                if btn.clicked() {
                                    clicked = Some(kind);
                                }
                                if open {
                                    let below = btn.rect.center_bottom() + egui::vec2(0.0, 3.0);
                                    ui.painter().circle_filled(below, 2.0, theme::ACCENT_A);
                                }
                            }
                        });
                    });
            });
        if let Some(kind) = clicked {
            self.launch(kernel, kind);
        }
    }

    fn help_hint(&self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("cam-hint"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 10.0))
            .order(egui::Order::Background)
            .show(ctx, |ui| {
                ui.weak("drag: orbit · wheel: zoom · Home: reset");
            });
    }
}
