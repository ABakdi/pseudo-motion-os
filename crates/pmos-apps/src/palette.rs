//! The command palette (UI spec §2.4): one surface, three modes —
//! fuzzy commands, `>` assistant chat, and "make/create …" app conjuring
//! through the App Smith repair loop (AI System spec §4).

use crate::apps::{AppKind, ALL};
use crate::theme;
use pmos_abi::{AgentId, AGENT_APP_SMITH, AGENT_ASSISTANT};

const MAX_REPAIR_ROUNDS: u8 = 2;
/// Tool-call budget per user request (AI System spec §3).
const MAX_TOOL_ROUNDS: u8 = 4;

/// A parsed `@@tool` invocation from an assistant reply (AI System spec §3).
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub tool: String,
    pub args: serde_json::Value,
}

/// Find a trailing `@@tool {...}` line in an assistant reply. Returns the
/// reply with that line removed (what the user should see) and the call.
/// Tolerates models wrapping the line in code fences.
fn extract_tool_call(text: &str) -> Option<(String, ToolCall)> {
    for line in text.lines().rev() {
        let t = line.trim().trim_matches('`').trim();
        let Some(json) = t.strip_prefix("@@tool ") else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json.trim()) else {
            continue;
        };
        let tool = v.get("tool")?.as_str()?.to_string();
        let args = v.get("args").cloned().unwrap_or(serde_json::json!({}));
        let visible = text
            .lines()
            .filter(|l| *l != line)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        return Some((visible, ToolCall { tool, args }));
    }
    None
}

/// One-line arg summary for the transparency log ("🔧 fs_read /notes/x.md").
fn compact_args(args: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(map) = args.as_object() {
        for (k, v) in map {
            let s = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            let s = if s.chars().count() > 40 {
                format!("{}…", s.chars().take(40).collect::<String>())
            } else {
                s
            };
            parts.push(if k == "path" || k == "name" {
                s
            } else {
                format!("{k}={s}")
            });
        }
    }
    parts.join(" ")
}

pub enum PaletteOutcome {
    Launch(AppKind),
    OpenLauncher,
    /// Validated Conjure JSON ready to spawn.
    SpawnConjure(String),
    /// Send a prompt to an agent (assistant chat or App Smith repair).
    Prompt(AgentId, String),
    /// Cancel speech capture (Esc while listening).
    VoiceStop,
    /// The assistant asked for a tool run — the shell executes it via
    /// capability-checked syscalls and reports back with `tool_result`.
    ToolCall(ToolCall),
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
    /// Tool calls consumed by the current assistant request (budgeted).
    tool_rounds: u8,
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
            tool_rounds: 0,
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
            // '\r' deltas replace the line (ABI 1.6 — transient progress).
            if let Some(Line::Assistant(s)) = self.lines.last_mut() {
                match text.strip_prefix('\r') {
                    Some(rest) => *s = rest.to_string(),
                    None => s.push_str(text),
                }
            } else {
                self.lines
                    .push(Line::Assistant(text.trim_start_matches('\r').to_string()));
            }
            if done {
                self.assistant_streaming = false;
                // Did the reply end in a tool call? Strip it from the visible
                // line, log it (nothing acts invisibly), hand it to the shell.
                let call = match self.lines.last_mut() {
                    Some(Line::Assistant(s)) => extract_tool_call(s).map(|(visible, call)| {
                        *s = visible;
                        call
                    }),
                    _ => None,
                };
                if let Some(call) = call {
                    if matches!(self.lines.last(), Some(Line::Assistant(s)) if s.is_empty()) {
                        self.lines.pop();
                    }
                    if self.tool_rounds >= MAX_TOOL_ROUNDS {
                        self.lines.push(Line::System(
                            "⚠ tool budget exhausted (4 calls per request)".into(),
                        ));
                    } else {
                        self.tool_rounds += 1;
                        self.lines.push(Line::System(format!(
                            "🔧 {} {}",
                            call.tool,
                            compact_args(&call.args)
                        )));
                        out.push(PaletteOutcome::ToolCall(call));
                    }
                } else {
                    self.tool_rounds = 0;
                }
            }
        } else if agent == AGENT_APP_SMITH && self.smith.active {
            if text.starts_with('⚠') {
                self.lines.push(Line::System(text.to_string()));
                self.smith = Smith::default();
                return out;
            }
            match text.strip_prefix('\r') {
                Some(rest) => self.smith.buf = rest.to_string(),
                None => self.smith.buf.push_str(text),
            }
            if done {
                out.extend(self.finish_conjure());
            }
        }
        out
    }

    /// Report a tool run's outcome back to the assistant and let it continue.
    pub fn tool_result(&mut self, tool: &str, ok: bool, result: &str) -> Vec<PaletteOutcome> {
        let payload =
            serde_json::json!({ "tool": tool, "ok": ok, "result": result }).to_string();
        self.assistant_streaming = true;
        self.lines.push(Line::Assistant(String::new()));
        vec![PaletteOutcome::Prompt(
            AGENT_ASSISTANT,
            format!("@@tool_result {payload}"),
        )]
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
            self.tool_rounds = 0;
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
            self.tool_rounds = 0;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_trailing_tool_call() {
        let text = "Let me check your notes.\n@@tool {\"tool\":\"fs_list\",\"args\":{\"path\":\"/notes\"}}";
        let (visible, call) = extract_tool_call(text).expect("tool call");
        assert_eq!(visible, "Let me check your notes.");
        assert_eq!(call.tool, "fs_list");
        assert_eq!(call.args["path"], "/notes");
    }

    #[test]
    fn tolerates_code_fences_and_missing_args() {
        let text = "`@@tool {\"tool\":\"sys_query\"}`";
        let (visible, call) = extract_tool_call(text).expect("tool call");
        assert!(visible.is_empty());
        assert_eq!(call.tool, "sys_query");
        assert!(call.args.as_object().unwrap().is_empty());
    }

    #[test]
    fn plain_replies_are_not_tool_calls() {
        assert!(extract_tool_call("The fps is around 60.").is_none());
        assert!(extract_tool_call("mentions @@tool but not as a call line").is_none());
    }
}
