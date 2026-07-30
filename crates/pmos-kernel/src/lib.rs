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

use pmos_abi::{Capability, ErrorCode, KernelApi, KernelEvent, Pid, Reply, Syscall, WinId};
use std::collections::HashMap;

pub struct WinRecord {
    pub owner: Pid,
    pub title: String,
    pub size: [f32; 2],
}

/// The kernel root object, owned by the platform entry point (`pmos-web`).
pub struct Kernel {
    pub procs: proc::ProcessTable,
    pub input: input::InputPipeline,
    /// Graphics engine — installed after the async wgpu device request.
    pub gfx: Option<gfx::Gfx>,
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
        }
        self.publish_hand_state();
    }

    /// Per-frame upkeep: tracking-loss timeout and shell notification.
    pub fn tick_hands(&mut self, now: f64) {
        let was_tracking = self.input.hands.tracking;
        self.input.hands.tick(now);
        if was_tracking != self.input.hands.tracking {
            self.publish_hand_state();
        }
    }

    pub fn set_camera_status(&mut self, enabled: bool) {
        self.input.hands.camera_enabled = enabled;
        self.push_event(proc::SHELL_PID, KernelEvent::CameraStatus { enabled });
        log::info!("camera pipeline enabled: {enabled}");
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
            Syscall::ProcRegister { name } => {
                // Registering is unprivileged; new processes start with the
                // default (minimal) capability set. The very first process is
                // by contract the shell and receives the shell grant.
                let caps = if self.procs.is_empty() {
                    proc::shell_caps()
                } else {
                    proc::default_caps()
                };
                let pid = self.procs.register(&name, caps);
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
