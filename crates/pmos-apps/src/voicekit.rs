//! The Voice Kit (Voice Kit spec): the always-on voice layer's face. A
//! top-right chip that is never hidden while the mic is live; expanded, it
//! shows the live transcript (commands in accent with ⌘), search over
//! `/voice` history, and session actions. Persistence is plain FsWrite —
//! sessions are JSON files, so Files/terminal/OPFS handle them for free.

use crate::theme;
use pmos_abi::{KernelApi, Pid, Reply, Syscall};

pub enum KitAction {
    ToggleCapture,
    Toast(String),
}

#[derive(Default)]
pub struct VoiceKit {
    pub expanded: bool,
    pub capturing: bool,
    /// COMMAND sign armed: the next utterance routes as a command.
    pub command_armed: bool,
    interim: String,
    /// (seconds into session, text, is_command)
    segments: Vec<(f64, String, bool)>,
    session_started: Option<f64>,
    session_path: Option<String>,
    search: String,
    results: Vec<String>,
}

impl VoiceKit {
    pub fn on_status(&mut self, listening: bool) {
        if listening && !self.capturing {
            self.session_started = None; // fresh session on first transcript
            self.segments.clear();
        }
        self.capturing = listening;
        if !listening {
            self.interim.clear();
            self.command_armed = false;
        }
    }

    pub fn on_interim(&mut self, text: &str) {
        self.interim = text.to_string();
    }

    /// A finalized utterance: record + persist. Returns the session-relative
    /// timestamp used (for the caller's logs).
    pub fn on_final(
        &mut self,
        text: &str,
        is_command: bool,
        kernel: &mut dyn KernelApi,
        pid: Pid,
        today: &str,
        now: f64,
    ) {
        self.interim.clear();
        let started = *self.session_started.get_or_insert(now);
        if self.session_path.is_none() || self.segments.is_empty() {
            self.session_path = Some(format!("/voice/{today}/s{:.0}.json", started));
        }
        self.segments.push((now - started, text.to_string(), is_command));
        // Incremental persistence: rewrite the whole (small) session file so
        // a crash never loses more than the in-flight utterance.
        if let Some(path) = &self.session_path {
            let json = serde_json::json!({
                "date": today,
                "segments": self.segments.iter().map(|(t, s, c)| {
                    serde_json::json!({"t": (t * 10.0).round() / 10.0, "text": s, "command": c})
                }).collect::<Vec<_>>(),
            });
            let _ = kernel.syscall(
                pid,
                Syscall::FsWrite {
                    path: path.clone(),
                    bytes: json.to_string().into_bytes(),
                },
            );
        }
    }

