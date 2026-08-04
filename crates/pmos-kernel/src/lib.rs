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
    /// Gaze assist opt-in (Settings → Face → /settings/face.json "gaze");
    /// set by the platform. Off = gaze scalars are dropped at the door.
    pub gaze_enabled: bool,
    /// Calibrated per-user gaze mapping (ABI 1.17). None = coarse heuristic.
    pub gaze_calib: Option<input::gaze::GazeCalib>,
    /// Latest raw gaze feature vector from the worker (camera space).
    gaze_feats: Option<Vec<f32>>,
    /// (features, screen target) pairs from the calibration overlay.
    calib_samples: Vec<(Vec<f32>, f32, f32)>,
    /// Previous frame's two-palm spread (camera-space) for zoom deltas.
    last_palm_spread: Option<f32>,
    /// Previous frame's second-hand pose+centroid (property-control deltas).
    last_hand2: Option<(pmos_abi::HandPose, (f32, f32))>,
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
            gaze_enabled: false,
            gaze_calib: None,
            gaze_feats: None,
            calib_samples: Vec::new(),
            last_palm_spread: None,
            last_hand2: None,
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
        if done {
            self.persist_ai_usage();
        }
    }

    /// Write the budget meter through the VFS when it changed (the platform
    /// mirrors /settings writes to OPFS like any other file).
    fn persist_ai_usage(&mut self) {
        if !self.ai.usage_dirty {
            return;
        }
        self.ai.usage_dirty = false;
        let json = serde_json::json!({
            "month": self.ai.usage.0,
            "used": self.ai.usage.1,
        })
        .to_string();
        let _ = self.vfs.write("/settings/ai_usage.json", json.into_bytes());
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

        // Non-dominant property controls (CSL spec §5): with an object
        // FOCUSED, the second hand edits it — ✌ vertical drag scales,
        // ✊ horizontal drag spins.
        let hand2 = self.input.hands.hand2;
        if let (Some(i), Some((pose2, c2))) = (self.phys.focused, hand2) {
            if let Some((_, prev)) = self.last_hand2 {
                let (dx, dy) = (c2.0 - prev.0, c2.1 - prev.1);
                match pose2 {
                    pmos_abi::HandPose::TwoFinger => {
                        // Up (dy<0, image coords) grows the object.
                        self.phys.scale_prop(i, 1.0 - dy * 3.0);
                    }
                    pmos_abi::HandPose::Grab => {
                        self.phys
                            .impulse_prop(i, glam::Vec3::ZERO, glam::Vec3::new(0.0, -dx * 40.0, 0.0));
                    }
                    _ => {}
                }
            }
        }
        self.last_hand2 = hand2.map(|(p, c)| (p, c));

        // CSL sign recognition rides the same debounced pose stream.
        // Face-anchored COMMAND (CSL §6): when the face is tracked, the ☝
        // hold must happen NEAR THE CHIN (the true ASL anchor); without face
        // data it works anywhere (fallback).
        let near_face = self.face.chin.and_then(|(chin, t)| {
            if now - t > 1.0 {
                return None; // stale face data → fallback
            }
            let c = self.input.hands.palm_centroid;
            let d = ((c.0 - chin.0).powi(2) + (c.1 - chin.1).powi(2)).sqrt();
            Some(d < 0.28)
        });
        if let Some(sign) = self.input.signs.step(
            self.input.hands.pose,
            self.input.hands.cursor,
            self.input.hands.palm_scale,
            near_face,
            self.input.hands.tracking,
            now,
        ) {
            if sign == pmos_abi::CslSign::Cancel {
                self.phys.focused = None; // CANCEL clears object focus too
            }
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

    /// A system notice → shell toast (ABI 1.15) — e.g. face-engine status.
    pub fn notice(&mut self, text: String) {
        self.push_event(proc::SHELL_PID, KernelEvent::Notice { text });
    }

    /// Smooth a gaze estimate and stream it to the shell. Heavy EMA — a
    /// highlight that flickers is worse than one that lags a little.
    fn emit_gaze(&mut self, x: f32, y: f32) {
        let target = (x, y);
        let (px, py) = self.face.gaze.unwrap_or(target);
        const ALPHA: f32 = 0.25;
        let sm = (px + (target.0 - px) * ALPHA, py + (target.1 - py) * ALPHA);
        self.face.gaze = Some(sm);
        self.face.gaze_announced = true;
        self.push_event(
            proc::SHELL_PID,
            KernelEvent::Gaze {
                x: sm.0,
                y: sm.1,
                active: true,
            },
        );
    }

    /// Per-frame gaze feature vector from the worker (ABI 1.17). With a
    /// calibration fitted, this IS the gaze estimate — accurate to a few
    /// percent of the screen instead of thirds; the heuristic never runs.
    pub fn gaze_features(&mut self, v: Vec<f32>) {
        if self.gaze_enabled {
            if let Some(calib) = &self.gaze_calib {
                if let Some((x, y)) = calib.predict(&v) {
                    // Allow slight over-range so edge targets aren't biased
                    // inward; the shell clamps for display.
                    self.emit_gaze(x.clamp(-0.2, 1.2), y.clamp(-0.2, 1.2));
                }
            }
        }
        self.gaze_feats = Some(v);
    }

    /// Face mesh frame → viewer overlay (ABI 1.16). Same policy as raw hand
    /// landmarks: only while the viewer is open, only to InputRawHands.
    pub fn face_mesh(&mut self, data: Vec<f32>) {
        if self.hands_directives.viewer_open
            && self
                .procs
                .has_cap(proc::SHELL_PID, &Capability::InputRawHands)
        {
            self.push_event(proc::SHELL_PID, KernelEvent::RawFace { data });
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
                let _ = self.input.signs.step(pmos_abi::HandPose::Rest, None, 0.1, None, false, now);
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
        // Props are first in the instance list; notes-graph nodes follow.
        // Only real stage objects mirror into the ray-traced scene.
        let stage_count = self.phys.props.len();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.render(
                primitives,
                textures_delta,
                pixels_per_point,
                time,
                &instances,
                stage_count,
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

    /// Index of the currently grabbed prop, if any (pinch-tap focus).
    pub fn grabbed_prop_index(&self) -> Option<usize> {
        let (body, _) = self.phys.grab_handle()?;
        self.phys.props.iter().position(|p| p.body == body)
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
    jaw_since: Option<f64>,
    jaw_fired: bool,
    brow_since: Option<f64>,
    brow_fired: bool,
    /// Chin landmark (camera space) + timestamp — anchors COMMAND (CSL §6).
    chin: Option<((f32, f32), f64)>,
    /// Smoothed gaze estimate (screen fractions, mirrored) — CSL §6.
    gaze: Option<(f32, f32)>,
    /// Whether the last Gaze event announced `active` — lost/off sends one
    /// `active = false` so the shell can drop the highlight immediately.
    gaze_announced: bool,
}

impl Kernel {
    /// Face frame from the platform: blinks, jawOpen, chin landmark, and the
    /// coarse gaze estimate (M10). Gaze arrives camera-image-normalized and
    /// leaves as a mirrored, EMA-smoothed `Gaze` event — region accuracy
    /// only (CSL spec §6): it soft-highlights, it never moves the cursor.
    #[allow(clippy::too_many_arguments)]
    pub fn face_frame(
        &mut self,
        blink_l: f32,
        blink_r: f32,
        jaw: f32,
        brow: f32,
        chin_x: f32,
        chin_y: f32,
        gaze_x: f32,
        gaze_y: f32,
        now: f64,
    ) {
        if self.gaze_enabled && gaze_x >= 0.0 {
            // Calibrated users get their prediction from gaze_features()
            // (which arrives with the same face message); the heuristic
            // only runs uncalibrated. Mirror x — camera vs. screen, the
            // same convention as the hands.
            if self.gaze_calib.is_none() {
                self.emit_gaze(1.0 - gaze_x, gaze_y);
            }
        } else if self.face.gaze_announced {
            self.face.gaze = None;
            self.face.gaze_announced = false;
            self.push_event(
                proc::SHELL_PID,
                KernelEvent::Gaze {
                    x: 0.0,
                    y: 0.0,
                    active: false,
                },
            );
        }
        // Brow-raise held 0.4 s → Confirm (consent sheets, UI spec §5).
        if brow > 0.5 {
            let since = *self.face.brow_since.get_or_insert(now);
            if now - since > 0.4 && !self.face.brow_fired {
                self.face.brow_fired = true;
                self.push_event(
                    proc::SHELL_PID,
                    KernelEvent::Sign {
                        sign: pmos_abi::CslSign::Confirm,
                    },
                );
            }
        } else {
            self.face.brow_since = None;
            self.face.brow_fired = false;
        }
        self.face.chin = if chin_x >= 0.0 {
            Some(((chin_x, chin_y), now))
        } else {
            None
        };
        // Mouth held wide open ~0.6 s = hands-free voice toggle (the face
        // layer's RECORD equivalent). Must close the mouth to re-arm.
        if jaw > 0.55 {
            let since = *self.face.jaw_since.get_or_insert(now);
            if now - since > 0.6 && !self.face.jaw_fired {
                self.face.jaw_fired = true;
                self.push_event(
                    proc::SHELL_PID,
                    KernelEvent::Sign {
                        sign: pmos_abi::CslSign::Record,
                    },
                );
            }
        } else {
            self.face.jaw_since = None;
            self.face.jaw_fired = false;
        }
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

    /// Scan /notes for markdown + wikilinks and spawn the graph (ABI 1.12).
    fn build_notes_graph(&mut self) {
        // Collect (title, path, body) for every .md under /notes.
        let mut files: Vec<(String, String)> = Vec::new(); // (path, text)
        let mut stack = vec!["/notes".to_string()];
        while let Some(dir) = stack.pop() {
            let Some(entries) = self.vfs.list(&dir) else {
                continue;
            };
            for e in entries {
                let full = format!("{dir}/{}", e.name);
                if e.dir {
                    stack.push(full);
                } else if e.name.ends_with(".md") {
                    if let Some(bytes) = self.vfs.read(&full) {
                        files.push((full, String::from_utf8_lossy(&bytes).into_owned()));
                    }
                }
            }
            if files.len() >= 100 {
                break; // O(n²) layout budget
            }
        }
        let title_of = |path: &str| -> String {
            path.rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".md")
                .to_string()
        };
        // Spawn nodes on a ring above the stage; layout forces take over.
        let n = files.len().max(1) as f32;
        for (i, (path, _)) in files.iter().enumerate() {
            let a = i as f32 / n * std::f32::consts::TAU;
            self.phys.spawn_note(
                glam::Vec3::new(a.cos() * 2.2, 3.4 + (i % 3) as f32 * 0.4, a.sin() * 2.2),
                title_of(path),
                path.clone(),
            );
        }
        // Wikilinks: [[Target]] → spring to the note titled Target.
        for (i, (_, text)) in files.iter().enumerate() {
            for part in text.split("[[").skip(1) {
                let Some(end) = part.find("]]") else { continue };
                let target = part[..end].trim().to_lowercase();
                if let Some(j) = files
                    .iter()
                    .position(|(p, _)| title_of(p).to_lowercase() == target)
                {
                    if j != i && !self.phys.note_links.contains(&(j.min(i), j.max(i))) {
                        self.phys.note_links.push((j.min(i), j.max(i)));
                    }
                }
            }
        }
        log::info!(
            "notes graph: {} nodes, {} links",
            self.phys.notes.len(),
            self.phys.note_links.len()
        );
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
                // Budget bookkeeping: the soft warning surfaces as a toast,
                // and the meter persists (hard stops changed it too).
                if let Some(text) = self.ai.pending_notice.take() {
                    self.ai_log(text.clone());
                    self.push_event(proc::SHELL_PID, KernelEvent::Notice { text });
                }
                self.persist_ai_usage();
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
                // Props only — graph nodes are notes, not stage objects.
                let list: Vec<serde_json::Value> = self
                    .phys
                    .prop_instances()
                    .iter()
                    .enumerate()
                    .map(|(i, (pos, _, shape, half, color))| {
                        serde_json::json!({
                            "index": i,
                            "shape": if *shape == 0 { "cube" } else { "sphere" },
                            "size": half,
                            "color": color,
                            "pos": pos,
                            "focused": self.phys.focused == Some(i),
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
            Syscall::GraphShow { show } => {
                self.require(caller, &Capability::NotesRead)?;
                self.phys.clear_notes();
                if show {
                    self.build_notes_graph();
                }
                Ok(Reply::None)
            }
            Syscall::GraphLabels { viewport } => {
                self.require(caller, &Capability::NotesRead)?;
                let Some(gfx) = self.gfx.as_ref() else {
                    return Ok(Reply::Bytes(b"{}".to_vec()));
                };
                let vp = gfx.camera.view_proj(viewport[0].max(1.0) / viewport[1].max(1.0));
                let mut nodes = Vec::new();
                for n in &self.phys.notes {
                    let Some(body) = self.phys.bodies.get(n.body) else {
                        continue;
                    };
                    let t = body.translation();
                    let clip = vp * glam::Vec4::new(t.x, t.y, t.z, 1.0);
                    if clip.w <= 0.05 {
                        nodes.push(serde_json::Value::Null); // behind camera
                        continue;
                    }
                    let ndc = clip.truncate() / clip.w;
                    nodes.push(serde_json::json!({
                        "title": n.title,
                        "path": n.path,
                        "x": (ndc.x * 0.5 + 0.5) * viewport[0],
                        "y": (0.5 - ndc.y * 0.5) * viewport[1],
                        "depth": clip.w,
                    }));
                }
                let json = serde_json::json!({
                    "nodes": nodes,
                    "links": self.phys.note_links,
                });
                Ok(Reply::Bytes(json.to_string().into_bytes()))
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
            Syscall::GazeCalib(op) => {
                // Same trust level as raw landmarks — the shell's overlay.
                self.require(caller, &Capability::InputRawHands)?;
                match op {
                    pmos_abi::GazeCalibOp::Sample { x, y } => {
                        if let Some(f) = &self.gaze_feats {
                            if self.calib_samples.len() < 2048 {
                                self.calib_samples.push((f.clone(), x, y));
                            }
                        }
                        // Reply with the running count so the overlay can
                        // warn LIVE when nothing is accumulating (face not
                        // tracked) instead of failing at the very end.
                        return Ok(Reply::Bytes(
                            self.calib_samples.len().to_string().into_bytes(),
                        ));
                    }
                    pmos_abi::GazeCalibOp::Finish => {
                        let samples = std::mem::take(&mut self.calib_samples);
                        let valid = samples
                            .iter()
                            .filter(|(f, _, _)| input::gaze::expand(f).is_some())
                            .count();
                        match input::gaze::fit(&samples) {
                            Some((calib, err)) => {
                                if let Ok(json) = serde_json::to_vec(&calib) {
                                    let _ = self.vfs.write("/settings/gaze_calib.json", json);
                                }
                                self.gaze_calib = Some(calib);
                                self.notice(format!(
                                    "🎯 gaze calibrated — mean error ≈ {:.0}% of the screen",
                                    err * 100.0
                                ));
                            }
                            None => self.notice(format!(
                                "⚠ gaze calibration failed — {valid} usable samples of {} (need 20+). \
                                 Face tracking must be running: the Hand Tracker shows your face as a dot mesh when it is",
                                samples.len()
                            )),
                        }
                    }
                    pmos_abi::GazeCalibOp::Cancel => self.calib_samples.clear(),
                    pmos_abi::GazeCalibOp::Reset => {
                        self.gaze_calib = None;
                        self.calib_samples.clear();
                        let _ = self.vfs.delete("/settings/gaze_calib.json");
                        self.notice("gaze calibration cleared — back to the coarse estimate".into());
                    }
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

    #[test]
    fn all_shaders_parse_and_validate() {
        // Shaders otherwise fail only at runtime, on the GPU, in the browser
        // — this is the compile step they never had.
        for (name, src) in [
            ("sky", include_str!("gfx/sky.wgsl")),
            ("floor", include_str!("gfx/floor.wgsl")),
            ("props", include_str!("gfx/props.wgsl")),
            ("rt", include_str!("gfx/rt.wgsl")),
        ] {
            let module = naga::front::wgsl::parse_str(src)
                .unwrap_or_else(|e| panic!("{name}.wgsl parse: {}", e.emit_to_string(src)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}.wgsl validate: {e:?}"));
        }
    }

    #[test]
    fn ai_budget_meters_warns_and_hard_stops() {
        let mut k = Kernel::new();
        let shell = register(&mut k, "shell");
        k.ai.month = "2026-08".into();
        // Remote provider with a small cap (the system prompt alone is
        // ~1.5k estimated tokens, so leave headroom for a few requests).
        k.ai.set_config(
            pmos_abi::AiProviderConfig {
                kind: 1,
                base_url: "http://localhost:1".into(),
                model: "m".into(),
                api_key: String::new(),
                monthly_cap: 6000,
            },
            false,
        );
        // ~2000 chars ≈ 500 est. tokens per request (plus prompt overhead).
        let msg = "x".repeat(2000);
        k.syscall(
            shell,
            Syscall::AiPrompt {
                agent: pmos_abi::AGENT_ASSISTANT,
                msg: msg.clone(),
            },
        )
        .unwrap();
        assert!(k.ai.usage.1 > 0, "request side must be metered");
        assert_eq!(k.ai.usage.0, "2026-08");
        // Finish the stream; the reply side counts too and usage persists.
        k.ai_chunk(pmos_abi::AGENT_ASSISTANT.0, "y".repeat(400), true);
        let after_reply = k.ai.usage.1;
        assert!(after_reply >= 100);
        assert!(k.vfs.read("/settings/ai_usage.json").is_some());
        // Drive over the cap → hard stop as a terminal ⚠ chunk.
        let mut stopped = false;
        for _ in 0..8 {
            k.syscall(
                shell,
                Syscall::AiPrompt {
                    agent: pmos_abi::AGENT_ASSISTANT,
                    msg: msg.clone(),
                },
            )
            .unwrap();
            let evs = k.poll_events(shell);
            if evs.iter().any(|e| matches!(
                e,
                KernelEvent::AiChunk { text, done: true, .. } if text.contains("budget")
            )) {
                stopped = true;
                break;
            }
            k.ai_chunk(pmos_abi::AGENT_ASSISTANT.0, "ok".into(), true);
        }
        assert!(stopped, "the cap must hard-stop, never silently exceed");
        // Requests are refused before exceeding; only reply-side metering
        // can nudge past the line, and replies here are tiny.
        assert!(k.ai.usage.1 <= 6000 + 600, "usage stops growing at the cap");
        // The in-browser provider is never metered.
        let before = k.ai.usage.1;
        k.ai.set_config(
            pmos_abi::AiProviderConfig {
                kind: 2,
                base_url: String::new(),
                model: ai::WEBLLM_DEFAULT_MODEL.into(),
                api_key: String::new(),
                monthly_cap: 1000,
            },
            false,
        );
        k.syscall(
            shell,
            Syscall::AiPrompt {
                agent: pmos_abi::AGENT_ASSISTANT,
                msg,
            },
        )
        .unwrap();
        assert_eq!(k.ai.usage.1, before);
    }

    #[test]
    fn gaze_calibration_end_to_end() {
        let mut k = Kernel::new();
        let shell = register(&mut k, "shell");
        let app = register(&mut k, "app");
        // Capability-gated like raw landmarks.
        assert!(matches!(
            k.syscall(
                app,
                Syscall::GazeCalib(pmos_abi::GazeCalibOp::Sample { x: 0.5, y: 0.5 })
            ),
            Err(ErrorCode::CapabilityDenied)
        ));

        k.gaze_enabled = true;
        // Synthetic eye: screen x tracks feature 0, screen y tracks feature
        // 5, everything else constant — the fit must recover this mapping.
        let feats_for = |tx: f32, ty: f32, jitter: f32| -> Vec<f32> {
            let mut f = vec![0.3f32; input::gaze::FEATS];
            f[0] = (tx - 0.1) / 0.8 + jitter * 0.002;
            f[5] = (ty - 0.2) / 0.6 - jitter * 0.002;
            f
        };
        let targets = [0.08f32, 0.5, 0.92];
        for &tx in &targets {
            for &ty in &targets {
                for j in 0..15 {
                    k.gaze_features(feats_for(tx, ty, j as f32));
                    k.syscall(shell, Syscall::GazeCalib(pmos_abi::GazeCalibOp::Sample { x: tx, y: ty }))
                        .unwrap();
                }
            }
        }
        k.syscall(shell, Syscall::GazeCalib(pmos_abi::GazeCalibOp::Finish))
            .unwrap();
        assert!(k.gaze_calib.is_some(), "fit must succeed on clean data");
        assert!(
            k.vfs.read("/settings/gaze_calib.json").is_some(),
            "calibration persists"
        );
        // Prediction: a fresh unseen point lands where it should.
        let _ = k.poll_events(proc::SHELL_PID); // drain
        k.gaze_features(feats_for(0.7, 0.62, 0.0));
        let ev = k
            .poll_events(proc::SHELL_PID)
            .into_iter()
            .find_map(|e| match e {
                KernelEvent::Gaze { x, y, active: true } => Some((x, y)),
                _ => None,
            })
            .expect("calibrated gaze emits");
        assert!((ev.0 - 0.7).abs() < 0.05, "x {}", ev.0);
        assert!((ev.1 - 0.62).abs() < 0.05, "y {}", ev.1);
        // Reset forgets everything.
        k.syscall(shell, Syscall::GazeCalib(pmos_abi::GazeCalibOp::Reset))
            .unwrap();
        assert!(k.gaze_calib.is_none());
        assert!(k.vfs.read("/settings/gaze_calib.json").is_none());
    }

    #[test]
    fn gaze_is_opt_in_mirrored_smoothed_and_announces_loss() {
        let mut k = Kernel::new();
        let _shell = register(&mut k, "shell");
        let gaze_events = |k: &mut Kernel| -> Vec<(f32, f32, bool)> {
            k.poll_events(proc::SHELL_PID)
                .into_iter()
                .filter_map(|e| match e {
                    KernelEvent::Gaze { x, y, active } => Some((x, y, active)),
                    _ => None,
                })
                .collect()
        };

        // Off by default: gaze scalars are dropped at the door.
        k.face_frame(0.0, 0.0, 0.0, 0.0, -1.0, -1.0, 0.8, 0.5, 0.0);
        assert!(gaze_events(&mut k).is_empty());

        k.gaze_enabled = true;
        // First frame seeds the filter directly (no lag from a fake origin)
        // and x is mirrored: camera 0.8 → screen 0.2.
        k.face_frame(0.0, 0.0, 0.0, 0.0, -1.0, -1.0, 0.8, 0.5, 0.1);
        let evs = gaze_events(&mut k);
        assert_eq!(evs.len(), 1);
        let (x0, y0, active) = evs[0];
        assert!(active);
        assert!((x0 - 0.2).abs() < 1e-5 && (y0 - 0.5).abs() < 1e-5);

        // A jump only moves the estimate fractionally (EMA smoothing).
        k.face_frame(0.0, 0.0, 0.0, 0.0, -1.0, -1.0, 0.0, 0.5, 0.2);
        let (x1, _, _) = gaze_events(&mut k)[0];
        assert!(x1 > x0 && x1 < 1.0 - 0.8 * 0.5, "smoothed, not teleported");

        // Face lost → exactly one active=false, then silence.
        k.face_frame(0.0, 0.0, 0.0, 0.0, -1.0, -1.0, -1.0, -1.0, 0.3);
        let evs = gaze_events(&mut k);
        assert_eq!(evs.len(), 1);
        assert!(!evs[0].2);
        k.face_frame(0.0, 0.0, 0.0, 0.0, -1.0, -1.0, -1.0, -1.0, 0.4);
        assert!(gaze_events(&mut k).is_empty());
    }
}
