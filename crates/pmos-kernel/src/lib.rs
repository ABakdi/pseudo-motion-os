//! The Pseudo Motion OS kernel (Architecture spec §4).
//!
//! Subsystems live in their own modules and are reachable from userland only
//! through the syscall dispatcher below. Nothing here may import `web-sys` —
//! all platform access goes through `pmos-platform` (Architecture §3); wgpu
//! is allowed (it is itself a platform abstraction).

pub mod gfx;
pub mod input;
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

/// The kernel root object, owned by the platform entry point (`pmos-web`).
pub struct Kernel {
    pub procs: proc::ProcessTable,
    pub input: input::InputPipeline,
    /// Graphics engine — installed after the async wgpu device request.
    pub gfx: Option<gfx::Gfx>,
    pub hands_directives: HandsDirectives,
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
            windows: HashMap::new(),
            next_win: 1,
            events: HashMap::new(),
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
                // Release anything the hand was holding (spec §7).
                let pos = self.input.hands.cursor.unwrap_or([0.0, 0.0]);
                self.input.fusion.lost(pos);
            }
            self.publish_hand_state();
        }
    }

    pub fn set_camera_status(&mut self, enabled: bool, reason: String) {
        self.input.hands.camera_enabled = enabled;
        log::info!("camera pipeline enabled: {enabled} ({reason})");
        self.push_event(
            proc::SHELL_PID,
            KernelEvent::CameraStatus { enabled, reason },
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

    fn require(&self, caller: Pid, cap: &Capability) -> Result<(), ErrorCode> {
        if self.procs.has_cap(caller, cap) {
            Ok(())
        } else {
            log::warn!("capability denied for {caller:?}: {cap:?}");
            Err(ErrorCode::CapabilityDenied)
        }
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
                Ok(Reply::Pid(pid))
            }
            Syscall::ProcKill(pid) => {
                // Only the shell (pid 1) may kill other processes in v1.
                if caller != proc::SHELL_PID && caller != pid {
                    return Err(ErrorCode::CapabilityDenied);
                }
                self.procs.kill(pid);
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
                log::debug!("sys query: {path} (synthetic /sys lands in M6)");
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
