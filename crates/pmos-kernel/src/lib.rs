//! The Pseudo Motion OS kernel (Architecture spec §4).
//!
//! Subsystems live in their own modules and are reachable from userland only
//! through the syscall dispatcher below. Nothing here may import `web-sys` —
//! all platform access goes through `pmos-platform` (Architecture §3); wgpu
//! is allowed (it is itself a platform abstraction).

pub mod ai;
pub mod gfx;
pub mod input;
pub mod phys;
pub mod proc;
pub mod vfs;

// Single wgpu version for the whole workspace: the one egui-wgpu pins.
pub use egui_wgpu::wgpu;

use pmos_abi::{
    Capability, ErrorCode, HandsTuning, KernelApi, KernelEvent, Pid, Reply, Syscall, WinId,
};
use std::collections::HashMap;

pub struct WinRecord {
    pub owner: Pid,
    pub title: String,
    pub size: [f32; 2],
}

/// Directives for the platform glue (Architecture §3): the kernel records
/// intent set via syscalls; the platform reads it each frame and drives the
/// JS pipeline. `generation` bumps on every change so the glue applies
/// changes exactly once.
#[derive(Clone, PartialEq)]
pub struct HandsDirectives {
    pub camera_start: bool,
    pub viewer_open: bool,
    pub stream_feed: bool,
    pub tuning: HandsTuning,
    pub generation: u32,
}

/// Voice-capture directive (ABI 1.5): shell intent → platform speech engine.
/// Same generation contract as [`HandsDirectives`].
#[derive(Clone, PartialEq)]
pub struct VoiceDirectives {
    pub capture: bool,
    pub generation: u32,
}

/// The kernel root object, owned by the platform entry point (`pmos-web`).
pub struct Kernel {
    pub procs: proc::ProcessTable,
    pub input: input::InputPipeline,
    /// Graphics engine — installed after the async wgpu device request.
    pub gfx: Option<gfx::Gfx>,
    pub hands_directives: HandsDirectives,
    pub voice_directives: VoiceDirectives,
    /// WebFetch requests waiting for the platform (ABI 1.11): (id, url).
    pub web_pending: Vec<(u32, String)>,
    next_web_id: u32,
    /// Face gesture state (M10): double-blink detection over blendshapes.
    face: FaceState,
    /// Set on double-blink; the platform injects a click at the pointer.
    pub face_click_pending: bool,
    /// Previous frame's two-palm spread (camera-space) for zoom deltas.
    last_palm_spread: Option<f32>,
    pub ai: ai::AiState,
    pub vfs: vfs::Vfs,
    pub phys: phys::Physics,
    windows: HashMap<WinId, WinRecord>,
    next_win: u32,
    events: HashMap<Pid, Vec<KernelEvent>>,
}

impl Kernel {
    pub fn new() -> Self {
        log::info!("pmos-kernel init, ABI {:?}", pmos_abi::ABI_VERSION);
        Self {
            procs: proc::ProcessTable::new(),
            input: input::InputPipeline::new(),
            gfx: None,
            hands_directives: HandsDirectives {
                camera_start: false,
                viewer_open: false,
                stream_feed: false,
                tuning: HandsTuning::default(),
                generation: 0,
            },
            voice_directives: VoiceDirectives {
                capture: false,
                generation: 0,
            },
            web_pending: Vec::new(),
            next_web_id: 1,
            face: FaceState::default(),
            face_click_pending: false,
            last_palm_spread: None,
            ai: ai::AiState::default(),
            vfs: vfs::Vfs::new(),
            phys: phys::Physics::new(),
            windows: HashMap::new(),
            next_win: 1,
            events: HashMap::new(),
        }
    }

    /// Streamed LLM delta from the platform → AiChunk event to the requester.
    pub fn ai_chunk(&mut self, agent: u32, delta: String, done: bool) {
        if done {
            self.ai_log(format!("← agent {agent} reply finished"));
        }
        if let Some(requester) = self.ai.chunk(agent, &delta, done) {
            self.push_event(
                requester,
                KernelEvent::AiChunk {
                    agent: pmos_abi::AgentId(agent),
                    text: delta,
                    done,
                },
            );
        }
    }

