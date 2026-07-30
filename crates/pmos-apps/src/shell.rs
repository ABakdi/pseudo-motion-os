//! The shell process (UI spec §2): desktop, dock, and window management.
//! Talks to the kernel strictly through the ABI.

use crate::app_host;
use crate::apps::{AppAction, AppKind, AppState, ALL};
use crate::cursor::HandCursor;
use crate::palette::{Palette, PaletteOutcome};
use crate::theme;
use pmos_abi::{KernelApi, KernelEvent, Pid, Reply, Syscall, WinDesc, WinId};
use pmos_conjure::{AppInstance, Effect};

struct OpenApp {
    pid: Pid,
    win: WinId,
    state: AppState,
    open: bool,
    focus: bool,
}

/// A conjured (AI-generated) app running in the App Host.
struct ConjureApp {
    pid: Pid,
    win: WinId,
    app: AppInstance,
    title: String,
    icon: String,
    open: bool,
}

pub struct Shell {
    pid: Pid,
    open_apps: Vec<OpenApp>,
    cursor: HandCursor,
    /// Latest raw landmark frame (viewer overlay; capability-gated events).
    raw_hands: (Vec<f32>, u8),
    /// Launcher overlay (UI spec §2.3): open-palm hold or dock ≡.
    launcher_open: bool,
    palm_since: Option<f64>,
    palm_fired: bool,
    /// The command palette (UI spec §2.4).
    palette: Palette,
    call_since: Option<f64>,
    conjure_apps: Vec<ConjureApp>,
    toasts: Vec<(String, f64)>,
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
            launcher_open: false,
            palm_since: None,
            palm_fired: false,
            palette: Palette::new(),
            call_since: None,
            conjure_apps: Vec::new(),
            toasts: Vec::new(),
            themed: false,
        }
    }

    fn toast(&mut self, text: String, now: f64) {
        self.toasts.push((text, now + 5.0));
    }

    fn handle_outcomes(
        &mut self,
        outcomes: Vec<PaletteOutcome>,
        kernel: &mut dyn KernelApi,
        now: f64,
    ) {
        for outcome in outcomes {
            match outcome {
                PaletteOutcome::Launch(kind) => self.launch(kernel, kind),
                PaletteOutcome::OpenLauncher => self.launcher_open = true,
                PaletteOutcome::Prompt(agent, msg) => {
                    let _ = kernel.syscall(self.pid, Syscall::AiPrompt { agent, msg });
                }
                PaletteOutcome::SpawnConjure(doc) => {
                    if let Err(e) = self.spawn_conjure(kernel, &doc, now) {
                        self.toast(format!("⚠ couldn't spawn app: {e}"), now);
                    }
                }
            }
        }
    }

    /// Spawn a validated Conjure document as a real process + App Host window
    /// (Architecture §3.3 — the ABI's ProcSpawnApp path).
    fn spawn_conjure(
        &mut self,
        kernel: &mut dyn KernelApi,
        doc_src: &str,
        now: f64,
    ) -> Result<(), String> {
        let doc = pmos_conjure::validate(doc_src)
            .map_err(|e| e.first().map(|x| x.message.clone()).unwrap_or_default())?;
        let Ok(Reply::Pid(pid)) = kernel.syscall(
            self.pid,
            Syscall::ProcRegister {
                name: doc.manifest.name.clone(),
                caps: Vec::new(),
            },
        ) else {
            return Err("process registration failed".into());
        };
        let desc = WinDesc {
            title: doc.manifest.name.clone(),
            size: doc.manifest.window.size,
            resizable: doc.manifest.window.resizable,
        };
        let Ok(Reply::Win(win)) = kernel.syscall(pid, Syscall::WinCreate(desc)) else {
            return Err("window creation failed".into());
        };
        // Conjured apps persist as app bundles (relaunchable from Files,
        // the terminal `run` command, or after a reload).
        let _ = kernel.syscall(
            self.pid,
            Syscall::FsWrite {
                path: format!("/apps/{}.conjure", doc.manifest.id),
                bytes: doc_src.as_bytes().to_vec(),
            },
        );
        let title = doc.manifest.name.clone();
        let icon = doc.manifest.icon.clone();
        self.conjure_apps.push(ConjureApp {
            pid,
            win,
            app: AppInstance::new(doc, now * 1000.0),
            title,
            icon,
            open: true,
        });
        Ok(())
    }

    pub fn update(
        &mut self,
        ctx: &egui::Context,
        kernel: &mut dyn KernelApi,
        camera_feed: Option<egui::TextureId>,
        today: &str,
    ) {
        // Route per-app events (e.g. the terminal's assistant stream).
        for app in &mut self.open_apps {
            for ev in kernel.poll_events(app.pid) {
                if let KernelEvent::AiChunk { text, done, .. } = ev {
                    app.state.on_ai_chunk(&text, done);
                }
            }
        }
        if !self.themed {
            theme::apply(ctx);
            self.themed = true;
        }
        let now = ctx.input(|i| i.time);
        let mut ai_outcomes: Vec<PaletteOutcome> = Vec::new();
        for ev in kernel.poll_events(self.pid) {
            match ev {
                KernelEvent::HandUpdate {
                    pose,
                    pinch,
                    pos,
                    tracking,
                    hands,
                } => {
                    // Click ripple feedback on pinch onset (UI spec §6).
                    if pose == pmos_abi::HandPose::Pinch
                        && self.cursor.pose != pmos_abi::HandPose::Pinch
                    {
                        self.cursor.last_pinch = now;
                    }
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
                KernelEvent::AiChunk { agent, text, done } => {
                    let outcomes = self.palette.on_chunk(agent, &text, done);
                    ai_outcomes.extend(outcomes);
                }
                other => log::debug!("shell event: {other:?}"),
            }
        }
        if !self.cursor.tracking {
            self.raw_hands.1 = 0;
        }

        // Open-palm hold toggles the launcher (Hand Gestures spec G5).
        if self.cursor.tracking && self.cursor.pose == pmos_abi::HandPose::OpenPalm {
            let since = *self.palm_since.get_or_insert(now);
            if now - since >= 0.6 && !self.palm_fired {
                self.launcher_open = !self.launcher_open;
                self.palm_fired = true;
            }
        } else {
            self.palm_since = None;
            self.palm_fired = false;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.launcher_open = false;
        }

        // 🤙 tap toggles the palette (Hand Gestures spec G8: tap < 0.5 s).
        if self.cursor.tracking && self.cursor.pose == pmos_abi::HandPose::CallSign {
            self.call_since.get_or_insert(now);
        } else if let Some(since) = self.call_since.take() {
            if now - since < 0.5 {
                self.palette.toggle();
            }
        }
        // Ctrl+K.
        if ctx.input(|i| i.key_pressed(egui::Key::K) && i.modifiers.command) {
            self.palette.toggle();
        }

        self.handle_outcomes(ai_outcomes, kernel, now);
        self.windows(ctx, kernel, camera_feed, today, now);
        self.conjure_windows(ctx, kernel, now);
        self.dock(ctx, kernel);
        self.launcher(ctx, kernel);
        let palette_outcomes = self.palette.ui(ctx);
        self.handle_outcomes(palette_outcomes, kernel, now);
        self.draw_toasts(ctx, now);
        self.help_hint(ctx);
        self.cursor.draw(ctx);
    }

    /// Windows of conjured apps, rendered through the App Host.
    fn conjure_windows(&mut self, ctx: &egui::Context, kernel: &mut dyn KernelApi, now: f64) {
        let mut closed: Vec<usize> = Vec::new();
        let mut effects: Vec<(usize, Vec<Effect>)> = Vec::new();
        for (i, capp) in self.conjure_apps.iter_mut().enumerate() {
            let mut open = capp.open;
            let win = egui::Window::new(format!("{}  {}", capp.icon, capp.title))
                .id(egui::Id::new(("conjure-window", capp.win)))
                .default_size(capp.app.doc.manifest.window.size)
                .resizable(capp.app.doc.manifest.window.resizable)
                .collapsible(true)
                .open(&mut open);
            let app = &mut capp.app;
            let result = win.show(ctx, |ui| app_host::ui(app, ui, now * 1000.0));
            if let Some(inner) = result.and_then(|r| r.inner) {
                if !inner.is_empty() {
                    effects.push((i, inner));
                }
            }
            capp.open = open;
            if !open {
                closed.push(i);
            }
        }
        for (i, effs) in effects {
            for eff in effs {
                match eff {
                    Effect::Notify { title, body } => {
                        let text = if body.is_empty() {
                            format!("🔔 {title}")
                        } else {
                            format!("🔔 {title} — {body}")
                        };
                        self.toast(text, now);
                    }
                    Effect::SetTitle(t) => self.conjure_apps[i].title = t,
                    Effect::CloseWindow => self.conjure_apps[i].open = false,
                }
            }
        }
        for i in (0..self.conjure_apps.len()).rev() {
            if !self.conjure_apps[i].open {
                let capp = self.conjure_apps.remove(i);
                let _ = kernel.syscall(capp.pid, Syscall::WinClose(capp.win));
                let _ = kernel.syscall(self.pid, Syscall::ProcKill(capp.pid));
            }
        }
    }

    fn draw_toasts(&mut self, ctx: &egui::Context, now: f64) {
        self.toasts.retain(|(_, until)| *until > now);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -70.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for (text, _) in self.toasts.iter().rev().take(4) {
                    egui::Frame::window(ui.style())
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.label(text);
                        });
                }
            });
    }

    /// The launcher overlay (UI spec §2.3): a dimmed layer with the app grid.
    fn launcher(&mut self, ctx: &egui::Context, kernel: &mut dyn KernelApi) {
        if !self.launcher_open {
            return;
        }
        let screen = ctx.content_rect();
        let mut clicked: Option<AppKind> = None;
        let mut close = false;
        egui::Area::new(egui::Id::new("launcher"))
            .fixed_pos(screen.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                // Dim the world; clicking the backdrop closes.
                let resp = ui.allocate_response(screen.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
                if resp.clicked() {
                    close = true;
                }
            });
        egui::Area::new(egui::Id::new("launcher-grid"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -30.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::window(ui.style())
                    .corner_radius(egui::CornerRadius::same(18))
                    .inner_margin(egui::Margin::same(24))
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new("Launcher")
                                    .size(18.0)
                                    .color(theme::INK_DIM),
                            );
                            ui.add_space(14.0);
                            egui::Grid::new("launcher-apps")
                                .num_columns(3)
                                .spacing([18.0, 18.0])
                                .show(ui, |ui| {
                                    for (i, kind) in ALL.iter().enumerate() {
                                        let tile = ui
                                            .add_sized(
                                                [110.0, 84.0],
                                                egui::Button::new(
                                                    egui::RichText::new(format!(
                                                        "{}\n{}",
                                                        kind.icon(),
                                                        kind.title()
                                                    ))
                                                    .size(15.0),
                                                ),
                                            )
                                            .on_hover_text(kind.title());
                                        if tile.clicked() {
                                            clicked = Some(*kind);
                                        }
                                        if i % 3 == 2 {
                                            ui.end_row();
                                        }
                                    }
                                });
                            ui.add_space(10.0);
                            ui.label(
                                egui::RichText::new("open palm to toggle · Esc to close")
                                    .size(11.0)
                                    .color(theme::INK_DIM),
                            );
                        });
                    });
            });
        if let Some(kind) = clicked {
            self.launch(kernel, kind);
            close = true;
        }
        if close {
            self.launcher_open = false;
        }
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
        // Built-in apps get their extra capabilities by delegation from the
        // shell's own set (per-app lists live next to the apps).
        let caps = kind.caps();
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
    #[allow(clippy::too_many_arguments)]
    fn windows(
        &mut self,
        ctx: &egui::Context,
        kernel: &mut dyn KernelApi,
        camera_feed: Option<egui::TextureId>,
        today: &str,
        now: f64,
    ) {
        let mut actions: Vec<AppAction> = Vec::new();
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
            let inner = win.show(ctx, |ui| match kind {
                AppKind::HandTracker => {
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
                    None
                }
                AppKind::Settings => {
                    state.settings_ui(ui, kernel, pid);
                    None
                }
                AppKind::Terminal => state.terminal_ui(ui, kernel, pid),
                AppKind::Files => state.files_ui(ui, kernel, pid),
                AppKind::Notes => {
                    state.notes_ui(ui, kernel, pid, today);
                    None
                }
                _ => {
                    state.ui(ui);
                    None
                }
            });
            if let Some(Some(action)) = inner.and_then(|r| r.inner) {
                actions.push(action);
            }
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
                            let launcher_btn = ui
                                .add(
                                    egui::Button::new(egui::RichText::new("◆").size(22.0))
                                        .frame(false),
                                )
                                .on_hover_text("Launcher (open palm)");
                            if launcher_btn.clicked() {
                                self.launcher_open = !self.launcher_open;
                            }
                            let palette_btn = ui
                                .add(
                                    egui::Button::new(egui::RichText::new("✨").size(22.0))
                                        .frame(false),
                                )
                                .on_hover_text("AI palette (Ctrl+K · 🤙 tap)");
                            if palette_btn.clicked() {
                                self.palette.toggle();
                            }
                            ui.separator();
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
