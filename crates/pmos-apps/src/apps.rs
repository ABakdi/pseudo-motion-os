//! The built-in system applications. M2 ships them as functional stubs —
//! each gains its real engine in a later milestone (docs/Todo.md).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppKind {
    Terminal,
    Files,
    Notes,
    HandTracker,
    Settings,
    Browser,
}

pub const ALL: [AppKind; 6] = [
    AppKind::Terminal,
    AppKind::Files,
    AppKind::Notes,
    AppKind::HandTracker,
    AppKind::Settings,
    AppKind::Browser,
];

impl AppKind {
    pub fn title(self) -> &'static str {
        match self {
            AppKind::Terminal => "Terminal",
            AppKind::Files => "Files",
            AppKind::Notes => "Motion Notes",
            AppKind::HandTracker => "Hand Tracker",
            AppKind::Settings => "Settings",
            AppKind::Browser => "Browser",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            AppKind::Terminal => "🖥",
            AppKind::Files => "📁",
            AppKind::Notes => "📝",
            AppKind::HandTracker => "✋",
            AppKind::Settings => "⚙",
            AppKind::Browser => "🌐",
        }
    }

    pub fn default_size(self) -> [f32; 2] {
        match self {
            AppKind::Terminal => [560.0, 360.0],
            AppKind::Files => [520.0, 400.0],
            AppKind::Notes => [560.0, 420.0],
            AppKind::HandTracker => [370.0, 560.0],
            AppKind::Settings => [460.0, 420.0],
            AppKind::Browser => [640.0, 440.0],
        }
    }
}

/// Per-window app state (the app's "process memory").
pub struct AppState {
    pub kind: AppKind,
    pub hand_tracker: crate::hand_tracker::HandTrackerState,
    terminal_input: String,
    terminal_log: Vec<String>,
    notes_text: String,
    // Settings → AI provider form (write-only; keys never read back).
    ai_kind: u8,
    ai_base: String,
    ai_model: String,
    ai_key: String,
    ai_status: String,
}

impl AppState {
    pub fn new(kind: AppKind) -> Self {
        Self {
            kind,
            hand_tracker: crate::hand_tracker::HandTrackerState::new(),
            terminal_input: String::new(),
            terminal_log: vec![
                "Pseudo Motion OS — pseudo terminal".to_string(),
                "type `help` to see what works already".to_string(),
            ],
            notes_text: String::new(),
            ai_kind: 0,
            ai_base: String::new(),
            ai_model: "claude-sonnet-4-5".to_string(),
            ai_key: String::new(),
            ai_status: String::new(),
        }
    }

    /// Settings window body — drawn by the shell so it can reach the kernel
    /// (the AI provider form issues `AiConfigure`).
    pub fn settings_ui(
        &mut self,
        ui: &mut egui::Ui,
        kernel: &mut dyn pmos_abi::KernelApi,
        pid: pmos_abi::Pid,
    ) {
        ui.heading("AI provider");
        ui.add_space(4.0);
        egui::Grid::new("ai-cfg")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Provider");
                egui::ComboBox::from_id_salt("ai-provider")
                    .selected_text(if self.ai_kind == 0 {
                        "Anthropic"
                    } else {
                        "OpenAI-compatible"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.ai_kind, 0, "Anthropic");
                        ui.selectable_value(
                            &mut self.ai_kind,
                            1,
                            "OpenAI-compatible (incl. Ollama)",
                        );
                    });
                ui.end_row();

                ui.label("Base URL");
                ui.add(
                    egui::TextEdit::singleline(&mut self.ai_base)
                        .hint_text(if self.ai_kind == 0 {
                            "default: api.anthropic.com"
                        } else {
                            "e.g. http://localhost:11434"
                        })
                        .desired_width(240.0),
                );
                ui.end_row();

                ui.label("Model");
                ui.add(
                    egui::TextEdit::singleline(&mut self.ai_model)
                        .hint_text("model id")
                        .desired_width(240.0),
                );
                ui.end_row();

                ui.label("API key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.ai_key)
                        .password(true)
                        .hint_text("stored locally, never displayed again")
                        .desired_width(240.0),
                );
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Save provider").clicked() {
                let cfg = pmos_abi::AiProviderConfig {
                    kind: self.ai_kind,
                    base_url: self.ai_base.trim().to_string(),
                    model: self.ai_model.trim().to_string(),
                    api_key: self.ai_key.trim().to_string(),
                };
                self.ai_status = match kernel.syscall(pid, pmos_abi::Syscall::AiConfigure(cfg)) {
                    Ok(_) => "✓ saved — try `> hello` or `make me a timer` in the palette (Ctrl+K)"
                        .into(),
                    Err(e) => format!("⚠ {e:?}"),
                };
                self.ai_key.clear();
            }
            ui.weak(&self.ai_status);
        });
        ui.add_space(10.0);
        ui.separator();
        ui.weak("Gestures are tuned in the Hand Tracker app.");
        ui.weak("Stage camera: drag = orbit · wheel = zoom · Home = reset");
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        match self.kind {
            AppKind::Terminal => self.terminal_ui(ui),
            AppKind::Files => {
                ui.label("The virtual file system lands in milestone 6.");
                ui.add_space(6.0);
                for entry in ["/home", "/apps", "/notes", "/sys"] {
                    ui.weak(format!("📁 {entry}"));
                }
            }
            AppKind::Notes => {
                ui.label("Motion Notes — full editor, wikilinks and the 3D graph land in M6.");
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.notes_text)
                        .hint_text("scratch space (not persisted yet)")
                        .desired_width(f32::INFINITY)
                        .desired_rows(12),
                );
            }
            // Drawn by the shell (needs kernel for AiConfigure).
            AppKind::Settings => {
                ui.weak("settings are shell-drawn");
            }
            AppKind::Browser => {
                ui.label("The browser app lands in milestone 8.");
                ui.weak("(iframe browsing in web mode, native webviews under Tauri)");
            }
            // Drawn by the shell (needs kernel + platform textures); this
            // arm is a defensive fallback only.
            AppKind::HandTracker => {
                ui.weak("hand tracker is shell-drawn");
            }
        }
    }

    fn terminal_ui(&mut self, ui: &mut egui::Ui) {
        let log_height = ui.available_height() - 32.0;
        egui::ScrollArea::vertical()
            .max_height(log_height)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &self.terminal_log {
                    ui.monospace(line);
                }
            });
        ui.add_space(4.0);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.terminal_input)
                .hint_text("$")
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let cmd = std::mem::take(&mut self.terminal_input);
            self.terminal_log.push(format!("$ {cmd}"));
            let reply = match cmd.trim() {
                "help" => {
                    Some("commands: help, about, clear — the real parser lands in M6".to_string())
                }
                "about" => Some(format!(
                    "Pseudo Motion OS · ABI {:?}",
                    pmos_abi::ABI_VERSION
                )),
                "clear" => {
                    self.terminal_log.clear();
                    None
                }
                "" => None,
                other => Some(format!("unknown command: {other} (try `help`)")),
            };
            if let Some(reply) = reply {
                self.terminal_log.push(reply);
            }
            resp.request_focus();
        }
    }
}
