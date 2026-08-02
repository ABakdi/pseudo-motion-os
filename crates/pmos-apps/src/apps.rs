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
            AppKind::Settings => vec![
                Capability::AiPrompt,
                // Appearance: pick the background (SysQuery gates the
                // Background syscall) and persist choices under /settings.
                Capability::SysQuery,
                Capability::FsRead("/settings".into()),
                Capability::FsWrite("/settings".into()),
                // Stage section: spawn/clear objects (ABI 1.8).
                Capability::PhysSpawn,
            ],
            AppKind::Browser => Vec::new(),
        }
    }
}

/// Actions an app hands back to the shell after its frame.
pub enum AppAction {
    LaunchConjure(String),
}

/// In-browser AI performance tiers (WebLLM prebuilt model ids, ABI 1.6).
/// Approximate one-time download sizes; the browser caches the model.
const WEBLLM_MODELS: &[(&str, &str)] = &[
    ("Fast — ~0.6 GB download", "Qwen2.5-0.5B-Instruct-q4f16_1-MLC"),
    ("Balanced — ~0.9 GB download", "Llama-3.2-1B-Instruct-q4f16_1-MLC"),
    ("Quality — ~1.9 GB download", "Qwen2.5-3B-Instruct-q4f16_1-MLC"),
];

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
    files_grid: bool,
    files_selected: Option<String>,
    /// (path, rendered preview text) for the selected file.
    files_preview: Option<(String, String)>,
    // notes
    notes_current: Option<String>,
    notes_buffer: String,
    notes_new_name: String,
    notes_status: String,
    notes_backlinks: Vec<String>,
    // browser
    browser_input: String,
    browser_url: String,
    // ray tracer
    rt_bounces: u8,
    rt_animate: bool,
    // settings
    ai_kind: u8,
    ai_base: String,
    ai_model: String,
    ai_key: String,
    // appearance (Settings → Appearance, UI spec §6.1)
    app_bg: u8,
    app_scheme: u8,
    appearance_loaded: bool,
    /// Machine-probed in-browser LLM tier (/sys/llm_tier); None = not read.
    llm_tier: Option<u8>,
    // voice (Settings → Voice)
    voice_model: String,
    voice_loaded: bool,
    // face (Settings → Face, M10 opt-in)
    face_enabled: bool,
    face_loaded: bool,
    // stage (Settings → Stage, ABI 1.8)
    stage_az: f32,
    stage_el: f32,
    stage_intensity: f32,
    stage_ambient: f32,
    stage_n: u32,
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
            files_grid: true,
            files_selected: None,
            files_preview: None,
            notes_current: None,
            notes_buffer: String::new(),
            notes_new_name: String::new(),
            notes_status: String::new(),
            notes_backlinks: Vec::new(),
            browser_input: "https://en.wikipedia.org".to_string(),
            // Load a known-embeddable page immediately: an empty white frame
            // reads as "broken" (most big sites refuse iframes via
            // X-Frame-Options, which we cannot detect cross-origin).
            browser_url: "https://en.wikipedia.org".to_string(),
            rt_bounces: 3,
            rt_animate: true,
            ai_kind: 2,
            ai_base: String::new(),
            ai_model: WEBLLM_MODELS[1].1.to_string(),
            ai_key: String::new(),
            app_bg: 0,
            app_scheme: 0,
            appearance_loaded: false,
            llm_tier: None,
            voice_model: "tiny".to_string(),
            voice_loaded: false,
            face_enabled: false,
            face_loaded: false,
            stage_az: 215.0,
            stage_el: 60.0,
            stage_intensity: 1.0,
            stage_ambient: 0.22,
            stage_n: 0,
            ai_status: String::new(),
        }
    }

    /// Streamed assistant output routed to this app (terminal `>` mode).
    pub fn on_ai_chunk(&mut self, text: &str, done: bool) {
        if self.kind != AppKind::Terminal || !self.term_waiting_ai {
            return;
        }
        if let Some(last) = self.term_log.last_mut() {
            // '\r' deltas replace the line (ABI 1.6 — transient progress).
            match text.strip_prefix('\r') {
                Some(rest) => *last = rest.to_string(),
                None => last.push_str(text),
            }
        }
        if done {
            self.term_waiting_ai = false;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.weak("this app is shell-drawn");
    }

    // ---------------- browser ----------------

    /// The nested browser (UI heritage §4.8): the chrome is egui; the page
    /// itself is a real DOM iframe the platform overlays on the content rect
    /// returned here. Honest caveat surfaced in the UI: many sites refuse to
    /// be iframed (X-Frame-Options/CSP) — real browsing arrives with Tauri's
    /// native webviews.
    /// Point the Browser app at a URL (AI web_open tool).
    pub fn browser_open(&mut self, url: &str) {
        self.browser_input = url.to_string();
        self.browser_url = url.to_string();
    }

    pub fn browser_ui(&mut self, ui: &mut egui::Ui) -> Option<(String, egui::Rect)> {
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.browser_input)
                    .hint_text("https://…")
                    .desired_width(ui.available_width() - 60.0),
            );
            let go = ui.button("Go").clicked()
                || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go {
                let mut url = self.browser_input.trim().to_string();
                if !url.starts_with("http") {
                    url = format!("https://{url}");
                }
                self.browser_url = url;
            }
        });
        ui.horizontal(|ui| {
            ui.weak("quick:");
            for (name, url) in [
                ("Wikipedia", "https://en.wikipedia.org"),
                ("Hacker News", "https://news.ycombinator.com"),
                ("OpenStreetMap", "https://www.openstreetmap.org/export/embed.html"),
                ("MDN", "https://developer.mozilla.org"),
            ] {
                if ui.small_button(name).clicked() {
                    self.browser_input = url.to_string();
                    self.browser_url = url.to_string();
                }
            }
            ui.weak("· blank page = that site refuses embedding");
        });
        ui.add_space(2.0);
        let rect = ui.available_rect_before_wrap();
        // Reserve the area so the window keeps its size.
        ui.allocate_rect(rect, egui::Sense::hover());
        if self.browser_url.is_empty() {
            None
        } else {
            Some((self.browser_url.clone(), rect))
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
                "ls [dir] · cd <dir> · cat <file> · write <file> <text> · mkdir <dir> · rm <path> · apps · run <app> · voice <query> · fps · clear · about · > question"
                    .into(),
            ),
            "about" => out.push(format!("Pseudo Motion OS · ABI {:?}", pmos_abi::ABI_VERSION)),
            "voice" => {
                // Search the persisted voice transcripts (Voice Kit spec §4).
                let needle = format!("{arg1} {rest}").trim().to_lowercase();
                if needle.is_empty() {
                    out.push("voice <query> — search your voice history".into());
                } else {
                    let mut hits = 0;
                    if let Ok(Reply::Entries(days)) =
                        kernel.syscall(pid, Syscall::FsList { path: "/voice".into() })
                    {
                        for day in days.iter().filter(|d| d.dir) {
                            let dir = format!("/voice/{}", day.name);
                            if let Ok(Reply::Entries(files)) =
                                kernel.syscall(pid, Syscall::FsList { path: dir.clone() })
                            {
                                for f in files.iter().filter(|f| !f.dir) {
                                    let path = format!("{dir}/{}", f.name);
                                    if let Ok(Reply::Bytes(b)) =
                                        kernel.syscall(pid, Syscall::FsRead { path })
                                    {
                                        if let Ok(v) =
                                            serde_json::from_slice::<serde_json::Value>(&b)
                                        {
                                            for seg in
                                                v["segments"].as_array().into_iter().flatten()
                                            {
                                                let text =
                                                    seg["segments"].as_str().unwrap_or(
                                                        seg["text"].as_str().unwrap_or(""),
                                                    );
                                                if text.to_lowercase().contains(&needle) {
                                                    out.push(format!(
                                                        "{} · {}",
                                                        day.name, text
                                                    ));
                                                    hits += 1;
                                                    if hits >= 20 {
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if hits == 0 {
                        out.push("no matches".into());
                    }
                }
            }
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

    fn file_icon(name: &str, dir: bool) -> &'static str {
        if dir {
            return "📁";
        }
        match name.rsplit('.').next().unwrap_or("") {
            "conjure" => "✨",
            "md" => "📝",
            "json" => "⚙",
            "txt" => "📄",
            "png" | "jpg" | "jpeg" | "gif" | "svg" => "🖼",
            _ => "📄",
        }
    }

    /// Load (or refresh) the preview for the selected path.
    fn load_preview(&mut self, kernel: &mut dyn KernelApi, pid: Pid, path: &str) {
        let text = match kernel.syscall(pid, Syscall::FsRead { path: path.into() }) {
            Ok(Reply::Bytes(b)) => match String::from_utf8(b.clone()) {
                Ok(mut s) => {
                    if s.chars().count() > 2400 {
                        s = s.chars().take(2400).collect::<String>() + "\n…";
                    }
                    s
                }
                Err(_) => format!("(binary · {} B)", b.len()),
            },
            Err(e) => format!("(unreadable: {e:?})"),
            Ok(_) => String::new(),
        };
        self.files_preview = Some((path.to_string(), text));
    }

    /// Select or open (double-click) an entry from either view.
    #[allow(clippy::too_many_arguments)]
    fn files_entry_interact(
        &mut self,
        resp: &egui::Response,
        kernel: &mut dyn KernelApi,
        pid: Pid,
        full: &str,
        dir: bool,
        action: &mut Option<AppAction>,
    ) {
        if resp.clicked() {
            if dir {
                // Single click navigates folders — the common expectation.
                self.files_cwd = full.to_string();
                self.files_selected = None;
                self.files_preview = None;
            } else {
                self.files_selected = Some(full.to_string());
                self.load_preview(kernel, pid, full);
            }
        }
        if resp.double_clicked() && !dir && full.ends_with(".conjure") {
            if let Ok(Reply::Bytes(b)) = kernel.syscall(pid, Syscall::FsRead { path: full.into() })
            {
                *action = Some(AppAction::LaunchConjure(
                    String::from_utf8_lossy(&b).to_string(),
                ));
            }
        }
        // Right-click / middle-pinch context menu (UI spec §3.3).
        resp.context_menu(|ui| {
            if dir {
                if ui.button("Open").clicked() {
                    self.files_cwd = full.to_string();
                    self.files_selected = None;
                    self.files_preview = None;
                    ui.close();
                }
            } else {
                if full.ends_with(".conjure") && ui.button("▶ Launch").clicked() {
                    if let Ok(Reply::Bytes(b)) =
                        kernel.syscall(pid, Syscall::FsRead { path: full.into() })
                    {
                        *action = Some(AppAction::LaunchConjure(
                            String::from_utf8_lossy(&b).to_string(),
                        ));
                    }
                    ui.close();
                }
                if ui.button("Preview").clicked() {
                    self.files_selected = Some(full.to_string());
                    self.load_preview(kernel, pid, full);
                    ui.close();
                }
            }
            if ui.button("Copy path").clicked() {
                ui.ctx().copy_text(full.to_string());
                ui.close();
            }
            if !full.starts_with("/sys") && ui.button("🗑 Delete").clicked() {
                let _ = kernel.syscall(pid, Syscall::FsDelete { path: full.into() });
                if self.files_selected.as_deref() == Some(full) {
                    self.files_selected = None;
                    self.files_preview = None;
                }
                ui.close();
            }
        });
    }

    pub fn files_ui(
        &mut self,
        ui: &mut egui::Ui,
        kernel: &mut dyn KernelApi,
        pid: Pid,
    ) -> Option<AppAction> {
        let mut action = None;

        // ---- Places sidebar ----
        egui::Panel::left(egui::Id::new(("files-places", pid.0)))
            .resizable(false)
            .exact_size(108.0)
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                ui.weak("PLACES");
                for (icon, name, path) in [
                    ("⌂", "Home", "/"),
                    ("📝", "Notes", "/notes"),
                    ("✨", "Apps", "/apps"),
                    ("⚙", "Settings", "/settings"),
                    ("🖥", "System", "/sys"),
                ] {
                    let here = self.files_cwd == path
                        || self.files_cwd.starts_with(&format!("{path}/")) && path != "/";
                    if ui.selectable_label(here, format!("{icon} {name}")).clicked() {
                        self.files_cwd = path.into();
                        self.files_selected = None;
                        self.files_preview = None;
                    }
                }
            });

        // ---- Preview sidebar ----
        if let Some(sel) = self.files_selected.clone() {
            egui::Panel::right(egui::Id::new(("files-preview", pid.0)))
                .resizable(true)
                .default_size(200.0)
                .show_inside(ui, |ui| {
                    ui.add_space(4.0);
                    let name = sel.rsplit('/').next().unwrap_or(&sel);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(Self::file_icon(name, false)).size(22.0),
                        );
                        ui.heading(name);
                    });
                    ui.weak(&sel);
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        if sel.ends_with(".conjure") && ui.button("▶ Launch").clicked() {
                            if let Ok(Reply::Bytes(b)) =
                                kernel.syscall(pid, Syscall::FsRead { path: sel.clone() })
                            {
                                action = Some(AppAction::LaunchConjure(
                                    String::from_utf8_lossy(&b).to_string(),
                                ));
                            }
                        }
                        if !sel.starts_with("/sys") && ui.button("🗑 Delete").clicked() {
                            let _ = kernel.syscall(pid, Syscall::FsDelete { path: sel.clone() });
                            self.files_selected = None;
                            self.files_preview = None;
                        }
                    });
                    ui.separator();
                    if let Some((_, text)) = &self.files_preview {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(text).monospace().size(11.5),
                                )
                                .wrap(),
                            );
                        });
                    }
                });
        }

        // ---- Toolbar ----
        ui.horizontal(|ui| {
            let up = self.files_cwd.rsplit_once('/').map(|(p, _)| {
                if p.is_empty() { "/".to_string() } else { p.to_string() }
            });
            if ui
                .add_enabled(self.files_cwd != "/", egui::Button::new("⬆"))
                .on_hover_text("up")
                .clicked()
            {
                if let Some(parent) = up {
                    self.files_cwd = parent;
                    self.files_selected = None;
                    self.files_preview = None;
                }
            }
            if ui.button("⌂").on_hover_text("/").clicked() {
                self.files_cwd = "/".into();
            }
            let mut acc = String::new();
            let cwd = self.files_cwd.clone();
            for part in cwd.split('/').filter(|p| !p.is_empty()) {
                acc.push('/');
                acc.push_str(part);
                ui.weak("›");
                if ui.button(part).clicked() {
                    self.files_cwd = acc.clone();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let grid = self.files_grid;
                if ui.selectable_label(!grid, "☰").on_hover_text("list").clicked() {
                    self.files_grid = false;
                }
                if ui.selectable_label(grid, "⊞").on_hover_text("grid").clicked() {
                    self.files_grid = true;
                }
            });
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

        // ---- Entries (grid or list) ----
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 36.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if entries.is_empty() {
                    ui.weak("(empty)");
                }
                let cwd = self.files_cwd.clone();
                let full_of = |name: &str| {
                    if cwd == "/" {
                        format!("/{name}")
                    } else {
                        format!("{cwd}/{name}")
                    }
                };
                if self.files_grid {
                    ui.horizontal_wrapped(|ui| {
                        for e in &entries {
                            let full = full_of(&e.name);
                            let selected = self.files_selected.as_deref() == Some(&full);
                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(86.0, 78.0),
                                egui::Sense::click(),
                            );
                            if selected || resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    8.0,
                                    if selected {
                                        crate::theme::accent_a().gamma_multiply(0.16)
                                    } else {
                                        egui::Color32::from_white_alpha(8)
                                    },
                                );
                            }
                            ui.painter().text(
                                rect.center_top() + egui::vec2(0.0, 26.0),
                                egui::Align2::CENTER_CENTER,
                                Self::file_icon(&e.name, e.dir),
                                egui::FontId::proportional(26.0),
                                crate::theme::INK,
                            );
                            let mut label = e.name.clone();
                            if label.chars().count() > 11 {
                                label = label.chars().take(10).collect::<String>() + "…";
                            }
                            ui.painter().text(
                                rect.center_bottom() - egui::vec2(0.0, 14.0),
                                egui::Align2::CENTER_CENTER,
                                label,
                                egui::FontId::proportional(11.5),
                                crate::theme::INK,
                            );
                            let resp = resp.on_hover_text(&e.name);
                            self.files_entry_interact(
                                &resp, kernel, pid, &full, e.dir, &mut action,
                            );
                        }
                    });
                } else {
                    for e in &entries {
                        let full = full_of(&e.name);
                        let selected = self.files_selected.as_deref() == Some(&full);
                        ui.horizontal(|ui| {
                            let resp = ui.selectable_label(
                                selected,
                                format!("{} {}", Self::file_icon(&e.name, e.dir), e.name),
                            );
                            if !e.dir {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| ui.weak(format!("{} B", e.size)),
                                );
                            }
                            self.files_entry_interact(
                                &resp, kernel, pid, &full, e.dir, &mut action,
                            );
                        });
                    }
                }
            });

        // ---- Footer: create ----
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.files_new_name)
                    .hint_text("name…")
                    .desired_width(150.0),
            );
            let name = self.files_new_name.trim().to_string();
            let target = |n: &str| {
                if self.files_cwd == "/" {
                    format!("/{n}")
                } else {
                    format!("{}/{n}", self.files_cwd)
                }
            };
            if ui.button("＋ folder").clicked() && !name.is_empty() {
                let _ = kernel.syscall(pid, Syscall::FsMkdir { path: target(&name) });
                self.files_new_name.clear();
            }
            if ui.button("＋ file").clicked() && !name.is_empty() {
                let n = if name.contains('.') { name.clone() } else { format!("{name}.md") };
                let _ = kernel.syscall(
                    pid,
                    Syscall::FsWrite {
                        path: target(&n),
                        bytes: Vec::new(),
                    },
                );
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
            .resizable(true)
            .default_size(180.0)
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
                    .selected_text(match self.ai_kind {
                        2 => "In-browser (free)",
                        0 => "Anthropic",
                        _ => "OpenAI-compatible",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.ai_kind, 2, "In-browser (free, no key)");
                        ui.selectable_value(&mut self.ai_kind, 0, "Anthropic");
                        ui.selectable_value(
                            &mut self.ai_kind,
                            1,
                            "OpenAI-compatible (incl. Ollama)",
                        );
                    });
                ui.end_row();

                if self.ai_kind == 2 {
                    // Performance tiers instead of free-form model/key fields.
                    // The platform probes RAM + GPU headroom at boot and the
                    // fitting tier is marked (user request: match the machine).
                    let tier = *self.llm_tier.get_or_insert_with(|| {
                        match kernel.syscall(
                            pid,
                            Syscall::SysQuery {
                                path: "/sys/llm_tier".into(),
                            },
                        ) {
                            Ok(pmos_abi::Reply::Bytes(b)) => {
                                String::from_utf8_lossy(&b).trim().parse().unwrap_or(1)
                            }
                            _ => 1,
                        }
                    });
                    if !WEBLLM_MODELS.iter().any(|(_, id)| *id == self.ai_model) {
                        self.ai_model = WEBLLM_MODELS[tier as usize].1.to_string();
                    }
                    ui.label("Performance");
                    ui.vertical(|ui| {
                        for (i, (label, id)) in WEBLLM_MODELS.iter().enumerate() {
                            let text = if i as u8 == tier {
                                format!("{label} · ★ fits this machine")
                            } else if i as u8 > tier {
                                format!("{label} · may not fit")
                            } else {
                                label.to_string()
                            };
                            ui.radio_value(&mut self.ai_model, id.to_string(), text);
                        }
                    });
                    ui.end_row();
                } else {
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
                }
            });
        if self.ai_kind == 2 {
            ui.add_space(2.0);
            ui.weak("Runs on your GPU, in the browser — free, private, works offline once");
            ui.weak("the model is downloaded (first reply) and cached. Bigger = smarter.");
        }
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
                    Ok(_) if self.ai_kind == 2 => {
                        "✓ saved — the first reply downloads the model (watch the palette)".into()
                    }
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
        self.appearance_ui(ui, kernel, pid);
        ui.add_space(10.0);
        ui.separator();
        self.voice_ui(ui, kernel, pid);
        ui.add_space(10.0);
        ui.separator();
        self.stage_ui(ui, kernel, pid);
        ui.add_space(10.0);
        ui.separator();
        ui.weak("Gestures are tuned in the Hand Tracker app.");
        ui.weak("Mouse: drag = orbit · wheel = zoom · shift/middle-drag = pan · click-drag object = grab · Home = reset");
        ui.weak("Hands: 👌 click/grab objects (tap = select) · ✊ orbit + drag windows · ✌ scroll/zoom (edits selection) · two ✋ = zoom");
        ui.weak("Signs: ✋ hold = voice on/off · ✋ push = cancel · ☝ hold (near chin) = ⌘ command · 👍/👎 = add/remove cube");
    }

    /// Settings → Voice: Whisper model size (deferred item from the voice
    /// milestone) — persisted to /settings/voice.json; the platform applies
    /// it to the speech engine (takes effect on the next voice session).
    fn voice_ui(&mut self, ui: &mut egui::Ui, kernel: &mut dyn KernelApi, pid: Pid) {
        const PATH: &str = "/settings/voice.json";
        const MODELS: &[(&str, &str)] = &[
            ("tiny", "Fast — ~40 MB, fine for commands"),
            ("base", "Balanced — ~80 MB, better accuracy"),
            ("small", "Accurate — ~250 MB, dictation-grade"),
        ];
        if !self.voice_loaded {
            self.voice_loaded = true;
            if let Ok(pmos_abi::Reply::Bytes(b)) =
                kernel.syscall(pid, Syscall::FsRead { path: PATH.into() })
            {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                    if let Some(m) = v["whisper"].as_str() {
                        self.voice_model = m.to_string();
                    }
                }
            }
        }
        ui.heading("Voice");
        ui.add_space(4.0);
        let mut changed = false;
        for (id, label) in MODELS {
            changed |= ui
                .radio_value(&mut self.voice_model, id.to_string(), *label)
                .changed();
        }
        if changed {
            let json = serde_json::json!({ "whisper": self.voice_model }).to_string();
            let _ = kernel.syscall(
                pid,
                Syscall::FsWrite {
                    path: PATH.into(),
                    bytes: json.into_bytes(),
                },
            );
        }
        ui.weak("speech-to-text runs on this machine; a changed size downloads once");

        // ---- Face (M10, opt-in) ----
        const FACE_PATH: &str = "/settings/face.json";
        if !self.face_loaded {
            self.face_loaded = true;
            if let Ok(pmos_abi::Reply::Bytes(b)) =
                kernel.syscall(pid, Syscall::FsRead { path: FACE_PATH.into() })
            {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                    self.face_enabled = v["enabled"].as_bool().unwrap_or(false);
                }
            }
        }
        ui.add_space(10.0);
        ui.separator();
        ui.heading("Face");
        ui.add_space(4.0);
        if ui
            .checkbox(
                &mut self.face_enabled,
                "Face gestures — double-blink = click (experimental)",
            )
            .changed()
        {
            let json = serde_json::json!({ "enabled": self.face_enabled }).to_string();
            let _ = kernel.syscall(
                pid,
                Syscall::FsWrite {
                    path: FACE_PATH.into(),
                    bytes: json.into_bytes(),
                },
            );
        }
        ui.weak("face landmarks only, computed on this machine — video never leaves the camera pipeline");
    }

    /// Settings → Stage (ABI 1.8): spawn/clear physics objects, sun control.
    /// The AI has the same powers as tools — ask it to build something.
    fn stage_ui(&mut self, ui: &mut egui::Ui, kernel: &mut dyn KernelApi, pid: Pid) {
        const PALETTE: [[f32; 3]; 5] = [
            [0.43, 0.91, 1.00],
            [0.75, 0.52, 0.99],
            [1.00, 0.62, 0.42],
            [0.55, 0.95, 0.65],
            [0.95, 0.85, 0.45],
        ];
        ui.heading("Stage");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let mut spawn = |shape: u8, n: &mut u32| {
                let i = *n;
                *n += 1;
                // Deterministic scatter: no RNG needed, still looks organic.
                let x = ((i * 37 + 11) % 13) as f32 * 0.5 - 3.0;
                let z = ((i * 53 + 7) % 11) as f32 * 0.5 - 2.5;
                let _ = kernel.syscall(
                    pid,
                    Syscall::StageSpawn {
                        shape,
                        pos: [x, 3.5, z],
                        half: 0.45,
                        color: PALETTE[(i % 5) as usize],
                    },
                );
            };
            if ui.button("🧊 Drop a cube").clicked() {
                spawn(0, &mut self.stage_n);
            }
            if ui.button("🔮 Drop a sphere").clicked() {
                spawn(1, &mut self.stage_n);
            }
            if ui.button("🧹 Clear stage").clicked() {
                let _ = kernel.syscall(pid, Syscall::StageClear);
            }
        });
        ui.weak("objects are physical — grab with ✊ or mouse, throw them around");
        ui.add_space(6.0);
        let mut light_changed = false;
        egui::Grid::new("stage-light")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Sun azimuth");
                light_changed |= ui
                    .add(egui::Slider::new(&mut self.stage_az, 0.0..=360.0).suffix("°"))
                    .changed();
                ui.end_row();
                ui.label("Sun elevation");
                light_changed |= ui
                    .add(egui::Slider::new(&mut self.stage_el, 10.0..=85.0).suffix("°"))
                    .changed();
                ui.end_row();
                ui.label("Intensity");
                light_changed |= ui
                    .add(egui::Slider::new(&mut self.stage_intensity, 0.0..=2.0))
                    .changed();
                ui.end_row();
                ui.label("Ambient");
                light_changed |= ui
                    .add(egui::Slider::new(&mut self.stage_ambient, 0.0..=0.8))
                    .changed();
                ui.end_row();
            });
        if light_changed {
            let (az, el) = (self.stage_az.to_radians(), self.stage_el.to_radians());
            let _ = kernel.syscall(
                pid,
                Syscall::StageLight {
                    dir: [el.cos() * az.cos(), -el.sin(), el.cos() * az.sin()],
                    intensity: self.stage_intensity,
                    ambient: self.stage_ambient,
                },
            );
        }
        ui.weak("or ask the AI: \"build me a little tower\" · \"make it sunset\"");
    }

    /// Settings → Appearance (UI spec §6.1): stage background + color scheme,
    /// applied live and persisted to /settings/appearance.json via the VFS.
    fn appearance_ui(&mut self, ui: &mut egui::Ui, kernel: &mut dyn KernelApi, pid: Pid) {
        const BACKGROUNDS: &[&str] = &["Deep Space", "Ember Nebula", "Aurora", "Void"];
        const PATH: &str = "/settings/appearance.json";

        if !self.appearance_loaded {
            self.appearance_loaded = true;
            self.app_scheme = crate::theme::scheme();
            if let Ok(pmos_abi::Reply::Bytes(b)) =
                kernel.syscall(pid, Syscall::FsRead { path: PATH.into() })
            {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                    self.app_bg = v["background"].as_u64().unwrap_or(0) as u8;
                    self.app_scheme = v["scheme"].as_u64().unwrap_or(0) as u8;
                }
            }
        }

        ui.heading("Appearance");
        ui.add_space(4.0);
        let mut changed = false;
        egui::Grid::new("appearance")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Background");
                ui.horizontal_wrapped(|ui| {
                    for (i, name) in BACKGROUNDS.iter().enumerate() {
                        changed |= ui
                            .radio_value(&mut self.app_bg, i as u8, *name)
                            .changed();
                    }
                });
                ui.end_row();

                ui.label("Colors");
                ui.horizontal_wrapped(|ui| {
                    for (i, (name, a, _)) in crate::theme::SCHEMES.iter().enumerate() {
                        let r = ui.radio_value(&mut self.app_scheme, i as u8, *name);
                        changed |= r.changed();
                        // Swatch next to the label.
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(10.0, 10.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().circle_filled(rect.center(), 4.0, *a);
                    }
                });
                ui.end_row();
            });
        if changed {
            let _ = kernel.syscall(pid, Syscall::Background { style: self.app_bg });
            crate::theme::set_scheme(ui.ctx(), self.app_scheme);
            let json = serde_json::json!({
                "background": self.app_bg,
                "scheme": self.app_scheme,
            })
            .to_string();
            let _ = kernel.syscall(
                pid,
                Syscall::FsWrite {
                    path: PATH.into(),
                    bytes: json.into_bytes(),
                },
            );
        }
    }
}