    /// The last few non-command lines — AI context (Voice Kit spec §5).
    pub fn recent_lines(&self, n: usize) -> Vec<String> {
        self.segments
            .iter()
            .rev()
            .filter(|(_, _, cmd)| !cmd)
            .take(n)
            .map(|(_, t, _)| t.clone())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// Search all persisted sessions for a substring (case-insensitive).
    fn run_search(&mut self, kernel: &mut dyn KernelApi, pid: Pid) {
        self.results.clear();
        let needle = self.search.to_lowercase();
        if needle.is_empty() {
            return;
        }
        let days = match kernel.syscall(pid, Syscall::FsList { path: "/voice".into() }) {
            Ok(Reply::Entries(e)) => e,
            _ => return,
        };
        for day in days.iter().filter(|d| d.dir) {
            let dir = format!("/voice/{}", day.name);
            let Ok(Reply::Entries(files)) = kernel.syscall(pid, Syscall::FsList { path: dir.clone() })
            else {
                continue;
            };
            for f in files.iter().filter(|f| !f.dir) {
                let path = format!("{dir}/{}", f.name);
                if let Ok(Reply::Bytes(b)) = kernel.syscall(pid, Syscall::FsRead { path }) {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                        for seg in v["segments"].as_array().into_iter().flatten() {
                            let text = seg["text"].as_str().unwrap_or("");
                            if text.to_lowercase().contains(&needle) {
                                self.results.push(format!("{} · {}", day.name, text));
                                if self.results.len() >= 30 {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Convert the current session to a markdown note (Voice Kit spec §4).
    fn to_note(&self, kernel: &mut dyn KernelApi, pid: Pid, today: &str) -> Option<String> {
        if self.segments.is_empty() {
            return None;
        }
        let mut md = format!("# Voice note — {today}\n\n");
        for (t, text, cmd) in &self.segments {
            if *cmd {
                md.push_str(&format!("- [ ] ⌘ {text}  *({t:.0}s)*\n"));
            } else {
                md.push_str(&format!("{text}\n\n"));
            }
        }
        let path = format!(
            "/notes/voice-{today}-s{:.0}.md",
            self.session_started.unwrap_or(0.0)
        );
        kernel
            .syscall(
                pid,
                Syscall::FsWrite {
                    path: path.clone(),
                    bytes: md.into_bytes(),
                },
            )
            .ok()?;
        Some(path)
    }

    pub fn ui(&mut self, ctx: &egui::Context, kernel: &mut dyn KernelApi, pid: Pid, today: &str) -> Vec<KitAction> {
        let mut actions = Vec::new();
        egui::Area::new(egui::Id::new("voice-kit"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 34.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                // Blends with the stage (user request): no window chrome —
                // a whisper of fill, no border, no shadow.
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(0x0d, 0x0f, 0x1c, 0x70))
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // The chip — NEVER hidden while the mic is live.
                            let t = ui.input(|i| i.time);
                            let chip = if self.capturing {
                                let a = (0.55 + 0.45 * (t * 3.0).sin().abs()) as f32;
                                egui::RichText::new("● REC")
                                    .color(egui::Color32::from_rgb(0xff, 0x5c, 0x7a).gamma_multiply(a))
                                    .strong()
                            } else {
                                egui::RichText::new("⏸ voice").color(theme::INK_DIM)
                            };
                            if ui
                                .add(egui::Button::new(chip).frame(false))
                                .on_hover_text("toggle voice capture (or hold ✋ still)")
                                .clicked()
                            {
                                actions.push(KitAction::ToggleCapture);
                            }
                            if self.command_armed {
                                ui.colored_label(theme::accent_a(), "⌘ armed");
                            }
                            let arrow = if self.expanded { "▴" } else { "▾" };
                            if ui.add(egui::Button::new(arrow).frame(false)).clicked() {
                                self.expanded = !self.expanded;
                            }
                        });
                        if self.capturing {
                            ctx.request_repaint(); // keep the pulse alive
                        }
                        if !self.expanded {
                            return;
                        }
                        ui.set_width(300.0);
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .max_height(170.0)
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                if self.segments.is_empty() && self.interim.is_empty() {
                                    ui.weak("say something — the transcript lives here");
                                }
                                for (t, text, cmd) in &self.segments {
                                    if *cmd {
                                        ui.colored_label(
                                            theme::accent_a(),
                                            format!("⌘ {text}"),
                                        );
                                    } else {
                                        ui.label(format!("{text}"));
                                    }
                                    let _ = t;
                                }
                                if !self.interim.is_empty() {
                                    ui.weak(format!("… {}", self.interim));
                                }
                            });
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("→ note").on_hover_text("save session as a note").clicked()
                            {
                                match self.to_note(kernel, pid, today) {
                                    Some(p) => actions.push(KitAction::Toast(format!("📝 saved {p}"))),
                                    None => actions.push(KitAction::Toast("nothing to save yet".into())),
                                }
                            }
                            if ui.button("✕ clear").clicked() {
                                self.segments.clear();
                                self.session_path = None;
                                self.session_started = None;
                            }
                        });
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("search voice history…")
                                .desired_width(f32::INFINITY),
                        );
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            self.run_search(kernel, pid);
                        }
                        for r in &self.results {
                            ui.weak(r);
                        }
                    });
            });
        actions
    }
}
