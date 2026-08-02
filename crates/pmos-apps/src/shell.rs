//! The shell process (UI spec §2): desktop, dock, and window management.
//! Talks to the kernel strictly through the ABI.

use crate::app_host;
use crate::apps::{AppAction, AppKind, AppState, ALL};
use crate::cursor::HandCursor;
use crate::palette::{Palette, PaletteOutcome, ToolCall};
use crate::voicekit::{KitAction, VoiceKit};
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
    /// The command palette (UI spec §2.4).
    palette: Palette,
    call_since: Option<f64>,
    call_fired: bool,
    /// Mirrors VoiceStatus — the RECORD sign toggles based on this.
    voice_listening: bool,
    /// The always-on voice layer (Voice Kit spec).
    voicekit: VoiceKit,
    /// A 🤙-hold palette voice session is waiting for its one utterance.
    palette_voice: bool,
    /// In-flight WebFetch tool calls: request id → tool name.
    pending_web: std::collections::HashMap<u32, String>,
    /// 👍/👎 hold state (G9 stage binding: add / remove-newest).
    thumbs_since: Option<f64>,
    thumbs_fired: bool,
    thumbs_down_since: Option<f64>,
    thumbs_down_fired: bool,
    /// Deterministic scatter counter for gesture/voice spawns.
    stage_n: u32,
    conjure_apps: Vec<ConjureApp>,
    toasts: Vec<(String, f64)>,
    /// Set each frame while the Browser window shows a page: (url, content
    /// rect in points). The platform overlays the actual iframe there.
    pub browser_view: Option<(String, [f32; 4])>,
    themed: bool,
    /// Boot-load of /settings/appearance.json (retried while OPFS loads).
    appearance_done: bool,
    appearance_tries: u32,
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
            palette: Palette::new(),
            call_since: None,
            call_fired: false,
            voice_listening: false,
            voicekit: VoiceKit::default(),
            palette_voice: false,
            pending_web: std::collections::HashMap::new(),
            thumbs_since: None,
            thumbs_fired: false,
            thumbs_down_since: None,
            thumbs_down_fired: false,
            stage_n: 0,
            conjure_apps: Vec::new(),
            toasts: Vec::new(),
            browser_view: None,
            themed: false,
            appearance_done: false,
            appearance_tries: 0,
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
                PaletteOutcome::Prompt(agent, msg) => {
                    let _ = kernel.syscall(self.pid, Syscall::AiPrompt { agent, msg });
                }
                PaletteOutcome::VoiceStop => {
                    let _ = kernel.syscall(self.pid, Syscall::VoiceCapture { start: false });
                }
                PaletteOutcome::StageSpawn { shape, spin } => {
                    self.stage_spawn(kernel, shape, spin, now)
                }
                PaletteOutcome::StageRemoveLast => self.stage_remove_last(kernel, now),
                PaletteOutcome::StageClear => {
                    let _ = kernel.syscall(self.pid, Syscall::StageClear);
                    self.toast("🧹 stage cleared".into(), now);
                }
                PaletteOutcome::ToolCall(call) => {
                    let (ok, result) = self.execute_tool(kernel, &call, now);
                    if result != "__deferred__" {
                        let more = self.palette.tool_result(&call.tool, ok, &result);
                        self.handle_outcomes(more, kernel, now);
                    } // else: the WebResult event resumes the tool loop
                }
                PaletteOutcome::SpawnConjure(doc) => {
                    if let Err(e) = self.spawn_conjure(kernel, &doc, now) {
                        self.toast(format!("⚠ couldn't spawn app: {e}"), now);
                    }
                }
            }
        }
    }

    /// Drop a primitive onto the stage (gesture 👍 / voice "drop a cube").
    /// `spin` starts it with a torque impulse ("a rotating cube").
    fn stage_spawn(&mut self, kernel: &mut dyn KernelApi, shape: u8, spin: bool, now: f64) {
        const PALETTE: [[f32; 3]; 5] = [
            [0.43, 0.91, 1.00],
            [0.75, 0.52, 0.99],
            [1.00, 0.62, 0.42],
            [0.55, 0.95, 0.65],
            [0.95, 0.85, 0.45],
        ];
        let i = self.stage_n;
        self.stage_n += 1;
        let x = ((i * 37 + 11) % 13) as f32 * 0.5 - 3.0;
        let z = ((i * 53 + 7) % 11) as f32 * 0.5 - 2.5;
        match kernel.syscall(
            self.pid,
            Syscall::StageSpawn {
                shape,
                pos: [x, 3.5, z],
                half: 0.45,
                color: PALETTE[(i % 5) as usize],
            },
        ) {
            Ok(reply) => {
                if spin {
                    if let Reply::Bytes(b) = &reply {
                        if let Ok(index) = String::from_utf8_lossy(b).trim().parse::<u32>() {
                            let _ = kernel.syscall(
                                self.pid,
                                Syscall::StageImpulse {
                                    index,
                                    impulse: [0.0, 0.0, 0.0],
                                    torque: [0.0, 2.5, 0.4],
                                },
                            );
                        }
                    }
                }
                self.toast(
                    format!(
                        "{} dropped — pinch 👌 or click-drag to grab it",
                        if shape == 0 { "🧊 cube" } else { "🔮 sphere" }
                    ),
                    now,
                );
            }
            Err(e) => self.toast(format!("⚠ spawn failed: {e:?}"), now),
        }
    }

    /// Remove the newest stage object (gesture 👎 / voice "remove the last").
    fn stage_remove_last(&mut self, kernel: &mut dyn KernelApi, now: f64) {
        let count = match kernel.syscall(self.pid, Syscall::StageList) {
            Ok(Reply::Bytes(b)) => serde_json::from_slice::<serde_json::Value>(&b)
                .ok()
                .and_then(|v| v.as_array().map(|a| a.len()))
                .unwrap_or(0),
            _ => 0,
        };
        if count == 0 {
            self.toast("the stage is already empty".into(), now);
            return;
        }
        let _ = kernel.syscall(
            self.pid,
            Syscall::StageRemove {
                index: (count - 1) as u32,
            },
        );
        self.toast("🗑 removed the newest object".into(), now);
    }

    /// A compact live-context block for voice commands (Voice Kit spec §5).
    fn context_envelope(&mut self, kernel: &mut dyn KernelApi) -> String {
        let stage = match kernel.syscall(self.pid, Syscall::StageList) {
            Ok(Reply::Bytes(b)) => serde_json::from_slice::<serde_json::Value>(&b)
                .ok()
                .and_then(|v| {
                    v.as_array().map(|a| {
                        let focused = a
                            .iter()
                            .find(|o| o["focused"].as_bool() == Some(true))
                            .map(|o| {
                                format!(
                                    " · focused: {} #{}",
                                    o["shape"].as_str().unwrap_or("object"),
                                    o["index"]
                                )
                            })
                            .unwrap_or_default();
                        format!("{} objects{focused}", a.len())
                    })
                })
                .unwrap_or_else(|| "unknown".into()),
            _ => "unavailable".into(),
        };
        let windows: Vec<&str> = self
            .open_apps
            .iter()
            .filter(|a| a.open)
            .map(|a| a.state.kind.title())
            .collect();
        format!(
            "[context] stage: {stage} · open windows: {} [/context]",
            if windows.is_empty() { "none".into() } else { windows.join(", ") }
        )
    }

    /// Kick off an async WebFetch; the WebResult event resumes the tool loop.
    fn web_defer(&mut self, kernel: &mut dyn KernelApi, tool: &str, url: String) -> (bool, String) {
        match kernel.syscall(self.pid, Syscall::WebFetch { url }) {
            Ok(Reply::Bytes(b)) => match String::from_utf8_lossy(&b).trim().parse::<u32>() {
                Ok(id) => {
                    self.pending_web.insert(id, tool.to_string());
                    (true, "__deferred__".into())
                }
                Err(_) => (false, "bad request id".into()),
            },
            Err(e) => (false, format!("{e:?}")),
            Ok(_) => (false, "unexpected reply".into()),
        }
    }

    /// Execute one assistant tool call through capability-checked syscalls
    /// (AI System spec §3). The shell is just an ABI client here — every call
    /// goes through the kernel dispatcher like any other process's would.
    fn execute_tool(
        &mut self,
        kernel: &mut dyn KernelApi,
        call: &ToolCall,
        now: f64,
    ) -> (bool, String) {
        const MAX_RESULT: usize = 4000;
        let arg = |key: &str| {
            call.args
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        match call.tool.as_str() {
            "sys_query" => match kernel.syscall(self.pid, Syscall::SysQuery { path: arg("path") })
            {
                Ok(Reply::Bytes(b)) => (true, String::from_utf8_lossy(&b).into_owned()),
                Ok(_) => (true, String::new()),
                Err(e) => (false, format!("{e:?}")),
            },
            "fs_list" => match kernel.syscall(self.pid, Syscall::FsList { path: arg("path") }) {
                Ok(Reply::Entries(entries)) => (
                    true,
                    entries
                        .iter()
                        .map(|e| {
                            if e.dir {
                                format!("{}/", e.name)
                            } else {
                                format!("{} ({} B)", e.name, e.size)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                Ok(_) => (true, String::new()),
                Err(e) => (false, format!("{e:?}")),
            },
            "fs_read" => match kernel.syscall(self.pid, Syscall::FsRead { path: arg("path") }) {
                Ok(Reply::Bytes(b)) => {
                    let mut text = String::from_utf8_lossy(&b).into_owned();
                    if text.len() > MAX_RESULT {
                        let cut = text
                            .char_indices()
                            .take_while(|(i, _)| *i < MAX_RESULT)
                            .last()
                            .map(|(i, c)| i + c.len_utf8())
                            .unwrap_or(0);
                        text.truncate(cut);
                        text.push_str("\n… (truncated)");
                    }
                    (true, text)
                }
                Ok(_) => (true, String::new()),
                Err(e) => (false, format!("{e:?}")),
            },
            "fs_write" => {
                let path = arg("path");
                let content = arg("content");
                match kernel.syscall(
                    self.pid,
                    Syscall::FsWrite {
                        path: path.clone(),
                        bytes: content.into_bytes(),
                    },
                ) {
                    Ok(_) => {
                        // Tier-1 transparency (AI System spec §6): reversible
                        // writes surface as a toast.
                        self.toast(format!("🤖 assistant wrote {path}"), now);
                        (true, "written".into())
                    }
                    Err(e) => (false, format!("{e:?}")),
                }
            }
            "stage_spawn" => {
                let f = |k: &str, d: f32| {
                    call.args.get(k).and_then(|v| v.as_f64()).unwrap_or(d as f64) as f32
                };
                let shape = match call.args.get("shape").and_then(|v| v.as_str()) {
                    Some("sphere") => 1u8,
                    _ => 0u8,
                };
                let color = call
                    .args
                    .get("color")
                    .and_then(|v| v.as_str())
                    .and_then(parse_hex_color)
                    .unwrap_or([0.6, 0.8, 1.0]);
                match kernel.syscall(
                    self.pid,
                    Syscall::StageSpawn {
                        shape,
                        pos: [f("x", 0.0), f("y", 3.0), f("z", 0.0)],
                        half: f("size", 0.45),
                        color,
                    },
                ) {
                    Ok(Reply::Bytes(b)) => {
                        (true, format!("spawned index {}", String::from_utf8_lossy(&b)))
                    }
                    Ok(_) => (true, "spawned".into()),
                    Err(e) => (false, format!("{e:?}")),
                }
            }
            "stage_list" => match kernel.syscall(self.pid, Syscall::StageList) {
                Ok(Reply::Bytes(b)) => (true, String::from_utf8_lossy(&b).into_owned()),
                Ok(_) => (true, "[]".into()),
                Err(e) => (false, format!("{e:?}")),
            },
            "stage_remove" => {
                let index = call.args.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                match kernel.syscall(self.pid, Syscall::StageRemove { index }) {
                    Ok(_) => (true, "removed".into()),
                    Err(e) => (false, format!("{e:?}")),
                }
            }
            "stage_clear" => match kernel.syscall(self.pid, Syscall::StageClear) {
                Ok(_) => (true, "cleared".into()),
                Err(e) => (false, format!("{e:?}")),
            },
            "stage_push" => {
                let f = |k: &str| {
                    call.args.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32
                };
                let index = call.args.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                match kernel.syscall(
                    self.pid,
                    Syscall::StageImpulse {
                        index,
                        impulse: [f("x"), f("y"), f("z")],
                        torque: [f("rx"), f("ry"), f("rz")],
                    },
                ) {
                    Ok(_) => (true, "pushed".into()),
                    Err(e) => (false, format!("{e:?}")),
                }
            }
            "stage_light" => {
                let f = |k: &str, d: f32| {
                    call.args.get(k).and_then(|v| v.as_f64()).unwrap_or(d as f64) as f32
                };
                let (az, el) = (f("azimuth", 215.0).to_radians(), f("elevation", 60.0).to_radians());
                match kernel.syscall(
                    self.pid,
                    Syscall::StageLight {
                        dir: [el.cos() * az.cos(), -el.sin(), el.cos() * az.sin()],
                        intensity: f("intensity", 1.0),
                        ambient: f("ambient", 0.22),
                    },
                ) {
                    Ok(_) => (true, "lighting set".into()),
                    Err(e) => (false, format!("{e:?}")),
                }
            }
            "web_open" => {
                let url = arg("url");
                if url.is_empty() {
                    (false, "missing url".into())
                } else {
                    self.launch(kernel, AppKind::Browser);
                    if let Some(app) = self
                        .open_apps
                        .iter_mut()
                        .find(|a| a.state.kind == AppKind::Browser)
                    {
                        app.state.browser_open(&url);
                    }
                    (true, format!("opened {url} in the Browser window (visible to the user)"))
                }
            }
            "web_search" => {
                // Wikipedia OpenSearch: keyless, CORS-open (AI System §5).
                let url = format!(
                    "https://en.wikipedia.org/w/api.php?action=opensearch&format=json&origin=*&limit=5&search={}",
                    urlencode(&arg("query"))
                );
                self.web_defer(kernel, &call.tool, url)
            }
            "web_fetch" => {
                // Jina Reader proxy: page → readable text, CORS-open.
                let url = arg("url");
                if url.is_empty() {
                    (false, "missing url".into())
                } else {
                    self.web_defer(kernel, &call.tool, format!("https://r.jina.ai/{url}"))
                }
            }
            "app_open" => {
                let name = arg("name").to_lowercase();
                if !name.is_empty() {
                    for kind in ALL {
                        if kind.title().to_lowercase().contains(&name) {
                            self.launch(kernel, kind);
                            return (true, format!("opened {}", kind.title()));
                        }
                    }
                }
                (false, format!("unknown app `{name}`"))
            }
            other => (false, format!("unknown tool `{other}`")),
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
        rt_tex: Option<egui::TextureId>,
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
            theme::install_fonts(ctx);
            theme::apply(ctx);
            self.themed = true;
        }
        // Apply the persisted appearance once the async VFS boot delivers it
        // (missing file after ~4 s of retries = fresh install, defaults stay).
        if !self.appearance_done {
            self.appearance_tries += 1;
            match kernel.syscall(
                self.pid,
                Syscall::FsRead {
                    path: "/settings/appearance.json".into(),
                },
            ) {
                Ok(pmos_abi::Reply::Bytes(b)) => {
                    self.appearance_done = true;
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                        let bg = v["background"].as_u64().unwrap_or(0) as u8;
                        let _ = kernel.syscall(self.pid, Syscall::Background { style: bg });
                        theme::set_scheme(ctx, v["scheme"].as_u64().unwrap_or(0) as u8);
                    }
                }
                _ if self.appearance_tries > 240 => self.appearance_done = true,
                _ => {}
            }
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
                KernelEvent::VoiceStatus {
                    listening,
                    available,
                    reason,
                } => {
                    self.voice_listening = listening;
                    self.voicekit.on_status(listening);
                    if self.palette_voice {
                        let outcomes =
                            self.palette.on_voice_status(listening, available, &reason);
                        ai_outcomes.extend(outcomes);
                        if !listening {
                            self.palette_voice = false;
                        }
                    } else if !available {
                        self.toast(format!("⚠ voice: {reason}"), now);
                    }
                }
                KernelEvent::Sign { sign } => match sign {
                    pmos_abi::CslSign::Record => {
                        // ✋ still hold: toggle the always-on Voice Kit.
                        if self.voice_listening {
                            let _ =
                                kernel.syscall(self.pid, Syscall::VoiceCapture { start: false });
                            self.toast("⏸ voice capture off (hold ✋ still to restart)".into(), now);
                        } else {
                            let _ =
                                kernel.syscall(self.pid, Syscall::VoiceCapture { start: true });
                            self.toast("● voice capture on — hold ✋ still to stop".into(), now);
                        }
                    }
                    pmos_abi::CslSign::Cancel => {
                        // ✋ push: Esc-equivalent — close the palette, clear
                        // command arming. (The kernel already cleared focus.)
                        self.palette.open = false;
                        self.voicekit.command_armed = false;
                        self.toast("✕ cancelled".into(), now);
                    }
                    pmos_abi::CslSign::Command => {
                        // ☝ held still while capturing: arm command mode.
                        if self.voice_listening && !self.voicekit.command_armed {
                            self.voicekit.command_armed = true;
                            self.toast("⌘ armed — the next words are a command".into(), now);
                        }
                    }
                    _ => {}
                },
                KernelEvent::VoiceTranscript { text, is_final } => {
                    if self.palette_voice {
                        // 🤙 push-to-talk: the palette owns this utterance.
                        let outcomes = self.palette.on_voice_transcript(text.clone(), is_final);
                        ai_outcomes.extend(outcomes);
                        if is_final {
                            self.palette_voice = false;
                        }
                    } else if is_final {
                        // Ambient transcript → the Voice Kit; ⌘-armed
                        // utterances route as commands with AI context.
                        let is_cmd = self.voicekit.command_armed;
                        self.voicekit.command_armed = false;
                        self.voicekit
                            .on_final(&text, is_cmd, kernel, self.pid, today, now);
                        if is_cmd {
                            let ctx_env = self.context_envelope(kernel);
                            let outcomes = self
                                .palette
                                .run_voice_command(text)
                                .into_iter()
                                .map(|o| match o {
                                    PaletteOutcome::Prompt(agent, msg)
                                        if agent == pmos_abi::AGENT_ASSISTANT =>
                                    {
                                        PaletteOutcome::Prompt(
                                            agent,
                                            format!("{ctx_env}\n\nUser (voice): {msg}"),
                                        )
                                    }
                                    other => other,
                                })
                                .collect();
                            ai_outcomes.extend::<Vec<PaletteOutcome>>(outcomes);
                        }
                    } else {
                        self.voicekit.on_interim(&text);
                    }
                }
                KernelEvent::WebResult { id, ok, body } => {
                    if let Some(tool) = self.pending_web.remove(&id) {
                        let mut body = body;
                        if body.len() > 4000 {
                            let cut = body
                                .char_indices()
                                .take_while(|(i, _)| *i < 4000)
                                .last()
                                .map(|(i, c)| i + c.len_utf8())
                                .unwrap_or(0);
                            body.truncate(cut);
                            body.push_str("\n… (truncated)");
                        }
                        let more = self.palette.tool_result(&tool, ok, &body);
                        ai_outcomes.extend(more);
                    }
                }
                other => log::debug!("shell event: {other:?}"),
            }
        }
        if !self.cursor.tracking {
            self.raw_hands.1 = 0;
        }

        // (The Launcher was removed entirely — user decision 2026-08-01.
        // The open palm belongs to the RECORD sign, CSL spec §4.)

        // 🤙 tap toggles the palette; 🤙 HELD ≥ 0.6 s opens it in voice mode
        // and starts speech capture (Hand Gestures spec G8). The dwell makes
        // the voice trigger impossible to hit by accident.
        if self.cursor.tracking && self.cursor.pose == pmos_abi::HandPose::CallSign {
            let since = *self.call_since.get_or_insert(now);
            if now - since >= 0.6 && !self.call_fired {
                self.call_fired = true;
                self.palette_voice = true;
                self.palette.start_voice();
                let _ = kernel.syscall(self.pid, Syscall::VoiceCapture { start: true });
            }
        } else {
            if let Some(since) = self.call_since.take() {
                if !self.call_fired && now - since < 0.5 {
                    self.palette.toggle();
                }
            }
            self.call_fired = false;
        }
        // Ctrl+K.
        if ctx.input(|i| i.key_pressed(egui::Key::K) && i.modifiers.command) {
            self.palette.toggle();
        }

        // 👍 hold drops a cube on the stage; 👎 hold removes the newest
        // object (Hand Gestures G9 — v1 stage binding; dialogs will take
        // precedence once consent sheets land).
        if self.cursor.tracking && self.cursor.pose == pmos_abi::HandPose::ThumbsUp {
            let since = *self.thumbs_since.get_or_insert(now);
            if now - since >= 0.6 && !self.thumbs_fired {
                self.thumbs_fired = true;
                self.stage_spawn(kernel, 0, false, now);
            }
        } else {
            self.thumbs_since = None;
            self.thumbs_fired = false;
        }
        if self.cursor.tracking && self.cursor.pose == pmos_abi::HandPose::ThumbsDown {
            let since = *self.thumbs_down_since.get_or_insert(now);
            if now - since >= 0.6 && !self.thumbs_down_fired {
                self.thumbs_down_fired = true;
                self.stage_remove_last(kernel, now);
            }
        } else {
            self.thumbs_down_since = None;
            self.thumbs_down_fired = false;
        }

        self.browser_view = None;
        self.handle_outcomes(ai_outcomes, kernel, now);
        self.windows(ctx, kernel, camera_feed, rt_tex, today, now);
        self.conjure_windows(ctx, kernel, now);
        self.dock(ctx, kernel);
        let palette_outcomes = self.palette.ui(ctx);
        self.handle_outcomes(palette_outcomes, kernel, now);
        for action in self.voicekit.ui(ctx, kernel, self.pid, today) {
            match action {
                KitAction::ToggleCapture => {
                    let start = !self.voice_listening;
                    let _ = kernel.syscall(self.pid, Syscall::VoiceCapture { start });
                }
                KitAction::Toast(t) => self.toast(t, now),
            }
        }
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
            .interactable(false) // display-only, never block stage input
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
        rt_tex: Option<egui::TextureId>,
        today: &str,
        now: f64,
    ) {
        let mut actions: Vec<AppAction> = Vec::new();
        let mut browser_view: Option<(String, [f32; 4])> = None;
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
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| state.settings_ui(ui, kernel, pid));
                    None
                }
                AppKind::Terminal => state.terminal_ui(ui, kernel, pid),
                AppKind::Files => state.files_ui(ui, kernel, pid),
                AppKind::Notes => {
                    state.notes_ui(ui, kernel, pid, today);
                    None
                }
                AppKind::RayTracer => {
                    state.ray_tracer_ui(ui, kernel, pid, rt_tex);
                    None
                }
                AppKind::Browser => {
                    if let Some((url, rect)) = state.browser_ui(ui) {
                        browser_view =
                            Some((url, [rect.min.x, rect.min.y, rect.width(), rect.height()]));
                    }
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
        // Hand the Browser's content rect to the platform — THIS was the
        // "browser loads nothing" bug: the local result was dropped here,
        // so the platform-side iframe never received a URL or rect.
        self.browser_view = browser_view;
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
                                    ui.painter().circle_filled(below, 2.0, theme::accent_a());
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
            // Display-only: MUST stay out of pointer hit-tests — this layer's
            // stored rect once swallowed stage clicks ("backg" in the logs)
            // and killed orbit/grab/zoom routing (user-reported).
            .interactable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.weak(self.cursor.tray_text());
                    ui.weak("·");
                    ui.weak("drag: orbit · wheel: zoom · Home: reset");
                });
            });
    }
}

/// Minimal percent-encoding for query strings.
fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Parse "#rrggbb" (hash optional) into linear-ish RGB floats.
fn parse_hex_color(s: &str) -> Option<[f32; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([
        ((v >> 16) & 0xff) as f32 / 255.0,
        ((v >> 8) & 0xff) as f32 / 255.0,
        (v & 0xff) as f32 / 255.0,
    ])
}
