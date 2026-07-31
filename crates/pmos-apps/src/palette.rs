//! The command palette (UI spec §2.4): one surface, three modes —
//! fuzzy commands, `>` assistant chat, and "make/create …" app conjuring
//! through the App Smith repair loop (AI System spec §4).

use crate::apps::{AppKind, ALL};
use crate::theme;
use pmos_abi::{AgentId, AGENT_APP_SMITH, AGENT_ASSISTANT};

const MAX_REPAIR_ROUNDS: u8 = 2;

pub enum PaletteOutcome {
    Launch(AppKind),
    OpenLauncher,
    /// Validated Conjure JSON ready to spawn.
    SpawnConjure(String),
    /// Send a prompt to an agent (assistant chat or App Smith repair).
    Prompt(AgentId, String),
    /// Cancel speech capture (Esc while listening).
    VoiceStop,
}

#[derive(Clone)]
enum Line {
    User(String),
    Assistant(String),
    System(String),
}

#[derive(Default)]
struct Smith {
    /// Accumulating document text while the App Smith streams.
    buf: String,
    active: bool,
    round: u8,
}

/// Voice mode state (UI spec §2.4): live transcript shown while listening —
/// voice never acts without its text being visible first.
#[derive(Default)]
struct Voice {
    listening: bool,
    /// Latest interim transcript (superseded by the final utterance).
    interim: String,
    /// Whether this session produced any transcript at all — a session that
    /// ends without one must still say so, never end silently.
    got_speech: bool,
    /// Engine progress shown while listening ("downloading speech model… 43%",
    /// "transcribing…") — Whisper's first run downloads a model.
    engine_note: String,
}

pub struct Palette {
    pub open: bool,
    input: String,
    lines: Vec<Line>,
    assistant_streaming: bool,
    smith: Smith,
    voice: Voice,
    focus_input: bool,
}

impl Palette {
    pub fn new() -> Self {
        Self {
            open: false,
            input: String::new(),
            lines: vec![Line::System(
                "type a command, `> question` for the assistant, or `make …` to conjure an app"
                    .into(),
            )],
            assistant_streaming: false,
            smith: Smith::default(),
            voice: Voice::default(),
            focus_input: false,
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.focus_input = true;
        }
    }

    /// Enter voice mode (🤙 hold): open, and show the listening state while
    /// speech capture spins up.
    pub fn start_voice(&mut self) {
        self.open = true;
        self.focus_input = false;
        self.voice = Voice {
            listening: true,
            ..Voice::default()
        };
        self.lines
            .push(Line::System("🎤 listening — speak a command…".into()));
    }

    /// Speech-engine status from the kernel. Ending with an unsubmitted
    /// interim transcript submits it — the engine died before finalizing,
    /// but the user said something and saw it on screen.
    pub fn on_voice_status(
        &mut self,
        listening: bool,
        available: bool,
        reason: &str,
    ) -> Vec<PaletteOutcome> {
        let was = self.voice.listening;
        self.voice.listening = listening;
        if !available {
            self.lines.push(Line::System(format!("⚠ voice: {reason}")));
            return Vec::new();
        }
        if listening {
            // Engine progress while live (model download, transcribing).
            self.voice.engine_note = reason.to_string();
            return Vec::new();
        }
        self.voice.engine_note.clear();
        if was && !listening {
            if !reason.is_empty() {
                self.lines.push(Line::System(format!("🎤 {reason}")));
            }
            let leftover = std::mem::take(&mut self.voice.interim);
            if !leftover.is_empty() {
                self.input.clear();
                return self.route(leftover, true);
            }
            if !self.voice.got_speech && reason.is_empty() {
                self.lines.push(Line::System(
                    "🎤 didn't hear anything — hold 🤙 to try again".into(),
                ));
            }
        }
        Vec::new()
    }

    /// Transcript from the kernel: interim text streams into the input line
    /// in real time; the final utterance replaces it and executes.
    pub fn on_voice_transcript(&mut self, text: String, is_final: bool) -> Vec<PaletteOutcome> {
        self.voice.got_speech = true;
        if is_final {
            self.voice.interim.clear();
            self.input.clear();
            self.route(text, true)
        } else {
            self.voice.interim = text.clone();
            self.input = text;
            Vec::new()
        }
    }

