//! Process & capability manager (Architecture spec §4.7).
//!
//! A process is kernel bookkeeping: PID + name + capability set. Scheduling is
//! cooperative on the main thread (egui is immediate-mode, so drawing IS the
//! update); consent-driven capability grants arrive with M5's consent sheets.

use pmos_abi::{Capability, Pid};
use std::collections::HashMap;

/// The shell is always process 1 (the kernel's first userland client).
pub const SHELL_PID: Pid = Pid(1);

pub struct Process {
    pub name: String,
    pub caps: Vec<Capability>,
}

/// Minimal default grant for newly registered processes (App DSL spec §10).
pub fn default_caps() -> Vec<Capability> {
    vec![Capability::WinOwn]
}

/// Everything the shell needs; kept explicit rather than a wildcard so the
/// list stays reviewable.
pub fn shell_caps() -> Vec<Capability> {
    vec![
        Capability::WinOwn,
        Capability::SysQuery,
        Capability::ProcSpawnApp,
        Capability::NotesRead,
    ]
}

pub struct ProcessTable {
    next: u32,
    procs: HashMap<Pid, Process>,
}

impl ProcessTable {
    pub fn new() -> Self {
        Self { next: 1, procs: HashMap::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.procs.is_empty()
    }

    pub fn register(&mut self, name: &str, caps: Vec<Capability>) -> Pid {
        let pid = Pid(self.next);
        self.next += 1;
        log::info!("proc {pid:?} registered: {name}");
        self.procs.insert(pid, Process { name: name.to_string(), caps });
        pid
    }

    pub fn kill(&mut self, pid: Pid) {
        if let Some(p) = self.procs.remove(&pid) {
            log::info!("proc {pid:?} ({}) killed", p.name);
        }
    }

    pub fn has_cap(&self, pid: Pid, cap: &Capability) -> bool {
        self.procs.get(&pid).is_some_and(|p| p.caps.contains(cap))
    }

    pub fn grant(&mut self, pid: Pid, caps: Vec<Capability>) {
        if let Some(p) = self.procs.get_mut(&pid) {
            for c in caps {
                if !p.caps.contains(&c) {
                    p.caps.push(c);
                }
            }
        }
    }
}

impl Default for ProcessTable {
    fn default() -> Self {
        Self::new()
    }
}
