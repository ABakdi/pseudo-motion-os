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
        Capability::InputRawHands,
        Capability::VoiceInput,
        Capability::PhysSpawn,
        Capability::NetLlm,
        Capability::AiPrompt,
        Capability::FsRead("/".into()),
        Capability::FsWrite("/".into()),
    ]
}

fn scope_matches(scope: &str, path: &str) -> bool {
    let scope = scope.trim_end_matches('/');
    scope.is_empty() || path == scope || path.starts_with(&format!("{scope}/"))
}

pub struct ProcessTable {
    next: u32,
    procs: HashMap<Pid, Process>,
}

impl ProcessTable {
    pub fn new() -> Self {
        Self {
            next: 1,
            procs: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.procs.is_empty()
    }

    pub fn register(&mut self, name: &str, caps: Vec<Capability>) -> Pid {
        let pid = Pid(self.next);
        self.next += 1;
        log::info!("proc {pid:?} registered: {name}");
        self.procs.insert(
            pid,
            Process {
                name: name.to_string(),
                caps,
            },
        );
        pid
    }

    pub fn kill(&mut self, pid: Pid) {
        if let Some(p) = self.procs.remove(&pid) {
            log::info!("proc {pid:?} ({}) killed", p.name);
        }
    }

    /// One line per process, for /sys/processes (M6 deferral).
    pub fn listing(&self) -> String {
        let mut rows: Vec<(u32, String)> = self
            .procs
            .iter()
            .map(|(pid, p)| (pid.0, format!("{:>4}  {:<14} {} caps", pid.0, p.name, p.caps.len())))
            .collect();
        rows.sort();
        let mut out = String::from(" PID  NAME           GRANTS\n");
        for (_, row) in rows {
            out.push_str(&row);
            out.push('\n');
        }
        out
    }

    pub fn has_cap(&self, pid: Pid, cap: &Capability) -> bool {
        self.procs.get(&pid).is_some_and(|p| p.caps.contains(cap))
    }

    /// Scoped filesystem check: any held FsRead/FsWrite whose scope is a
    /// path-prefix of `path` grants access (write implies read).
    pub fn has_fs_cap(&self, pid: Pid, path: &str, write: bool) -> bool {
        let Some(p) = self.procs.get(&pid) else {
            return false;
        };
        p.caps.iter().any(|c| match c {
            Capability::FsWrite(scope) => scope_matches(scope, path),
            Capability::FsRead(scope) if !write => scope_matches(scope, path),
            _ => false,
        })
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