    /// Route a streamed AI chunk. Returns outcomes (e.g. repair prompts).
    pub fn on_chunk(&mut self, agent: AgentId, text: &str, done: bool) -> Vec<PaletteOutcome> {
        let mut out = Vec::new();
        if agent == AGENT_ASSISTANT {
            if let Some(Line::Assistant(s)) = self.lines.last_mut() {
                s.push_str(text);
            } else {
                self.lines.push(Line::Assistant(text.to_string()));
            }
            if done {
                self.assistant_streaming = false;
            }
        } else if agent == AGENT_APP_SMITH && self.smith.active {
            if text.starts_with('⚠') {
                self.lines.push(Line::System(text.to_string()));
                self.smith = Smith::default();
                return out;
            }
            self.smith.buf.push_str(text);
            if done {
                out.extend(self.finish_conjure());
            }
        }
        out
    }

    fn finish_conjure(&mut self) -> Vec<PaletteOutcome> {
        let raw = std::mem::take(&mut self.smith.buf);
        // Tolerate stray prose/fences: take the outermost {...} slice.
        let json = match (raw.find('{'), raw.rfind('}')) {
            (Some(a), Some(b)) if b > a => raw[a..=b].to_string(),
            _ => {
                self.lines
                    .push(Line::System("⚠ the App Smith returned no JSON".into()));
                self.smith = Smith::default();
                return Vec::new();
            }
        };
        match pmos_conjure::validate(&json) {
            Ok(doc) => {
                self.lines.push(Line::System(format!(
                    "✨ conjured {} {}",
                    doc.manifest.icon, doc.manifest.name
                )));
                self.smith = Smith::default();
                vec![PaletteOutcome::SpawnConjure(json)]
            }
            Err(errors) => {
                if self.smith.round < MAX_REPAIR_ROUNDS {
                    self.smith.round += 1;
                    self.smith.buf.clear();
                    self.lines.push(Line::System(format!(
                        "…repairing ({} issue{}, round {})",
                        errors.len(),
                        if errors.len() == 1 { "" } else { "s" },
                        self.smith.round
                    )));
                    let summary: Vec<String> = errors
                        .iter()
                        .map(|e| format!("- {} at {}: {} ({})", e.code, e.path, e.message, e.hint))
                        .collect();
                    let repair = format!(
                        "Your document failed validation:\n{}\n\nPrevious document:\n{}\n\nReturn the FULL corrected JSON document and nothing else.",
                        summary.join("\n"),
                        json
                    );
                    vec![PaletteOutcome::Prompt(AGENT_APP_SMITH, repair)]
                } else {
                    self.lines.push(Line::System(format!(
                        "⚠ conjuring failed after repairs: {}",
                        errors
                            .first()
                            .map(|e| e.message.clone())
                            .unwrap_or_default()
                    )));
                    self.smith = Smith::default();
                    Vec::new()
                }
            }
        }
    }

