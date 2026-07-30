//! The Pseudo Motion OS syscall ABI.
//!
//! This crate is the *only* thing userland (`pmos-apps`, Conjure apps via the
//! App Host) may link against to talk to the kernel. Every payload is
//! serde-serializable by design: v1 dispatches in-memory, but the same types
//! must survive a postMessage boundary when out-of-process apps arrive.
//! Spec: docs/Architecture.md §6.

use serde::{Deserialize, Serialize};

/// (major, minor). Additive changes bump minor; breaking changes bump major.
pub const ABI_VERSION: (u16, u16) = (1, 1);

// ---------- handles ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pid(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WinId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextureHandle(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BodyId(pub u32);

// ---------- capabilities ----------

/// Capability grants checked on every syscall (Architecture §4.7).
/// Scoped variants carry their scope as a path glob.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    WinOwn,
    FsRead(String),
    FsWrite(String),
    NotesRead,
    NotesWrite,
    NotesLink,
    NetLlm,
    AiPrompt,
    PhysSpawn,
    InputRawHands,
    SysQuery,
    ProcSpawnApp,
    ClipboardWrite,
    TimerSchedule,
}

// ---------- syscalls ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WinDesc {
    pub title: String,
    pub size: [f32; 2],
    pub resizable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Syscall {
    // process registration (built-in apps register at open; Conjure apps
    // are registered by the App Host)
    ProcRegister {
        name: String,
    },
    // window
    WinCreate(WinDesc),
    WinClose(WinId),
    WinSetTitle(WinId, String),
    // vfs
    FsRead {
        path: String,
    },
    FsWrite {
        path: String,
        bytes: Vec<u8>,
    },
    FsList {
        path: String,
    },
    FsWatch {
        path: String,
    },
    // input
    InputSubscribe {
        raw_hands: bool,
    },
    // ai
    AiPrompt {
        agent: AgentId,
        msg: String,
    },
    // process
    ProcSpawnApp {
        conjure_doc: String,
    },
    ProcKill(Pid),
    CapRequest {
        caps: Vec<Capability>,
        reason: String,
    },
    // system
    SysQuery {
        path: String,
    },
}

// ---------- events ----------

/// Where an input event came from. Apps normally must NOT branch on this —
/// modality parity (UI spec §3.2) — but the shell uses it for cursor styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputSource {
    Mouse,
    Keyboard,
    Hand,
    Voice,
    Synthetic,
}

/// Recognized hand poses driving the morphing cursor (Hand Gestures spec §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandPose {
    Rest,
    Point,
    Pinch,
    MiddlePinch,
    Grab,
    OpenPalm,
    TwoFinger,
    CallSign,
    ThumbsUp,
    ThumbsDown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KernelEvent {
    PointerMove {
        pos: [f32; 2],
        source: InputSource,
    },
    Select {
        pos: [f32; 2],
        source: InputSource,
    },
    /// Per-frame hand state (ABI 1.1): drives the morphing cursor
    /// (UI spec §4) and the tray indicator.
    HandUpdate {
        pose: HandPose,
        /// Pinch closure 0..1 — the cursor ring tightens with this.
        pinch: f32,
        /// Cursor position in egui points (already control-box mapped and
        /// One-Euro filtered). None while no hand is tracked.
        pos: Option<[f32; 2]>,
        tracking: bool,
        hands: u8,
    },
    /// Camera pipeline availability (ABI 1.1): permission granted and the
    /// gesture worker is live.
    CameraStatus {
        enabled: bool,
    },
    Key {
        code: u32,
        pressed: bool,
    },
    WinClosed(WinId),
    FsChanged {
        path: String,
    },
    AiChunk {
        agent: AgentId,
        text: String,
        done: bool,
    },
    CapGranted {
        caps: Vec<Capability>,
    },
    CapDenied {
        caps: Vec<Capability>,
    },
    SyscallError {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    CapabilityDenied,
    NotFound,
    InvalidArgument,
    Unsupported,
}

/// Synchronous syscall replies. v1 dispatches in-memory so cheap syscalls
/// answer directly; results that take time (I/O, AI) still arrive as
/// [`KernelEvent`]s — see Architecture §6 ("async by default").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Reply {
    None,
    Pid(Pid),
    Win(WinId),
}

/// The kernel as seen from userland. `pmos-apps` receives `&mut dyn KernelApi`
/// and nothing else — the crate graph guarantees userland can only do what
/// this trait (and therefore the capability checks behind it) allows.
pub trait KernelApi {
    fn syscall(&mut self, caller: Pid, call: Syscall) -> Result<Reply, ErrorCode>;
    fn poll_events(&mut self, pid: Pid) -> Vec<KernelEvent>;
}
