//! The shell process (UI spec §2): desktop, dock, and window management.
//! Talks to the kernel strictly through the ABI.

use crate::apps::{AppKind, AppState, ALL};
use crate::cursor::HandCursor;
use crate::theme;
use pmos_abi::{KernelApi, KernelEvent, Pid, Reply, Syscall, WinDesc, WinId};

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
    cursor: HandCursor,
    /// Latest raw landmark frame (viewer overlay; capability-gated events).
    raw_hands: (Vec<f32>, u8),
    themed: bool,
}

impl Shell {
    pub fn new(kernel: &mut dyn KernelApi) -> Self {
        // The kernel guarantees the first registered process is the shell
        // (pid 1) and grants it the shell capability set.
        let pid = match kernel.syscall(
            Pid(0),
            Syscall::ProcRegister {
                name: "shell".into(),
                caps: Vec::new(),
            },
        ) {
            Ok(Reply::Pid(pid)) => pid,
            other => {
                log::error!("shell registration failed: {other:?}");
                Pid(1)
            }
        };
        Self {
            pid,
            open_apps: Vec::new(),
            cursor: HandCursor::new(),
            raw_hands: (Vec::new(), 0),
            themed: false,
        }
    }

    pub fn update(
        &mut self,
        ctx: &egui::Context,
        kernel: &mut dyn KernelApi,
        camera_feed: Option<egui::TextureId>,
    ) {
        if !self.themed {
            theme::apply(ctx);
            self.themed = true;
        }
        for ev in kernel.poll_events(self.pid) {
            match ev {
                KernelEvent::HandUpdate {
                    pose,
                    pinch,
                    pos,
                    tracking,
                    hands,
                } => {
                    self.cursor.pose = pose;
                    self.cursor.pinch = pinch;
                    if pos.is_some() {
                        self.cursor.pos = pos; // tracking-lost keeps the last position (frozen)
                    }
                    self.cursor.tracking = tracking;
                    self.cursor.hands = hands;
                }
                KernelEvent::CameraStatus { enabled, reason } => {
                    self.cursor.camera_enabled = enabled;
                    self.cursor.camera_reason = reason;
                }
                KernelEvent::RawHands { data, hands } => self.raw_hands = (data, hands),
                other => log::debug!("shell event: {other:?}"),
            }
        }
        if !self.cursor.tracking {
            self.raw_hands.1 = 0;
        }
        self.windows(ctx, kernel, camera_feed);
        self.dock(ctx, kernel);
        self.help_hint(ctx);
        self.cursor.draw(ctx);
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
        // Built-in apps that need more than the default grant get it by
        // delegation from the shell's own capability set.
        let caps = match kind {
            AppKind::HandTracker => vec![pmos_abi::Capability::InputRawHands],
            _ => Vec::new(),
        };
        let Ok(Reply::Pid(pid)) = kernel.syscall(
            self.pid,
            Syscall::ProcRegister {
                name: kind.title().into(),
                caps,
            },
        ) else {
            return;
        };
        let desc = WinDesc {
            title: kind.title().into(),
            size: kind.default_size(),
            resizable: true,
        };
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
        let mut app = self.open_apps.remove(idx);
        if app.state.kind == AppKind::HandTracker {
            app.state.hand_tracker.on_close(kernel, app.pid);
        }
        let _ = kernel.syscall(app.pid, Syscall::WinClose(app.win));
        let _ = kernel.syscall(self.pid, Syscall::ProcKill(app.pid));
    }

    /// Window manager v1 (UI spec §2.1): egui windows own drag/resize; the
    /// shell owns lifecycle, which flows through syscalls.
    fn windows(
        &mut self,
        ctx: &egui::Context,
        kernel: &mut dyn KernelApi,
        camera_feed: Option<egui::TextureId>,
    ) {
        // Snapshot the cursor/landmark state the Hand Tracker window needs,
        // so the window closure doesn't re-borrow self.
        let raw = self.raw_hands.clone();
        let cam_on = self.cursor.camera_enabled;
        let cam_reason = self.cursor.camera_reason.clone();
        let tracking = self.cursor.tracking;
        let pose_label = format!("{:?}", self.cursor.pose);
        let screen = ctx.content_rect();

        let mut to_close: Vec<usize> = Vec::new();
        for (i, app) in self.open_apps.iter_mut().enumerate() {
            let mut open = app.open;
            let kind = app.state.kind;
            let size = kind.default_size();
            let win = egui::Window::new(format!("{}  {}", kind.icon(), kind.title()))
                .id(egui::Id::new(("app-window", app.win)))
                .default_size(size)
                .resizable(true)
                .collapsible(true)
                .open(&mut open);
            let win = if app.focus {
                app.focus = false;
                if kind == AppKind::HandTracker {
                    // The viewer lives bottom-right, above the dock (spec §8).
                    win.default_pos(egui::pos2(
                        screen.max.x - size[0] - 18.0,
                        (screen.max.y - size[1] - 84.0).max(10.0),
                    ))
                } else {
                    win.default_pos(egui::pos2(120.0 + i as f32 * 36.0, 90.0 + i as f32 * 30.0))
                }
            } else {
                win
            };
            let pid = app.pid;
            let state = &mut app.state;
            win.show(ctx, |ui| {
                if kind == AppKind::HandTracker {
                    state.hand_tracker.ui(
                        ui,
                        kernel,
                        pid,
                        camera_feed,
                        &raw,
                        cam_on,
                        &cam_reason,
                        tracking,
                        &pose_label,
                    );
                } else {
                    state.ui(ui);
                }
            });
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
                ui.horizontal(|ui| {
                    ui.weak(self.cursor.tray_text());
                    ui.weak("·");
                    ui.weak("drag: orbit · wheel: zoom · Home: reset");
                });
            });
    }
}