    pub fn install_gfx(&mut self, gfx: gfx::Gfx) {
        self.gfx = Some(gfx);
    }

    /// Ingest a landmark frame from the gesture worker and forward the
    /// resulting hand state to the shell (Architecture §4.4).
    pub fn hand_frame(&mut self, data: &[f32], hands: u32, viewport: [f32; 2], now: f64) {
        self.input.hands.ingest(data, hands, viewport, now);
        if let Some(pos) = self.input.hands.cursor {
            self.input.pointer_moved(pos, pmos_abi::InputSource::Hand);
            let (pose, tracking) = (self.input.hands.pose, self.input.hands.tracking);
            self.input.fusion.step(pose, pos, tracking);
        }
        // Two-palm zoom (CSL spec §5): both hands open → spreading them
        // apart zooms out, bringing them together zooms in.
        let spread = self.input.hands.palm_spread;
        if let (Some(s), Some(prev), Some(gfx)) =
            (spread, self.last_palm_spread, self.gfx.as_mut())
        {
            gfx.camera.zoom((s - prev) * 12.0);
        }
        self.last_palm_spread = spread;

        // CSL sign recognition rides the same debounced pose stream.
        if let Some(sign) = self.input.signs.step(
            self.input.hands.pose,
            self.input.hands.cursor,
            self.input.hands.tracking,
            now,
        ) {
            self.push_event(proc::SHELL_PID, KernelEvent::Sign { sign });
        }
        self.publish_hand_state();
        // Raw landmarks flow only while the viewer wants them, and only to
        // the raw-hands-capable shell (ABI 1.2).
        if self.hands_directives.viewer_open
            && self
                .procs
                .has_cap(proc::SHELL_PID, &Capability::InputRawHands)
        {
            self.push_event(
                proc::SHELL_PID,
                KernelEvent::RawHands {
                    data: data.to_vec(),
                    hands: hands.min(2) as u8,
                },
            );
        }
    }

    /// Per-frame upkeep: tracking-loss timeout and shell notification.
    pub fn tick_hands(&mut self, now: f64) {
        let was_tracking = self.input.hands.tracking;
        self.input.hands.tick(now);
        if was_tracking != self.input.hands.tracking {
            if !self.input.hands.tracking {
                // Release anything the hand was holding (spec §7); a
                // half-formed sign aborts too.
                let pos = self.input.hands.cursor.unwrap_or([0.0, 0.0]);
                self.input.fusion.lost(pos);
                let _ = self.input.signs.step(pmos_abi::HandPose::Rest, None, false, now);
            }
            self.publish_hand_state();
        }
    }