    /// One brain for typed and spoken input. `voice` changes only the
    /// fallback: unrecognized speech is conversational, so it goes to the
    /// assistant instead of an "unknown command" shrug.
    fn route(&mut self, raw: String, voice: bool) -> Vec<PaletteOutcome> {
        let text = raw.trim().to_string();
        if text.is_empty() {
            return Vec::new();
        }
        self.lines.push(Line::User(if voice {
            format!("🎤 {text}")
        } else {
            text.clone()
        }));

        // Assistant chat.
        if let Some(q) = text.strip_prefix('>') {
            self.assistant_streaming = true;
            self.lines.push(Line::Assistant(String::new()));
            return vec![PaletteOutcome::Prompt(AGENT_ASSISTANT, q.trim().into())];
        }

        // App conjuring.
        let lower = text.to_lowercase();
        if ["make ", "create ", "build ", "conjure "]
            .iter()
            .any(|p| lower.starts_with(p))
        {
            self.smith = Smith {
                buf: String::new(),
                active: true,
                round: 0,
            };
            self.lines.push(Line::System("🪄 conjuring…".into()));
            return vec![PaletteOutcome::Prompt(AGENT_APP_SMITH, text)];
        }

        // Commands. Spoken phrasing arrives as "open the terminal" — strip
        // launch verbs and articles before matching app names.
        let cmd = {
            let mut c = lower.trim_end_matches(['.', '!', '?']).trim();
            for verb in ["open ", "launch ", "start ", "show ", "go to "] {
                if let Some(rest) = c.strip_prefix(verb) {
                    c = rest;
                    break;
                }
            }
            c.strip_prefix("the ").unwrap_or(c).trim().to_string()
        };
        if cmd == "launcher" {
            self.open = false;
            return vec![PaletteOutcome::OpenLauncher];
        }
        if cmd == "demo" || cmd == "demo app" {
            self.lines
                .push(Line::System("✨ spawning the demo app".into()));
            return vec![PaletteOutcome::SpawnConjure(
                include_str!("../../pmos-conjure/examples/pomodoro.conjure.json").to_string(),
            )];
        }
        for kind in ALL {
            if kind.title().to_lowercase().contains(&cmd) {
                self.open = false;
                return vec![PaletteOutcome::Launch(kind)];
            }
        }
        if voice {
            self.assistant_streaming = true;
            self.lines.push(Line::Assistant(String::new()));
            return vec![PaletteOutcome::Prompt(AGENT_ASSISTANT, text)];
        }
        self.lines.push(Line::System(format!(
            "unknown command `{text}` — try an app name, `demo`, `> question`, or `make …`"
        )));
        Vec::new()
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Vec<PaletteOutcome> {
        if !self.open {
            return Vec::new();
        }
        let mut outcomes = Vec::new();
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("palette"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 64.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::window(ui.style())
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width((screen.width() * 0.5).clamp(360.0, 640.0));

                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.input)
                                .hint_text("app name · demo · > ask · make me a…")
                                .desired_width(f32::INFINITY)
                                .font(egui::TextStyle::Heading),
                        );
                        if self.focus_input {
                            resp.request_focus();
                            self.focus_input = false;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            let text = std::mem::take(&mut self.input);
                            outcomes.extend(self.route(text, false));
                            resp.request_focus();
                        }

                        if self.voice.listening {
                            let t = ui.input(|i| i.time);
                            let a = (0.55 + 0.45 * (t * 4.0).sin().abs()) as f32;
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    egui::Color32::from_rgb(0xff, 0x5c, 0x7a).gamma_multiply(a),
                                    "● 🎤 listening",
                                );
                                ui.weak(if !self.voice.engine_note.is_empty() {
                                    self.voice.engine_note.as_str()
                                } else if self.voice.interim.is_empty() {
                                    "— speak, your words appear live · Esc cancels"
                                } else {
                                    "— pause to run it"
                                });
                            });
                            ui.ctx().request_repaint(); // keep the pulse alive
                        }

                        ui.add_space(6.0);
                        egui::ScrollArea::vertical()
                            .max_height(260.0)
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                for line in &self.lines {
                                    match line {
                                        Line::User(t) => {
                                            ui.colored_label(theme::ACCENT_A, format!("❯ {t}"));
                                        }
                                        Line::Assistant(t) => {
                                            ui.label(if t.is_empty() { "…" } else { t.as_str() });
                                        }
                                        Line::System(t) => {
                                            ui.weak(t);
                                        }
                                    }
                                }
                                if self.smith.active && !self.smith.buf.is_empty() {
                                    ui.weak(format!(
                                        "🪄 writing the app… {} chars",
                                        self.smith.buf.len()
                                    ));
                                }
                            });
                        ui.add_space(2.0);
                        ui.weak("Esc closes · 🤙 tap or Ctrl+K toggles · 🤙 hold to speak");
                    });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.voice.listening {
                self.voice = Voice::default();
                self.input.clear();
                self.lines.push(Line::System("🎤 cancelled".into()));
                outcomes.push(PaletteOutcome::VoiceStop);
            } else {
                self.open = false;
            }
        }
        outcomes
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}
