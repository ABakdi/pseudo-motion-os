//! The Pseudo Motion OS kernel (Architecture spec §4).
//!
//! Subsystems live in their own modules and are reachable from userland only
//! through the syscall dispatcher. Nothing here may import `web-sys` — all
//! platform access goes through the traits in `pmos-platform` (Architecture §3).

pub mod gfx;
pub mod input;
pub mod proc;
pub mod vfs;

use pmos_abi::{KernelEvent, Pid, Syscall};

/// The kernel root object, owned by the platform entry point (`pmos-web`).
pub struct Kernel {
    // Subsystems attach here as milestones land (docs/Todo.md).
}

impl Kernel {
    pub fn new() -> Self {
        log::info!("pmos-kernel init, ABI {:?}", pmos_abi::ABI_VERSION);
        Self {}
    }

    /// Syscall entry point. Capability checking happens here, before any
    /// subsystem sees the call (Architecture §6).
    pub fn syscall(&mut self, caller: Pid, call: Syscall) {
        // Dispatcher lands with milestone 2 (kernel ABI & process model).
        log::debug!("syscall from {caller:?}: {call:?}");
    }

    /// Drain pending events for a process. Stub until the process model lands.
    pub fn poll_events(&mut self, _for: Pid) -> Vec<KernelEvent> {
        Vec::new()
    }
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}
