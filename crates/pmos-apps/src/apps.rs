//! The built-in system applications (Architecture §5). Terminal, Files and
//! Notes are real VFS clients as of M6; each is drawn by the shell with
//! kernel access so every operation flows through capability-checked
//! syscalls — they remain the ABI's permanent integration tests.

use pmos_abi::{Capability, KernelApi, Pid, Reply, Syscall, AGENT_ASSISTANT};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppKind {
    Terminal,
    Files,
    Notes,
    RayTracer,
    HandTracker,
    Settings,
    Browser,
}

pub const ALL: [AppKind; 7] = [
    AppKind::Terminal,
    AppKind::Files,
    AppKind::Notes,
    AppKind::RayTracer,
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
            AppKind::RayTracer => "Ray Tracer",
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
            AppKind::RayTracer => "◇",
            AppKind::HandTracker => "✋",
            AppKind::Settings => "⚙",
            AppKind::Browser => "🌐",
        }
    }

    pub fn default_size(self) -> [f32; 2] {
        match self {
            AppKind::Terminal => [560.0, 380.0],
            AppKind::Files => [480.0, 420.0],
            AppKind::Notes => [640.0, 460.0],
            AppKind::RayTracer => [548.0, 480.0],
            AppKind::HandTracker => [370.0, 560.0],
            AppKind::Settings => [460.0, 420.0],
            AppKind::Browser => [640.0, 440.0],
        }
    }

    /// Extra capabilities delegated by the shell at launch (ABI 1.2 rule:
    /// only ones the shell itself holds). Notes is deliberately scoped to
    /// /notes — the scoping model's reference user.
    pub fn caps(self) -> Vec<Capability> {
        match self {
            AppKind::Terminal => vec![
                Capability::FsRead("/".into()),
                Capability::FsWrite("/".into()),
                Capability::AiPrompt,
                Capability::SysQuery,
            ],
            AppKind::Files => vec![
                Capability::FsRead("/".into()),
                Capability::FsWrite("/".into()),
            ],
            AppKind::Notes => vec![
                Capability::FsRead("/notes".into()),
                Capability::FsWrite("/notes".into()),
            ],
            AppKind::RayTracer => vec![Capability::SysQuery],
            AppKind::HandTracker => vec![Capability::InputRawHands],
            AppKind::Settings => vec![Capability::AiPrompt],
            AppKind::Browser => Vec::new(),
        }
    }
}

/// Actions an app hands back to the shell after its frame.
pub enum AppAction {
    LaunchConjure(String),
}