    /// Advance physics and render one frame (Architecture §7 steps 4+6).
    pub fn render_frame(
        &mut self,
        primitives: &[egui::ClippedPrimitive],
        textures_delta: &egui::TexturesDelta,
        pixels_per_point: f32,
        time: f32,
        dt: f32,
    ) {
        self.phys.step(dt.min(0.25));
        let instances = self.phys.instances();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.render(
                primitives,
                textures_delta,
                pixels_per_point,
                time,
                &instances,
            );
        }
    }

    /// Per-frame hover pick: which prop sits under the pointer (mouse or
    /// hand)? Drives the hover glow — objects respond before any click
    /// (first-class citizens, user request 2026-08-02).
    pub fn update_hover(&mut self, pos: Option<[f32; 2]>, viewport: [f32; 2]) {
        self.phys.hovered = pos.and_then(|p| {
            let gfx = self.gfx.as_ref()?;
            let (origin, dir) = gfx.screen_ray(p, viewport);
            let (body, _) = self.phys.pick(origin, dir)?;
            self.phys.props.iter().position(|pr| pr.body == body)
        });
    }

    /// Try to grab a prop under the given screen position. Returns true if a
    /// body was grabbed (else the caller falls back to camera orbit).
    pub fn try_grab_prop(&mut self, pos: [f32; 2], viewport: [f32; 2]) -> bool {
        let Some(gfx) = self.gfx.as_ref() else {
            return false;
        };
        let (origin, dir) = gfx.screen_ray(pos, viewport);
        if let Some((body, depth)) = self.phys.pick(origin, dir) {
            self.phys.grab(body, depth);
            true
        } else {
            false
        }
    }

    /// Move the grab target to the ray point at the stored grab depth.
    pub fn move_grab(&mut self, pos: [f32; 2], viewport: [f32; 2]) {
        let (Some(gfx), Some(depth)) = (self.gfx.as_ref(), self.phys.grab_depth()) else {
            return;
        };
        let (origin, dir) = gfx.screen_ray(pos, viewport);
        self.phys.grab_move(origin + dir * depth);
    }

    pub fn release_grab(&mut self) {
        self.phys.release();
    }

    pub fn set_camera_status(&mut self, enabled: bool, reason: String) {
        self.input.hands.camera_enabled = enabled;
        log::info!("camera pipeline enabled: {enabled} ({reason})");
        self.push_event(
            proc::SHELL_PID,
            KernelEvent::CameraStatus { enabled, reason },
        );
    }

    /// Speech-engine status from the platform → shell (ABI 1.5). When the
    /// engine ends on its own (end of utterance, error), the capture intent
    /// is synced without a generation bump — no stop call needs dispatching.
    pub fn voice_status(&mut self, listening: bool, available: bool, reason: String) {
        if !listening {
            self.voice_directives.capture = false;
        }
        self.push_event(
            proc::SHELL_PID,
            KernelEvent::VoiceStatus {
                listening,
                available,
                reason,
            },
        );
    }

    /// Speech transcript from the platform → shell (ABI 1.5). Text only:
    /// audio never crosses this boundary (AI System spec §5).
    pub fn voice_transcript(&mut self, text: String, is_final: bool) {
        self.push_event(
            proc::SHELL_PID,
            KernelEvent::VoiceTranscript { text, is_final },
        );
    }

    fn publish_hand_state(&mut self) {
        let h = &self.input.hands;
        let ev = KernelEvent::HandUpdate {
            pose: h.pose,
            pinch: h.pinch,
            pos: h.cursor,
            tracking: h.tracking,
            hands: h.hands,
        };
        self.push_event(proc::SHELL_PID, ev);
    }

    fn push_event(&mut self, pid: Pid, ev: KernelEvent) {
        self.events.entry(pid).or_default().push(ev);
    }

    /// Append to the /sys/ai/log ring (transparency — AI System spec §3).
    fn ai_log(&mut self, line: String) {
        let log = &mut self.vfs.sys_ai_log;
        log.push_back(line);
        while log.len() > 200 {
            log.pop_front();
        }
    }

    /// Scoped filesystem capability check: the caller must hold an
    /// FsRead/FsWrite whose scope prefixes the requested path.
    fn require_fs(&self, caller: Pid, path: &str, write: bool) -> Result<(), ErrorCode> {
        if self.procs.has_fs_cap(caller, path, write) {
            Ok(())
        } else {
            log::warn!(
                "fs capability denied for {caller:?}: {} {}",
                if write { "write" } else { "read" },
                path
            );
            Err(ErrorCode::CapabilityDenied)
        }
    }

    fn require(&self, caller: Pid, cap: &Capability) -> Result<(), ErrorCode> {
        if self.procs.has_cap(caller, cap) {
            Ok(())
        } else {
            log::warn!("capability denied for {caller:?}: {cap:?}");
            Err(ErrorCode::CapabilityDenied)
        }
    }
}

/// Minimal face-layer state (CSL spec §6, M10): the platform streams a few
/// blendshape scores; the kernel turns a quick double-blink into a sign.
#[derive(Default)]
struct FaceState {
    eyes_closed: bool,
    blink_times: Vec<f64>,
}

