//! The Pseudo Motion OS syscall ABI.
//!
//! This crate is the *only* thing userland (`pmos-apps`, Conjure apps via the
//! App Host) may link against to talk to the kernel. Every payload is
//! serde-serializable by design: v1 dispatches in-memory, but the same types
//! must survive a postMessage boundary when out-of-process apps arrive.
//! Spec: docs/Architecture.md §6.

use serde::{Deserialize, Serialize};

/// (major, minor). Additive changes bump minor; breaking changes bump major.
pub const ABI_VERSION: (u16, u16) = (1, 3);

// ---------- handles ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pid(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WinId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextureHandle(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub u32);

/// Built-in agents (AI System spec §1).
pub const AGENT_ASSISTANT: AgentId = AgentId(1);
pub const AGENT_APP_SMITH: AgentId = AgentId(2);

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
    // Process registration (built-in apps register at open; Conjure apps are
    // registered by the App Host). `caps` requests extra capabilities beyond
    // the minimal default — granted only by DELEGATION: each is honored iff
    // the registering caller itself holds it (ABI 1.2).
    ProcRegister {
        name: String,
        caps: Vec<Capability>,
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
    FsDelete {
        path: String,
    },
    FsMkdir {
        path: String,
    },
    // input
    InputSubscribe {
        raw_hands: bool,
    },
    /// Start the camera pipeline (or retry after an earlier denial). Used by
    /// the Hand Tracker app; the browser permission prompt fires from the
    /// user's click (ABI 1.2). Requires `InputRawHands`.
    CameraStart,
    /// Hand-tracker viewer state. `open` gates RawHands landmark events;
    /// `stream_feed` additionally makes the platform stream preview pixels —
    /// directly to the shell overlay, NEVER through the kernel (Hand
    /// Gestures spec §7). Requires `InputRawHands`.
    HandsViewer {
        open: bool,
        stream_feed: bool,
    },
    /// Tune the gesture pipeline (ABI 1.2). Worker-side fields (num_hands,
    /// confidences) are applied by the platform; recognizer fields
    /// (smoothing preset, pinch thresholds) by the kernel.
    HandsTune(HandsTuning),
    // ai
    AiPrompt {
        agent: AgentId,
        msg: String,
    },
    /// Set the LLM provider profile (AI System spec §2). The key is stored
    /// kernel-side and is never readable back through any syscall.
    AiConfigure(AiProviderConfig),
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

/// Gesture-pipeline tuning (Hand Gestures spec §6 defaults).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HandsTuning {
    pub num_hands: u8,
    /// MediaPipe minimum detection / tracking confidence, 0..1.
    pub det_conf: f32,
    pub track_conf: f32,
    /// Cursor smoothing preset: 0 = Precise, 1 = Balanced, 2 = Smooth.
    pub smoothing: u8,
    /// Pinch hysteresis thresholds, normalized by palm scale.
    pub pinch_enter: f32,
    pub pinch_exit: f32,
}

impl Default for HandsTuning {
    fn default() -> Self {
        Self {
            num_hands: 2,
            det_conf: 0.5,
            track_conf: 0.5,
            smoothing: 1,
            pinch_enter: 0.35,
            pinch_exit: 0.55,
        }
    }
}

/// LLM provider profile (AI System spec §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiProviderConfig {
    /// 0 = Anthropic (direct browser), 1 = OpenAI-compatible (incl. local
    /// servers like Ollama / LM Studio).
    pub kind: u8,
    /// Empty = provider default endpoint.
    pub base_url: String,
    pub model: String,
    pub api_key: String,
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
    /// Camera pipeline availability (ABI 1.1; reason added in 1.2): when
    /// disabled, `reason` explains why (permission denied, no device, loader
    /// failure) so the UI can guide the user. Empty = no detail.
    CameraStatus {
        enabled: bool,
        reason: String,
    },
    /// Raw landmark frame (ABI 1.2): `hands × 21 × [x,y,z]` normalized camera
    /// coordinates. Emitted only while the hand-tracker viewer is open, and
    /// only to processes holding `InputRawHands`.
    RawHands {
        data: Vec<f32>,
        hands: u8,
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

/// A directory listing entry (ABI 1.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

/// Synchronous syscall replies. v1 dispatches in-memory so cheap syscalls
/// answer directly; results that take time (I/O, AI) still arrive as
/// [`KernelEvent`]s — see Architecture §6 ("async by default").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Reply {
    None,
    Pid(Pid),
    Win(WinId),
    Bytes(Vec<u8>),
    Entries(Vec<DirEntry>),
}

/// The kernel as seen from userland. `pmos-apps` receives `&mut dyn KernelApi`
/// and nothing else — the crate graph guarantees userland can only do what
/// this trait (and therefore the capability checks behind it) allows.
pub trait KernelApi {
    fn syscall(&mut self, caller: Pid, call: Syscall) -> Result<Reply, ErrorCode>;
    fn poll_events(&mut self, pid: Pid) -> Vec<KernelEvent>;
}