/// Per-window app state (the app's "process memory").
pub struct AppState {
    pub kind: AppKind,
    pub hand_tracker: crate::hand_tracker::HandTrackerState,
    // terminal
    term_input: String,
    term_log: Vec<String>,
    term_cwd: String,
    term_waiting_ai: bool,
    // files
    files_cwd: String,
    files_new_name: String,
    // notes
    notes_current: Option<String>,
    notes_buffer: String,
    notes_new_name: String,
    notes_status: String,
    notes_backlinks: Vec<String>,
    // ray tracer
    rt_bounces: u8,
    rt_animate: bool,
    // settings
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
            term_input: String::new(),
            term_log: vec!["Pseudo Motion OS terminal — `help` lists commands".to_string()],
            term_cwd: "/home".to_string(),
            term_waiting_ai: false,
            files_cwd: "/".to_string(),
            files_new_name: String::new(),
            notes_current: None,
            notes_buffer: String::new(),
            notes_new_name: String::new(),
            notes_status: String::new(),
            notes_backlinks: Vec::new(),
            rt_bounces: 3,
            rt_animate: true,
            ai_kind: 0,
            ai_base: String::new(),
            ai_model: "claude-sonnet-4-5".to_string(),
            ai_key: String::new(),
            ai_status: String::new(),
        }
    }

    /// Streamed assistant output routed to this app (terminal `>` mode).
    pub fn on_ai_chunk(&mut self, text: &str, done: bool) {
        if self.kind != AppKind::Terminal || !self.term_waiting_ai {
            return;
        }
        if let Some(last) = self.term_log.last_mut() {
            last.push_str(text);
        }
        if done {
            self.term_waiting_ai = false;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        match self.kind {
            AppKind::Browser => {
                ui.label("The browser app lands in milestone 8.");
                ui.weak("(iframe browsing in web mode, native webviews under Tauri)");
            }
            _ => {
                ui.weak("this app is shell-drawn");
            }
        }
    }

    // ---------------- terminal ----------------

    pub fn terminal_ui(
        &mut self,
        ui: &mut egui::Ui,
        kernel: &mut dyn KernelApi,
        pid: Pid,
    ) -> Option<AppAction> {
        let mut action = None;
        let log_height = ui.available_height() - 34.0;
        egui::ScrollArea::vertical()
            .max_height(log_height)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &self.term_log {
                    ui.monospace(line);
                }
            });
        ui.add_space(4.0);
        let hint = format!("{} $", self.term_cwd);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.term_input)
                .hint_text(hint)
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let cmd = std::mem::take(&mut self.term_input);
            action = self.run_command(&cmd, kernel, pid);
            resp.request_focus();
        }
        action
    }

    fn resolve(&self, arg: &str) -> String {
        if arg.starts_with('/') {
            arg.to_string()
        } else if self.term_cwd == "/" {
            format!("/{arg}")
        } else {
            format!("{}/{arg}", self.term_cwd)
        }
    }

    fn run_command(
        &mut self,
        cmd: &str,
        kernel: &mut dyn KernelApi,
        pid: Pid,
    ) -> Option<AppAction> {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return None;
        }
        self.term_log.push(format!("{} $ {cmd}", self.term_cwd));

        // Natural-language mode.
        if let Some(q) = cmd.strip_prefix('>') {
            self.term_waiting_ai = true;
            self.term_log.push(String::new());
            let _ = kernel.syscall(
                pid,
                Syscall::AiPrompt {
                    agent: AGENT_ASSISTANT,
                    msg: q.trim().to_string(),
                },
            );
            return None;
        }

        let mut parts = cmd.splitn(3, ' ');
        let verb = parts.next().unwrap_or_default();
        let arg1 = parts.next().unwrap_or_default().to_string();
        let rest = parts.next().unwrap_or_default().to_string();
        let mut out: Vec<String> = Vec::new();
        let mut action = None;

        match verb {
            "help" => out.push(
                "ls [dir] · cd <dir> · cat <file> · write <file> <text> · mkdir <dir> · rm <path> · apps · run <app> · fps · clear · about · > question"
                    .into(),
            ),
            "about" => out.push(format!("Pseudo Motion OS · ABI {:?}", pmos_abi::ABI_VERSION)),
            "clear" => {
                self.term_log.clear();
            }
            "ls" => {
                let path = if arg1.is_empty() {
                    self.term_cwd.clone()
                } else {
                    self.resolve(&arg1)
                };
                match kernel.syscall(pid, Syscall::FsList { path }) {
                    Ok(Reply::Entries(entries)) => {
                        if entries.is_empty() {
                            out.push("(empty)".into());
                        }
                        for e in entries {
                            out.push(if e.dir {
                                format!("  {}/", e.name)
                            } else {
                                format!("  {}  {}B", e.name, e.size)
                            });
                        }
                    }
                    _ => out.push("ls: no such directory".into()),
                }
            }
            "cd" => {
                let path = self.resolve(&arg1);
                match kernel.syscall(pid, Syscall::FsList { path: path.clone() }) {
                    Ok(Reply::Entries(_)) => self.term_cwd = path,
                    _ => out.push("cd: no such directory".into()),
                }
            }
            "cat" => match kernel.syscall(
                pid,
                Syscall::FsRead {
                    path: self.resolve(&arg1),
                },
            ) {
                Ok(Reply::Bytes(b)) => {
                    out.extend(String::from_utf8_lossy(&b).lines().map(String::from))
                }
                _ => out.push("cat: no such file".into()),
            },
            "write" => {
                let path = self.resolve(&arg1);
                match kernel.syscall(
                    pid,
                    Syscall::FsWrite {
                        path,
                        bytes: rest.into_bytes(),
                    },
                ) {
                    Ok(_) => out.push("written".into()),
                    Err(e) => out.push(format!("write failed: {e:?}")),
                }
            }
            "mkdir" => match kernel.syscall(
                pid,
                Syscall::FsMkdir {
                    path: self.resolve(&arg1),
                },
            ) {
                Ok(_) => out.push("created".into()),
                Err(e) => out.push(format!("mkdir failed: {e:?}")),
            },
            "rm" => match kernel.syscall(
                pid,
                Syscall::FsDelete {
                    path: self.resolve(&arg1),
                },
            ) {
                Ok(_) => out.push("removed".into()),
                Err(e) => out.push(format!("rm failed: {e:?}")),
            },
            "apps" => match kernel.syscall(
                pid,
                Syscall::FsList {
                    path: "/apps".into(),
                },
            ) {
                Ok(Reply::Entries(entries)) => {
                    if entries.is_empty() {
                        out.push("no conjured apps yet — try `make me a …` in the palette".into());
                    }
                    for e in entries {
                        out.push(format!("  {}", e.name));
                    }
                }
                _ => out.push("apps: /apps unavailable".into()),
            },
            "run" => {
                let path = if arg1.contains('/') {
                    self.resolve(&arg1)
                } else {
                    format!("/apps/{arg1}")
                };
                let path = if path.ends_with(".conjure") {
                    path
                } else {
                    format!("{path}.conjure")
                };
                match kernel.syscall(pid, Syscall::FsRead { path }) {
                    Ok(Reply::Bytes(b)) => {
                        action = Some(AppAction::LaunchConjure(
                            String::from_utf8_lossy(&b).to_string(),
                        ));
                        out.push("launching…".into());
                    }
                    _ => out.push("run: app not found (see `apps`)".into()),
                }
            }
            "fps" => match kernel.syscall(
                pid,
                Syscall::SysQuery {
                    path: "/sys/fps".into(),
                },
            ) {
                Ok(Reply::Bytes(b)) => {
                    out.push(format!("{} fps", String::from_utf8_lossy(&b).trim()))
                }
                _ => out.push("fps unavailable".into()),
            },
            other => out.push(format!("unknown command `{other}` (try `help`)")),
        }
        self.term_log.extend(out);
        if self.term_log.len() > 500 {
            let excess = self.term_log.len() - 500;
            self.term_log.drain(..excess);
        }
        action
    }

    // ---------------- files ----------------

    pub fn files_ui(
        &mut self,
        ui: &mut egui::Ui,
        kernel: &mut dyn KernelApi,
        pid: Pid,
    ) -> Option<AppAction> {
        let mut action = None;
        // Breadcrumbs.
        ui.horizontal(|ui| {
            if ui.button("⌂").on_hover_text("/").clicked() {
                self.files_cwd = "/".into();
            }
            let mut acc = String::new();
            let cwd = self.files_cwd.clone();
            for part in cwd.split('/').filter(|p| !p.is_empty()) {
                acc.push('/');
                acc.push_str(part);
                if ui.button(part).clicked() {
                    self.files_cwd = acc.clone();
                }
            }
        });
        ui.separator();

        let entries = match kernel.syscall(
            pid,
            Syscall::FsList {
                path: self.files_cwd.clone(),
            },
        ) {
            Ok(Reply::Entries(e)) => e,
            _ => {
                ui.weak("directory unavailable");
                return None;
            }
        };

        let mut delete: Option<String> = None;
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 40.0)
            .show(ui, |ui| {
                if entries.is_empty() {
                    ui.weak("(empty)");
                }
                for e in &entries {
                    let full = if self.files_cwd == "/" {
                        format!("/{}", e.name)
                    } else {
                        format!("{}/{}", self.files_cwd, e.name)
                    };
                    ui.horizontal(|ui| {
                        let label = if e.dir {
                            format!("📁 {}", e.name)
                        } else if e.name.ends_with(".conjure") {
                            format!("✨ {}", e.name)
                        } else if e.name.ends_with(".md") {
                            format!("📝 {}", e.name)
                        } else {
                            format!("📄 {}", e.name)
                        };
                        if ui.button(label).clicked() {
                            if e.dir {
                                self.files_cwd = full.clone();
                            } else if e.name.ends_with(".conjure") {
                                if let Ok(Reply::Bytes(b)) =
                                    kernel.syscall(pid, Syscall::FsRead { path: full.clone() })
                                {
                                    action = Some(AppAction::LaunchConjure(
                                        String::from_utf8_lossy(&b).to_string(),
                                    ));
                                }
                            }
                        }
                        if !e.dir {
                            ui.weak(format!("{} B", e.size));
                        }
                        if !full.starts_with("/sys")
                            && ui.small_button("🗑").on_hover_text("delete").clicked()
                        {
                            delete = Some(full.clone());
                        }
                    });
                }
            });
        if let Some(path) = delete {
            let _ = kernel.syscall(pid, Syscall::FsDelete { path });
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.files_new_name)
                    .hint_text("new folder name")
                    .desired_width(160.0),
            );
            if ui.button("＋ folder").clicked() && !self.files_new_name.trim().is_empty() {
                let path = if self.files_cwd == "/" {
                    format!("/{}", self.files_new_name.trim())
                } else {
                    format!("{}/{}", self.files_cwd, self.files_new_name.trim())
                };
                let _ = kernel.syscall(pid, Syscall::FsMkdir { path });
                self.files_new_name.clear();
            }
        });
        action
    }

    // ---------------- notes ----------------

    fn note_files(kernel: &mut dyn KernelApi, pid: Pid) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec!["/notes".to_string()];
        while let Some(dir) = stack.pop() {
            if let Ok(Reply::Entries(entries)) =
                kernel.syscall(pid, Syscall::FsList { path: dir.clone() })
            {
                for e in entries {
                    let full = format!("{dir}/{}", e.name);
                    if e.dir {
                        stack.push(full);
                    } else if e.name.ends_with(".md") {
                        out.push(full);
                    }
                }
            }
        }
        out.sort();
        out
    }

    fn note_title(path: &str) -> String {
        path.rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".md")
            .to_string()
    }

    fn open_note(&mut self, path: &str, kernel: &mut dyn KernelApi, pid: Pid) {
        match kernel.syscall(
            pid,
            Syscall::FsRead {
                path: path.to_string(),
            },
        ) {
            Ok(Reply::Bytes(b)) => self.notes_buffer = String::from_utf8_lossy(&b).to_string(),
            _ => self.notes_buffer = String::new(),
        }
        self.notes_current = Some(path.to_string());
        self.notes_status.clear();
        self.refresh_backlinks(kernel, pid);
    }

    fn save_note(&mut self, kernel: &mut dyn KernelApi, pid: Pid) {
        if let Some(path) = self.notes_current.clone() {
            let res = kernel.syscall(
                pid,
                Syscall::FsWrite {
                    path,
                    bytes: self.notes_buffer.clone().into_bytes(),
                },
            );
            self.notes_status = match res {
                Ok(_) => "saved".into(),
                Err(e) => format!("save failed: {e:?}"),
            };
            self.refresh_backlinks(kernel, pid);
        }
    }

    fn refresh_backlinks(&mut self, kernel: &mut dyn KernelApi, pid: Pid) {
        self.notes_backlinks.clear();
        let Some(current) = &self.notes_current else {
            return;
        };
        let me = Self::note_title(current);
        let needle = format!("[[{me}]]");
        for path in Self::note_files(kernel, pid) {
            if path == *current {
                continue;
            }
            if let Ok(Reply::Bytes(b)) = kernel.syscall(pid, Syscall::FsRead { path: path.clone() })
            {
                if String::from_utf8_lossy(&b).contains(&needle) {
                    self.notes_backlinks.push(path);
                }
            }
        }
    }

    fn wikilinks(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find("[[") {
            let after = &rest[start + 2..];
            let Some(end) = after.find("]]") else { break };
            let name = after[..end].split('|').next().unwrap_or("").trim();
            if !name.is_empty() && !out.contains(&name.to_string()) {
                out.push(name.to_string());
            }
            rest = &after[end + 2..];
        }
        out
    }

    pub fn notes_ui(
        &mut self,
        ui: &mut egui::Ui,
        kernel: &mut dyn KernelApi,
        pid: Pid,
        today: &str,
    ) {
        egui::Panel::left(egui::Id::new(("notes-list", pid.0)))
            .resizable(false)
            .exact_size(180.0)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.notes_new_name)
                            .hint_text("new note")
                            .desired_width(110.0),
                    );
                    if ui.button("＋").clicked() && !self.notes_new_name.trim().is_empty() {
                        let path =
                            format!("/notes/{}.md", self.notes_new_name.trim().replace(' ', "-"));
                        let _ = kernel.syscall(
                            pid,
                            Syscall::FsWrite {
                                path: path.clone(),
                                bytes: format!("# {}\n\n", self.notes_new_name.trim()).into_bytes(),
                            },
                        );
                        self.notes_new_name.clear();
                        self.open_note(&path, kernel, pid);
                    }
                });
                if ui.button("📅 today").clicked() {
                    let path = format!("/notes/daily/{today}.md");
                    if kernel
                        .syscall(pid, Syscall::FsRead { path: path.clone() })
                        .is_err()
                    {
                        let _ = kernel.syscall(
                            pid,
                            Syscall::FsWrite {
                                path: path.clone(),
                                bytes: format!("# {today}\n\n").into_bytes(),
                            },
                        );
                    }
                    self.open_note(&path, kernel, pid);
                }
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for path in Self::note_files(kernel, pid) {
                        let selected = self.notes_current.as_deref() == Some(path.as_str());
                        let label = Self::note_title(&path);
                        if ui.selectable_label(selected, label).clicked() {
                            self.open_note(&path, kernel, pid);
                        }
                    }
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let Some(current) = self.notes_current.clone() else {
                ui.weak("select or create a note — [[wikilinks]] connect them");
                return;
            };
            ui.horizontal(|ui| {
                ui.strong(Self::note_title(&current));
                if ui.button("💾 save").clicked() {
                    self.save_note(kernel, pid);
                }
                ui.weak(&self.notes_status);
            });
            let editor = ui.add(
                egui::TextEdit::multiline(&mut self.notes_buffer)
                    .desired_width(f32::INFINITY)
                    .desired_rows(12)
                    .font(egui::TextStyle::Monospace),
            );
            if editor.changed() {
                self.notes_status = "unsaved".into();
            }

            let links = Self::wikilinks(&self.notes_buffer);
            if !links.is_empty() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.weak("links:");
                    for name in links {
                        if ui.link(format!("[[{name}]]")).clicked() {
                            let path = format!("/notes/{}.md", name.replace(' ', "-"));
                            if kernel
                                .syscall(pid, Syscall::FsRead { path: path.clone() })
                                .is_err()
                            {
                                // Ghost note: following it creates it.
                                let _ = kernel.syscall(
                                    pid,
                                    Syscall::FsWrite {
                                        path: path.clone(),
                                        bytes: format!("# {name}\n\n").into_bytes(),
                                    },
                                );
                            }
                            self.save_note(kernel, pid);
                            self.open_note(&path, kernel, pid);
                        }
                    }
                });
            }
            if !self.notes_backlinks.is_empty() {
                ui.add_space(4.0);
                ui.separator();
                ui.weak("backlinks:");
                let backlinks = self.notes_backlinks.clone();
                ui.horizontal_wrapped(|ui| {
                    for path in backlinks {
                        if ui.link(Self::note_title(&path)).clicked() {
                            self.open_note(&path, kernel, pid);
                        }
                    }
                });
            }
        });
    }

    // ---------------- ray tracer ----------------

    /// The Whitted tracer's viewport window (Architecture §4.3): the compute
    /// pass renders continuously; this window just shows the texture and
    /// pushes control changes through the RtConfig syscall.
    pub fn ray_tracer_ui(
        &mut self,
        ui: &mut egui::Ui,
        kernel: &mut dyn KernelApi,
        pid: Pid,
        rt_tex: Option<egui::TextureId>,
    ) {
        match rt_tex {
            Some(tex) => {
                let w = ui.available_width().clamp(320.0, 512.0);
                let size = egui::vec2(w, w * 384.0 / 512.0);
                ui.add(
                    egui::Image::new(egui::load::SizedTexture::new(tex, size)).corner_radius(8.0),
                );
            }
            None => {
                ui.weak("renderer starting…");
            }
        }
        ui.add_space(6.0);
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("bounces");
            let mut b = self.rt_bounces as u32;
            changed |= ui.add(egui::Slider::new(&mut b, 1..=5)).changed();
            self.rt_bounces = b as u8;
            changed |= ui.checkbox(&mut self.rt_animate, "animate").changed();
        });
        ui.weak(
            "Whitted tracing on the GPU: mirror + glass spheres, checkered floor, hard shadows",
        );
        if changed {
            let _ = kernel.syscall(
                pid,
                Syscall::RtConfig {
                    bounces: self.rt_bounces,
                    animate: self.rt_animate,
                },
            );
        }
    }

    // ---------------- settings ----------------

    /// Settings window body — drawn by the shell so it can reach the kernel
    /// (the AI provider form issues `AiConfigure`).
    pub fn settings_ui(&mut self, ui: &mut egui::Ui, kernel: &mut dyn KernelApi, pid: Pid) {
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
                self.ai_status = match kernel.syscall(pid, Syscall::AiConfigure(cfg)) {
                    Ok(_) => {
                        "✓ saved — try `> hello` or `make me a timer` in the palette (✨)".into()
                    }
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
}