impl Kernel {
    /// Blendshape frame from the platform: [blinkL, blinkR, jawOpen, browUp].
    pub fn face_frame(&mut self, blink_l: f32, blink_r: f32, now: f64) {
        let closed = blink_l > 0.55 && blink_r > 0.55;
        if closed && !self.face.eyes_closed {
            self.face.blink_times.push(now);
            self.face.blink_times.retain(|t| now - t < 0.7);
            if self.face.blink_times.len() >= 2 {
                self.face.blink_times.clear();
                self.face_click_pending = true;
                self.push_event(
                    proc::SHELL_PID,
                    KernelEvent::Sign {
                        sign: pmos_abi::CslSign::DoubleBlink,
                    },
                );
            }
        }
        self.face.eyes_closed = closed;
    }

    /// Platform delivered a WebFetch body (ABI 1.11).
    pub fn web_result(&mut self, id: u32, ok: bool, body: String) {
        self.push_event(proc::SHELL_PID, KernelEvent::WebResult { id, ok, body });
    }
}

impl KernelApi for Kernel {
    /// Syscall entry point (Architecture §6). Every call is capability-checked
    /// against the calling process before any subsystem sees it.
    fn syscall(&mut self, caller: Pid, call: Syscall) -> Result<Reply, ErrorCode> {
        match call {
            Syscall::ProcRegister { name, caps } => {
                // Registering is unprivileged; new processes start with the
                // default (minimal) set. Extra caps are granted only by
                // delegation — the caller must itself hold each one. The very
                // first process is by contract the shell (shell grant).
                let granted = if self.procs.is_empty() {
                    proc::shell_caps()
                } else {
                    let mut g = proc::default_caps();
                    for c in caps {
                        if self.procs.has_cap(caller, &c) && !g.contains(&c) {
                            g.push(c);
                        }
                    }
                    g
                };
                let pid = self.procs.register(&name, granted);
                self.vfs.sys_processes = self.procs.listing();
                Ok(Reply::Pid(pid))
            }
            Syscall::ProcKill(pid) => {
                // Only the shell (pid 1) may kill other processes in v1.
                if caller != proc::SHELL_PID && caller != pid {
                    return Err(ErrorCode::CapabilityDenied);
                }
                self.procs.kill(pid);
                self.vfs.sys_processes = self.procs.listing();
                self.windows.retain(|_, w| w.owner != pid);
                Ok(Reply::None)
            }
            Syscall::WinCreate(desc) => {
                self.require(caller, &Capability::WinOwn)?;
                let id = WinId(self.next_win);
                self.next_win += 1;
                self.windows.insert(
                    id,
                    WinRecord {
                        owner: caller,
                        title: desc.title,
                        size: desc.size,
                    },
                );
                Ok(Reply::Win(id))
            }
            Syscall::WinClose(id) => {
                let win = self.windows.get(&id).ok_or(ErrorCode::NotFound)?;
                if win.owner != caller && caller != proc::SHELL_PID {
                    return Err(ErrorCode::CapabilityDenied);
                }
                let owner = win.owner;
                self.windows.remove(&id);
                self.push_event(owner, KernelEvent::WinClosed(id));
                Ok(Reply::None)
            }
            Syscall::WinSetTitle(id, title) => {
                let win = self.windows.get_mut(&id).ok_or(ErrorCode::NotFound)?;
                if win.owner != caller {
                    return Err(ErrorCode::CapabilityDenied);
                }
                win.title = title;
                Ok(Reply::None)
            }
            Syscall::SysQuery { path } => {
                self.require(caller, &Capability::SysQuery)?;
                match self.vfs.read(&path) {
                    Some(bytes) => Ok(Reply::Bytes(bytes)),
                    None => Err(ErrorCode::NotFound),
                }
            }
            Syscall::FsRead { path } => {
                self.require_fs(caller, &path, false)?;
                match self.vfs.read(&path) {
                    Some(bytes) => Ok(Reply::Bytes(bytes)),
                    None => Err(ErrorCode::NotFound),
                }
            }
            Syscall::FsList { path } => {
                self.require_fs(caller, &path, false)?;
                match self.vfs.list(&path) {
                    Some(entries) => Ok(Reply::Entries(entries)),
                    None => Err(ErrorCode::NotFound),
                }
            }
            Syscall::FsWrite { path, bytes } => {
                self.require_fs(caller, &path, true)?;
                self.vfs.write(&path, bytes).map_err(|e| {
                    log::warn!("fs write failed: {e}");
                    ErrorCode::InvalidArgument
                })?;
                self.push_event(proc::SHELL_PID, KernelEvent::FsChanged { path });
                Ok(Reply::None)
            }
            Syscall::FsDelete { path } => {
                self.require_fs(caller, &path, true)?;
                self.vfs.delete(&path).map_err(|e| {
                    log::warn!("fs delete failed: {e}");
                    ErrorCode::InvalidArgument
                })?;
                self.push_event(proc::SHELL_PID, KernelEvent::FsChanged { path });
                Ok(Reply::None)
            }
            Syscall::FsMkdir { path } => {
                self.require_fs(caller, &path, true)?;
                self.vfs.mkdir(&path).map_err(|e| {
                    log::warn!("fs mkdir failed: {e}");
                    ErrorCode::InvalidArgument
                })?;
                self.push_event(proc::SHELL_PID, KernelEvent::FsChanged { path });
                Ok(Reply::None)
            }
            Syscall::AiConfigure(cfg) => {
                self.require(caller, &Capability::AiPrompt)?;
                self.ai.set_config(cfg, true);
                Ok(Reply::None)
            }
            Syscall::AiPrompt { agent, msg } => {
                self.require(caller, &Capability::AiPrompt)?;
                let head: String = msg.chars().take(100).collect();
                self.ai_log(format!(
                    "→ agent {} ({caller:?}): {head}{}",
                    agent.0,
                    if msg.chars().count() > 100 { "…" } else { "" }
                ));
                if let Err(e) = self.ai.prompt(agent, caller, msg) {
                    // Deliver the failure as a terminal chunk so callers have
                    // one uniform streaming path.
                    self.push_event(
                        caller,
                        KernelEvent::AiChunk {
                            agent,
                            text: format!("⚠ {e}"),
                            done: true,
                        },
                    );
                }
                Ok(Reply::None)
            }
            Syscall::RtConfig { bounces, animate } => {
                self.require(caller, &Capability::SysQuery)?;
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.rt_bounces = bounces.clamp(1, 5);
                    gfx.rt_animate = animate;
                }
                Ok(Reply::None)
            }
            Syscall::CameraStart => {
                self.require(caller, &Capability::InputRawHands)?;
                self.hands_directives.camera_start = true;
                self.hands_directives.generation += 1;
                Ok(Reply::None)
            }
            Syscall::HandsViewer { open, stream_feed } => {
                self.require(caller, &Capability::InputRawHands)?;
                let d = &mut self.hands_directives;
                if d.viewer_open != open || d.stream_feed != stream_feed {
                    d.viewer_open = open;
                    d.stream_feed = stream_feed;
                    d.generation += 1;
                }
                Ok(Reply::None)
            }
            Syscall::HandsTune(tuning) => {
                self.require(caller, &Capability::InputRawHands)?;
                if self.hands_directives.tuning != tuning {
                    self.input.hands.apply_tuning(&tuning);
                    self.hands_directives.tuning = tuning;
                    self.hands_directives.generation += 1;
                }
                Ok(Reply::None)
            }
            Syscall::Background { style } => {
                self.require(caller, &Capability::SysQuery)?;
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.sky_style = style.min(3);
                }
                Ok(Reply::None)
            }
            Syscall::StageSpawn {
                shape,
                pos,
                half,
                color,
            } => {
                self.require(caller, &Capability::PhysSpawn)?;
                self.phys.spawn_prop(
                    glam::Vec3::from(pos),
                    shape.min(1),
                    half.clamp(0.1, 2.0),
                    color,
                );
                let index = self.phys.props.len() - 1;
                Ok(Reply::Bytes(index.to_string().into_bytes()))
            }
            Syscall::StageRemove { index } => {
                self.require(caller, &Capability::PhysSpawn)?;
                if self.phys.remove_prop(index as usize) {
                    Ok(Reply::None)
                } else {
                    Err(ErrorCode::NotFound)
                }
            }
            Syscall::StageClear => {
                self.require(caller, &Capability::PhysSpawn)?;
                self.phys.clear_props();
                Ok(Reply::None)
            }
            Syscall::StageImpulse {
                index,
                impulse,
                torque,
            } => {
                self.require(caller, &Capability::PhysSpawn)?;
                if self.phys.impulse_prop(
                    index as usize,
                    glam::Vec3::from(impulse),
                    glam::Vec3::from(torque),
                ) {
                    Ok(Reply::None)
                } else {
                    Err(ErrorCode::NotFound)
                }
            }
            Syscall::StageList => {
                self.require(caller, &Capability::PhysSpawn)?;
                let list: Vec<serde_json::Value> = self
                    .phys
                    .instances()
                    .iter()
                    .enumerate()
                    .map(|(i, (pos, _, shape, half, color))| {
                        serde_json::json!({
                            "index": i,
                            "shape": if *shape == 0 { "cube" } else { "sphere" },
                            "size": half,
                            "color": color,
                            "pos": pos,
                        })
                    })
                    .collect();
                Ok(Reply::Bytes(
                    serde_json::to_string(&list).unwrap_or_default().into_bytes(),
                ))
            }
            Syscall::StageLight {
                dir,
                intensity,
                ambient,
            } => {
                self.require(caller, &Capability::SysQuery)?;
                if let Some(gfx) = self.gfx.as_mut() {
                    let d = glam::Vec3::from(dir).normalize_or(glam::Vec3::new(-0.4, -1.0, -0.3));
                    gfx.light_dir = d.into();
                    gfx.light_intensity = intensity.clamp(0.0, 2.5);
                    gfx.light_ambient = ambient.clamp(0.0, 1.0);
                }
                Ok(Reply::None)
            }
            Syscall::WebFetch { url } => {
                self.require(caller, &Capability::NetLlm)?;
                let id = self.next_web_id;
                self.next_web_id += 1;
                self.web_pending.push((id, url));
                Ok(Reply::Bytes(id.to_string().into_bytes()))
            }
            Syscall::VoiceCapture { start } => {
                self.require(caller, &Capability::VoiceInput)?;
                if self.voice_directives.capture != start {
                    self.voice_directives.capture = start;
                    self.voice_directives.generation += 1;
                }
                Ok(Reply::None)
            }
            other => {
                log::debug!("unimplemented syscall from {caller:?}: {other:?}");
                Err(ErrorCode::Unsupported)
            }
        }
    }

    fn poll_events(&mut self, pid: Pid) -> Vec<KernelEvent> {
        self.events.remove(&pid).unwrap_or_default()
    }
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(k: &mut Kernel, name: &str) -> Pid {
        match k
            .syscall(
                proc::SHELL_PID,
                Syscall::ProcRegister {
                    name: name.into(),
                    caps: vec![],
                },
            )
            .unwrap()
        {
            Reply::Pid(p) => p,
            other => panic!("unexpected reply {other:?}"),
        }
    }

    #[test]
    fn voice_capture_is_gated_and_generation_counted() {
        let mut k = Kernel::new();
        let shell = register(&mut k, "shell"); // first process = shell grant
        let app = register(&mut k, "app"); // default caps only

        assert!(matches!(
            k.syscall(app, Syscall::VoiceCapture { start: true }),
            Err(ErrorCode::CapabilityDenied)
        ));

        let g0 = k.voice_directives.generation;
        k.syscall(shell, Syscall::VoiceCapture { start: true }).unwrap();
        assert!(k.voice_directives.capture);
        assert_eq!(k.voice_directives.generation, g0 + 1);
        // Idempotent: same intent must not re-trigger the platform.
        k.syscall(shell, Syscall::VoiceCapture { start: true }).unwrap();
        assert_eq!(k.voice_directives.generation, g0 + 1);

        // Engine ending on its own syncs intent without a generation bump.
        k.voice_status(false, true, String::new());
        assert!(!k.voice_directives.capture);
        assert_eq!(k.voice_directives.generation, g0 + 1);
    }
}
