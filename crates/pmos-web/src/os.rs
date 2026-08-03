//! The OS runtime: winit event loop on the stage canvas, async wgpu init,
//! per-frame glue between kernel, shell and egui (Architecture §7).

use pmos_apps::shell::Shell;
use pmos_kernel::wgpu;
use pmos_kernel::{gfx::Gfx, Kernel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys};
use winit::window::{Window, WindowId};

/// Called by the landing page after the Launch click and permission
/// onboarding (Architecture §9.1). `permissions_json` carries the onboarding
/// results: `{"camera":bool,"microphone":bool,"notifications":bool}`.
#[wasm_bindgen]
pub fn pmos_launch(permissions_json: String) {
    // Idempotent: a second launch would spawn a second event loop and a
    // second kernel fighting over the same canvas.
    static LAUNCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if LAUNCHED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        log::warn!("pmos_launch called again — ignored");
        return;
    }
    log::info!("launch requested, permissions: {permissions_json}");
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&permissions_json) {
        let mic = v["microphone"].as_bool().unwrap_or(false);
        MIC_GRANTED.with(|m| *m.borrow_mut() = mic);
    }

    let doc = web_sys::window()
        .and_then(|w| w.document())
        .expect("document");
    if let Some(landing) = doc.get_element_by_id("landing") {
        let _ = landing.set_attribute("hidden", "");
    }
    if let Some(os_root) = doc.get_element_by_id("os-root") {
        let _ = os_root.remove_attribute("hidden");
    }

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.spawn_app(OsApp::new());
}


/// Is this egui layer real UI (something a pointer should defer to)?
/// Background-order layers are backdrops/hints in PMOS — never UI.
fn is_ui_layer(l: egui::LayerId) -> bool {
    l.order != egui::Order::Background
}

fn now_secs() -> f64 {
    web_sys::window().unwrap().performance().unwrap().now() / 1000.0
}

// ---------- gesture-worker bridge ----------
// gesture.js delivers landmark frames here; the frame loop drains them into
// the kernel. Only the freshest frame matters (the recognizer is stateful).

thread_local! {
    static HAND_FRAME: RefCell<Option<(Vec<f32>, u32)>> = const { RefCell::new(None) };
    static CAMERA_STATUS: RefCell<Option<(bool, String)>> = const { RefCell::new(None) };
    static CAMERA_PIXELS: RefCell<Option<(Vec<u8>, u32, u32)>> = const { RefCell::new(None) };
    static AI_CHUNKS: RefCell<Vec<(u32, String, bool)>> = const { RefCell::new(Vec::new()) };
    // A queue, not a latest-wins cell: error + end statuses can land in the
    // same frame, and the error reason must not be lost.
    static VOICE_STATUS: RefCell<Vec<(bool, bool, String)>> = const { RefCell::new(Vec::new()) };
    static LLM_TIER: RefCell<Option<u8>> = const { RefCell::new(None) };
    static WEB_RESULTS: RefCell<Vec<(u32, bool, String)>> = const { RefCell::new(Vec::new()) };
    static FACE_FRAME: RefCell<Option<(f32, f32, f32, f32, f32, f32)>> = const { RefCell::new(None) };
    static MIC_GRANTED: RefCell<bool> = const { RefCell::new(false) };
    static VOICE_TRANSCRIPTS: RefCell<Vec<(String, bool)>> = const { RefCell::new(Vec::new()) };
    static VFS_LOADED: RefCell<Vec<(String, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    static VFS_DIRS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static VFS_READY: RefCell<Option<bool>> = const { RefCell::new(None) };
}

/// storage.js boot-load callbacks.
#[wasm_bindgen]
pub fn pmos_vfs_file(path: String, data: Vec<u8>) {
    VFS_LOADED.with(|q| q.borrow_mut().push((path, data)));
}

#[wasm_bindgen]
pub fn pmos_vfs_dir(path: String) {
    VFS_DIRS.with(|q| q.borrow_mut().push(path));
}

#[wasm_bindgen]
pub fn pmos_vfs_ready(ok: bool, _err: String) {
    VFS_READY.with(|q| *q.borrow_mut() = Some(ok));
}

fn call_storage(method: &str, args: &js_sys::Array) {
    let Some(win) = web_sys::window() else { return };
    let Ok(g) = js_sys::Reflect::get(&win, &JsValue::from_str("pmosStorage")) else {
        return;
    };
    if let Ok(f) = js_sys::Reflect::get(&g, &JsValue::from_str(method)) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = js_sys::Reflect::apply(f, &g, args);
        }
    }
}

const AI_CFG_KEY: &str = "pmos.ai.cfg";

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// Execute one kernel-built LLM request via llm.js.
fn dispatch_llm(req: &pmos_kernel::ai::LlmRequest) {
    let Some(win) = web_sys::window() else { return };
    let Ok(g) = js_sys::Reflect::get(&win, &JsValue::from_str("pmosLlm")) else {
        return;
    };
    let Ok(f) = js_sys::Reflect::get(&g, &JsValue::from_str("request")) else {
        return;
    };
    let Some(f) = f.dyn_ref::<js_sys::Function>() else {
        return;
    };
    let headers: std::collections::BTreeMap<&str, &str> = req
        .headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let headers_json = serde_json::to_string(&headers).unwrap_or_else(|_| "{}".into());
    let args = js_sys::Array::of5(
        &JsValue::from_f64(req.agent as f64),
        &JsValue::from_str(&req.url),
        &JsValue::from_str(&headers_json),
        &JsValue::from_str(&req.body),
        &JsValue::from_f64(req.kind as f64),
    );
    let _ = js_sys::Reflect::apply(f, &g, &args);
}

/// Called by gesture.js with `hands * 63` floats (21 landmarks × x,y,z).
#[wasm_bindgen]
pub fn pmos_hands_frame(data: Vec<f32>, hands: u32) {
    HAND_FRAME.with(|f| *f.borrow_mut() = Some((data, hands)));
}

/// Called by gesture.js when the camera pipeline comes up, fails, or is
/// starting; `reason` explains disabled states to the user.
#[wasm_bindgen]
pub fn pmos_camera_status(enabled: bool, reason: Option<String>) {
    CAMERA_STATUS.with(|s| *s.borrow_mut() = Some((enabled, reason.unwrap_or_default())));
}

/// Streamed LLM deltas from llm.js.
#[wasm_bindgen]
pub fn pmos_ai_chunk(agent: u32, delta: String, done: bool) {
    AI_CHUNKS.with(|q| q.borrow_mut().push((agent, delta, done)));
}

/// Called by speech.js when the recognition engine starts, ends, or fails.
#[wasm_bindgen]
pub fn pmos_voice_status(listening: bool, available: bool, reason: String) {
    VOICE_STATUS.with(|s| s.borrow_mut().push((listening, available, reason)));
}

/// Called by speech.js with interim (live) and final transcript text.
#[wasm_bindgen]
pub fn pmos_voice_transcript(text: String, is_final: bool) {
    VOICE_TRANSCRIPTS.with(|q| q.borrow_mut().push((text, is_final)));
}

/// Called by webllm.js after probing RAM + GPU limits: the in-browser LLM
/// tier this machine can handle (0 Fast · 1 Balanced · 2 Quality).
#[wasm_bindgen]
pub fn pmos_llm_tier(tier: u8) {
    LLM_TIER.with(|t| *t.borrow_mut() = Some(tier.min(2)));
}

/// Called by llm.js (pmosWeb) when a WebFetch completes (ABI 1.11).
#[wasm_bindgen]
pub fn pmos_web_result(id: u32, ok: bool, body: String) {
    WEB_RESULTS.with(|q| q.borrow_mut().push((id, ok, body)));
}

/// Called by gesture.js with face signals (M10, opt-in): eyeBlink scores,
/// jawOpen, and the chin landmark (camera-normalized; -1 = no face).
/// Blendshapes + one landmark only — never pixels.
#[wasm_bindgen]
pub fn pmos_face_frame(
    blink_l: f32,
    blink_r: f32,
    jaw: f32,
    brow: f32,
    chin_x: f32,
    chin_y: f32,
) {
    FACE_FRAME.with(|f| *f.borrow_mut() = Some((blink_l, blink_r, jaw, brow, chin_x, chin_y)));
}

/// Preview pixels for the Hand Tracker viewer (RGBA, mirrored). These go
/// straight into an egui texture for the shell — deliberately NEVER through
/// the kernel (Hand Gestures spec §7 privacy boundary).
#[wasm_bindgen]
pub fn pmos_camera_frame(data: Vec<u8>, w: u32, h: u32) {
    CAMERA_PIXELS.with(|p| *p.borrow_mut() = Some((data, w, h)));
}

/// Keep the browser-app iframe in sync with the egui window's content rect
/// (egui points == CSS px, since pixels_per_point == devicePixelRatio here).
fn sync_browser_iframe(view: &Option<(String, [f32; 4])>) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let el = match doc.get_element_by_id("pmos-browser-frame") {
        Some(el) => el,
        None => {
            let el = doc.create_element("iframe").unwrap();
            el.set_id("pmos-browser-frame");
            let _ = el.set_attribute(
                "style",
                "position:absolute;border:0;z-index:3;display:none;background:#fff;border-radius:0 0 10px 10px;",
            );
            let _ = el.set_attribute("sandbox", "allow-scripts allow-same-origin allow-forms");
            if let Some(root) = doc.get_element_by_id("os-root") {
                let _ = root.append_child(&el);
            }
            el
        }
    };
    match view {
        Some((url, rect)) => {
            if el.get_attribute("src").as_deref() != Some(url.as_str()) {
                let _ = el.set_attribute("src", url);
            }
            let style = format!(
                "position:absolute;border:0;z-index:3;background:#fff;border-radius:0 0 10px 10px;left:{}px;top:{}px;width:{}px;height:{}px;",
                rect[0], rect[1], rect[2].max(0.0), rect[3].max(0.0)
            );
            let _ = el.set_attribute("style", &style);
        }
        None => {
            let _ = el.set_attribute(
                "style",
                "position:absolute;border:0;z-index:3;display:none;",
            );
        }
    }
}

/// Write text to the system clipboard via navigator.clipboard (async,
/// fire-and-forget — a rejection just means the browser withheld permission).
fn copy_to_clipboard(text: &str) {
    let Some(win) = web_sys::window() else { return };
    let nav: JsValue = win.navigator().into();
    let Ok(clip) = js_sys::Reflect::get(&nav, &JsValue::from_str("clipboard")) else {
        return;
    };
    if clip.is_undefined() {
        return;
    }
    let Ok(f) = js_sys::Reflect::get(&clip, &JsValue::from_str("writeText")) else {
        return;
    };
    if let Some(f) = f.dyn_ref::<js_sys::Function>() {
        let _ = f.call1(&clip, &JsValue::from_str(text));
    }
}

/// Push voice preferences (Whisper model size) from /settings/voice.json to
/// speech.js. Called after VFS boot and whenever that file is rewritten.
fn apply_voice_config(kernel: &pmos_kernel::Kernel) {
    let Some(bytes) = kernel.vfs.read("/settings/voice.json") else {
        return;
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    let Some(size) = v.get("whisper").and_then(|s| s.as_str()) else {
        return;
    };
    let Some(win) = web_sys::window() else { return };
    let Ok(g) = js_sys::Reflect::get(&win, &JsValue::from_str("pmosVoice")) else {
        return;
    };
    if let Ok(f) = js_sys::Reflect::get(&g, &JsValue::from_str("configure")) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let opts = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &opts,
                &JsValue::from_str("whisper"),
                &JsValue::from_str(size),
            );
            let _ = f.call1(&g, &opts);
        }
    }
}

/// Dispatch one WebFetch to llm.js's pmosWeb bridge.
fn dispatch_web_fetch(id: u32, url: &str) {
    let Some(win) = web_sys::window() else { return };
    let Ok(g) = js_sys::Reflect::get(&win, &JsValue::from_str("pmosWeb")) else {
        return;
    };
    if let Ok(f) = js_sys::Reflect::get(&g, &JsValue::from_str("fetch")) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = f.call2(&g, &JsValue::from_f64(id as f64), &JsValue::from_str(url));
        }
    }
}

/// Push face-tracking preferences (/settings/face.json) to gesture.js —
/// the face model only runs when the user opts in (Settings → Face).
fn apply_face_config(kernel: &pmos_kernel::Kernel) {
    let enabled = kernel
        .vfs
        .read("/settings/face.json")
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("enabled").and_then(|e| e.as_bool()))
        .unwrap_or(false);
    let Some(win) = web_sys::window() else { return };
    let Ok(g) = js_sys::Reflect::get(&win, &JsValue::from_str("pmosGestures")) else {
        return;
    };
    if g.is_undefined() {
        return;
    }
    if let Ok(f) = js_sys::Reflect::get(&g, &JsValue::from_str("configure")) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let opts = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &opts,
                &JsValue::from_str("face"),
                &JsValue::from_bool(enabled),
            );
            let _ = f.call1(&g, &opts);
        }
    }
}

/// Apply the kernel voice directive to speech.js (start/stop capture).
fn apply_voice_directive(capture: bool) {
    let Some(win) = web_sys::window() else { return };
    let Ok(g) = js_sys::Reflect::get(&win, &JsValue::from_str("pmosVoice")) else {
        return;
    };
    let method = if capture { "start" } else { "stop" };
    if let Ok(f) = js_sys::Reflect::get(&g, &JsValue::from_str(method)) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = f.call0(&g);
        }
    }
}

/// Apply kernel hands-directives to the JS pipeline (configure + start).
fn apply_hands_directives(d: &pmos_kernel::HandsDirectives, camera_start: bool) {
    let Some(win) = web_sys::window() else { return };
    let Ok(g) = js_sys::Reflect::get(&win, &JsValue::from_str("pmosGestures")) else {
        return;
    };
    if g.is_undefined() {
        return;
    }
    let opts = js_sys::Object::new();
    let set = |k: &str, v: JsValue| {
        let _ = js_sys::Reflect::set(&opts, &JsValue::from_str(k), &v);
    };
    set("streamFeed", JsValue::from_bool(d.stream_feed));
    set("numHands", JsValue::from_f64(d.tuning.num_hands as f64));
    set("detConf", JsValue::from_f64(d.tuning.det_conf as f64));
    set("trackConf", JsValue::from_f64(d.tuning.track_conf as f64));
    if let Ok(f) = js_sys::Reflect::get(&g, &JsValue::from_str("configure")) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = f.call1(&g, &opts);
        }
    }
    if camera_start {
        if let Ok(f) = js_sys::Reflect::get(&g, &JsValue::from_str("start")) {
            if let Some(f) = f.dyn_ref::<js_sys::Function>() {
                let _ = f.call0(&g);
            }
        }
    }
}

struct OsApp {
    window: Option<Arc<Window>>,
    canvas: Option<web_sys::HtmlCanvasElement>,
    /// Filled by the async wgpu init task, drained on the next frame.
    pending_gfx: Rc<RefCell<Option<Gfx>>>,
    kernel: Kernel,
    shell: Option<Shell>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    boot_time: f64,
    /// Hand Tracker preview texture (pixels never touch the kernel).
    camera_tex: Option<egui::TextureHandle>,
    applied_hands_generation: u32,
    applied_voice_generation: u32,
    last_frame_time: f64,
    frame_dt: f32,
    // hand-grab routing (M4/M7): what the closed hand is holding
    hand_grab_mode: GrabMode,
    hand_grab_last: [f32; 2],
    // camera interaction
    mouse_mode: GrabMode,
    last_cursor: Option<(f32, f32)>,
    last_press: f64,
    /// Left button held — the mouse owns the egui pointer while true, so
    /// hand-tracking Move events must not hijack drags or text selection.
    mouse_left_down: bool,
    /// Shift held (winit modifiers) — shift+drag pans the stage camera.
    shift_down: bool,
    /// Last mouse CursorMoved time — the hand yields while the mouse is live.
    last_mouse_move: f64,
    /// 👌 pinch routing: Ui click/drag, or Prop (object grabbed by pinch).
    pinch_mode: GrabMode,
    /// (press time, press pos, grabbed prop index) — a quick, still pinch on
    /// an object is a SELECT: it becomes the focused object (CSL §5).
    pinch_press: Option<(f64, [f32; 2], Option<usize>)>,
    /// One-shot: auto-start voice capture once the shell exists.
    voice_autostarted: bool,
    /// Last hand cursor position forwarded to egui (sub-point jitter from a
    /// resting hand would otherwise fight the mouse for the pointer).
    last_hand_move: Option<[f32; 2]>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum GrabMode {
    None,
    Ui,
    Prop,
    Orbit,
    /// Camera pan (shift+drag or middle-drag over empty stage).
    Pan,
}

impl OsApp {

    /// Viewport in egui points. NEVER window.inner_size() — on web that can
    /// report 0×0 in event handlers (the canvas is CSS-sized), which fed a
    /// zero viewport into ray picking and silently broke every prop grab
    /// since M7 (user-reported). The canvas client size is the truth.
    fn viewport_pts(&self) -> [f32; 2] {
        self.canvas
            .as_ref()
            .map(|c| [c.client_width() as f32, c.client_height() as f32])
            .filter(|v| v[0] > 0.0 && v[1] > 0.0)
            .unwrap_or([1.0, 1.0])
    }
    fn new() -> Self {
        let mut kernel = Kernel::new();
        if let Some(json) = local_storage().and_then(|s| s.get_item(AI_CFG_KEY).ok().flatten()) {
            if let Ok(cfg) = serde_json::from_str::<pmos_abi::AiProviderConfig>(&json) {
                kernel.ai.set_config(cfg, false);
            }
        }
        Self {
            window: None,
            canvas: None,
            pending_gfx: Rc::new(RefCell::new(None)),
            kernel,
            shell: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            boot_time: now_secs(),
            camera_tex: None,
            applied_hands_generation: 0,
            applied_voice_generation: 0,
            last_frame_time: now_secs(),
            frame_dt: 1.0 / 60.0,
            hand_grab_mode: GrabMode::None,
            hand_grab_last: [0.0, 0.0],
            mouse_mode: GrabMode::None,
            last_cursor: None,
            last_press: 0.0,
            mouse_left_down: false,
            shift_down: false,
            last_mouse_move: -10.0,
            pinch_mode: GrabMode::None,
            pinch_press: None,
            voice_autostarted: false,
            last_hand_move: None,
        }
    }

    fn frame(&mut self) {
        if let Some(gfx) = self.pending_gfx.borrow_mut().take() {
            self.kernel.install_gfx(gfx);
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Some(el) = doc.get_element_by_id("boot-status") {
                    let _ = el.set_attribute("hidden", "");
                }
            }
            log::info!("gfx online — render graph live");
            call_storage("loadAll", &js_sys::Array::new());
        }
        let viewport = self.viewport_pts();
        let (Some(window), Some(egui_state)) = (&self.window, &mut self.egui_state) else {
            return;
        };
        if self.kernel.gfx.is_none() {
            return;
        }
        // The canvas is CSS-sized; keep winit and the surface in sync with its
        // layout size every frame (initial Resized events can predate gfx).
        if let Some(canvas) = &self.canvas {
            let dpr = web_sys::window().unwrap().device_pixel_ratio();
            let (cw, ch) = (
                (canvas.client_width() as f64 * dpr) as u32,
                (canvas.client_height() as f64 * dpr) as u32,
            );
            if cw > 0 && ch > 0 {
                let inner = window.inner_size();
                if (cw, ch) != (inner.width, inner.height) {
                    let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(cw, ch));
                }
                if let Some(gfx) = self.kernel.gfx.as_mut() {
                    gfx.resize(cw, ch);
                }
            }
        }
        if self.shell.is_none() {
            self.shell = Some(Shell::new(&mut self.kernel));
        }

        // Drain the gesture bridge into the kernel input pipeline.
        let now = now_secs();
        if let Some((enabled, reason)) = CAMERA_STATUS.with(|s| s.borrow_mut().take()) {
            self.kernel.set_camera_status(enabled, reason);
        }
        if let Some((data, hands)) = HAND_FRAME.with(|f| f.borrow_mut().take()) {
            self.kernel.hand_frame(&data, hands, viewport, now);
        }
        self.kernel.tick_hands(now);

        // Hover pick (first-class objects): whichever pointer is live — the
        // hand while tracking, else the mouse — lights up the prop under it,
        // unless it's over UI.
        {
            let ppp = self.egui_ctx.pixels_per_point();
            let pointer = if self.kernel.input.hands.tracking {
                self.kernel.input.hands.cursor
            } else {
                self.last_cursor.map(|(x, y)| [x / ppp, y / ppp])
            };
            let pointer = pointer.filter(|p| {
                !self
                    .egui_ctx
                    .layer_id_at(egui::pos2(p[0], p[1]))
                    .is_some_and(is_ui_layer)
            });
            self.kernel.update_hover(pointer, viewport);
        }

        // Voice plumbing: engine status + transcripts in, capture intent out.
        // Transcripts BEFORE statuses: the engine emits the final transcript
        // then the session-end status, and the shell must see them in order.
        for (text, is_final) in VOICE_TRANSCRIPTS.with(|q| std::mem::take(&mut *q.borrow_mut())) {
            self.kernel.voice_transcript(text, is_final);
        }
        for (listening, available, reason) in
            VOICE_STATUS.with(|s| std::mem::take(&mut *s.borrow_mut()))
        {
            self.kernel.voice_status(listening, available, reason);
        }
        if self.kernel.voice_directives.generation != self.applied_voice_generation {
            self.applied_voice_generation = self.kernel.voice_directives.generation;
            apply_voice_directive(self.kernel.voice_directives.capture);
        }
        // Always-on voice (Voice Kit spec §2): if the mic was granted at
        // onboarding, capture starts at boot — the RECORD sign pauses it.
        if !self.voice_autostarted && self.shell.is_some() {
            self.voice_autostarted = true;
            if MIC_GRANTED.with(|m| *m.borrow()) {
                self.kernel.voice_directives.capture = true;
                self.kernel.voice_directives.generation += 1;
                log::info!("voice capture auto-started (mic granted at onboarding)");
            }
        }
        // Web fetches for the AI tools (ABI 1.11).
        for (id, url) in std::mem::take(&mut self.kernel.web_pending) {
            dispatch_web_fetch(id, &url);
        }
        for (id, ok, body) in WEB_RESULTS.with(|q| std::mem::take(&mut *q.borrow_mut())) {
            self.kernel.web_result(id, ok, body);
        }
        // Face layer (M10, opt-in): blendshapes → kernel; double-blink
        // injects a click at the current pointer (accessibility).
        if let Some((bl, br, jaw, brow, cx, cy)) = FACE_FRAME.with(|f| f.borrow_mut().take()) {
            self.kernel.face_frame(bl, br, jaw, brow, cx, cy, now);
        }
        // Machine-probed LLM tier: surface via /sys/llm_tier, and — when the
        // user never saved an AI config — fit the default model to the tier.
        if let Some(tier) = LLM_TIER.with(|t| t.borrow_mut().take()) {
            self.kernel.vfs.sys_llm_tier = tier;
            let user_saved = local_storage()
                .and_then(|s| s.get_item(AI_CFG_KEY).ok().flatten())
                .is_some();
            if !user_saved {
                if let Some(cfg) = &mut self.kernel.ai.config {
                    if cfg.kind == 2 {
                        const TIER_MODELS: [&str; 3] = [
                            "Qwen2.5-0.5B-Instruct-q4f16_1-MLC",
                            "Llama-3.2-1B-Instruct-q4f16_1-MLC",
                            "Qwen2.5-3B-Instruct-q4f16_1-MLC",
                        ];
                        cfg.model = TIER_MODELS[tier as usize].to_string();
                        log::info!("default in-browser model fit to tier {tier}: {}", cfg.model);
                    }
                }
            }
        }

        // VFS plumbing: ingest boot-loaded state, then mirror dirty ops out.
        for dir in VFS_DIRS.with(|q| std::mem::take(&mut *q.borrow_mut())) {
            let _ = self.kernel.vfs.mkdir(&dir);
            self.kernel.vfs.dirty.clear(); // boot mkdirs need no re-persist
        }
        for (path, data) in VFS_LOADED.with(|q| std::mem::take(&mut *q.borrow_mut())) {
            self.kernel.vfs.load(&path, data);
        }
        if let Some(ok) = VFS_READY.with(|q| q.borrow_mut().take()) {
            self.kernel.vfs.ready = true;
            log::info!("vfs ready (persistent: {ok})");
            apply_voice_config(&self.kernel);
            apply_face_config(&self.kernel);
        }
        for op in std::mem::take(&mut self.kernel.vfs.dirty) {
            use pmos_kernel::vfs::VfsOp;
            match op {
                VfsOp::Write(path, bytes) => {
                    if path == "/settings/voice.json" {
                        apply_voice_config(&self.kernel);
                    }
                    if path == "/settings/face.json" {
                        apply_face_config(&self.kernel);
                    }
                    let arr = js_sys::Array::of2(
                        &JsValue::from_str(&path),
                        &js_sys::Uint8Array::from(bytes.as_slice()).into(),
                    );
                    call_storage("write", &arr);
                }
                VfsOp::Delete(path) => {
                    call_storage("remove", &js_sys::Array::of1(&JsValue::from_str(&path)));
                }
                VfsOp::Mkdir(path) => {
                    call_storage("mkdir", &js_sys::Array::of1(&JsValue::from_str(&path)));
                }
            }
        }
        // Live frame rate for /sys/fps (EMA).
        let dt = (now - self.last_frame_time).max(1e-4);
        self.last_frame_time = now;
        self.frame_dt = dt as f32;
        let fps = 1.0 / dt;
        self.kernel.vfs.sys_fps = if self.kernel.vfs.sys_fps == 0.0 {
            fps as f32
        } else {
            self.kernel.vfs.sys_fps * 0.95 + fps as f32 * 0.05
        };

        // AI plumbing: deliver streamed chunks, dispatch queued requests,
        // persist a changed provider config (key stays inside the kernel;
        // localStorage persistence is a documented v1 convenience).
        for (agent, delta, done) in AI_CHUNKS.with(|q| std::mem::take(&mut *q.borrow_mut())) {
            self.kernel.ai_chunk(agent, delta, done);
        }
        for req in std::mem::take(&mut self.kernel.ai.pending) {
            dispatch_llm(&req);
        }
        if self.kernel.ai.config_dirty {
            self.kernel.ai.config_dirty = false;
            if let (Some(cfg), Some(store)) = (&self.kernel.ai.config, local_storage()) {
                if let Ok(json) = serde_json::to_string(cfg) {
                    let _ = store.set_item(AI_CFG_KEY, &json);
                }
            }
        }

        // Apply changed hands directives to the JS pipeline (exactly once
        // per generation), then upload any fresh preview pixels.
        if self.kernel.hands_directives.generation != self.applied_hands_generation {
            self.applied_hands_generation = self.kernel.hands_directives.generation;
            let start = self.kernel.hands_directives.camera_start;
            self.kernel.hands_directives.camera_start = false;
            apply_hands_directives(&self.kernel.hands_directives, start);
        }
        if let Some((data, w, h)) = CAMERA_PIXELS.with(|p| p.borrow_mut().take()) {
            let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &data);
            match &mut self.camera_tex {
                Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                None => {
                    self.camera_tex = Some(self.egui_ctx.load_texture(
                        "camera-preview",
                        img,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
        }
        let camera_feed = self.camera_tex.as_ref().map(|t| t.id());
        let rt_tex = self.kernel.gfx.as_mut().map(|g| g.rt_texture());

        let time = (now_secs() - self.boot_time) as f32;
        let mut raw_input = egui_state.take_egui_input(window);

        // Hand pointer intents → synthetic egui pointer events (M4). Grab is
        // routed by context: over UI it drags like a primary press (windows
        // move by titlebar); over empty stage it orbits the camera.
        if self.kernel.face_click_pending {
            self.kernel.face_click_pending = false;
            if let Some(pos) = self.egui_ctx.pointer_latest_pos() {
                raw_input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                });
                raw_input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                });
            }
        }
        use pmos_kernel::input::fusion::HandIntent;
        for intent in self.kernel.input.fusion.take_intents() {
            let ev = &mut raw_input.events;
            let p = |xy: [f32; 2]| egui::pos2(xy[0], xy[1]);
            match intent {
                HandIntent::Move(pos) => {
                    // Mouse priority: while the mouse button is down OR the
                    // mouse moved recently, the hand yields the pointer
                    // entirely (user request: no fighting cursors). Sub-point
                    // hand jitter is deduplicated too.
                    let moved = self
                        .last_hand_move
                        .map(|l| (pos[0] - l[0]).abs() + (pos[1] - l[1]).abs() > 0.5)
                        .unwrap_or(true);
                    let mouse_active = self.mouse_left_down
                        || now_secs() - self.last_mouse_move < 1.0;
                    if self.pinch_mode == GrabMode::Prop {
                        // Pinch-dragging an object: the hand steers physics.
                        self.kernel.move_grab(pos, viewport);
                    } else if !mouse_active && moved {
                        self.last_hand_move = Some(pos);
                        ev.push(egui::Event::PointerMoved(p(pos)));
                    }
                }
                HandIntent::Press { pos, secondary } => {
                    // 👌 pinch = the hand's click AND its object grab (user
                    // decision 2026-08-02, matching CSL §5 pinch-select):
                    // over UI it clicks; over a stage object it grabs it.
                    let on_ui = self
                        .egui_ctx
                        .layer_id_at(p(pos))
                        .is_some_and(is_ui_layer);
                    if !secondary && !on_ui && self.kernel.try_grab_prop(pos, viewport) {
                        self.pinch_mode = GrabMode::Prop;
                        self.pinch_press =
                            Some((now_secs(), pos, self.kernel.grabbed_prop_index()));
                    } else {
                        if !on_ui {
                            // Pinch on empty stage clears object focus.
                            self.kernel.phys.focused = None;
                        }
                        self.pinch_mode = GrabMode::Ui;
                        ev.push(egui::Event::PointerMoved(p(pos)));
                        ev.push(egui::Event::PointerButton {
                            pos: p(pos),
                            button: if secondary {
                                egui::PointerButton::Secondary
                            } else {
                                egui::PointerButton::Primary
                            },
                            pressed: true,
                            modifiers: egui::Modifiers::default(),
                        });
                    }
                }
                HandIntent::Release { pos, secondary } => {
                    if self.pinch_mode == GrabMode::Prop {
                        self.kernel.release_grab(); // velocity carries: throw
                        // Quick + still = SELECT: focus the object.
                        if let Some((t0, p0, idx)) = self.pinch_press.take() {
                            let moved =
                                (pos[0] - p0[0]).abs() + (pos[1] - p0[1]).abs();
                            if now_secs() - t0 < 0.35 && moved < 18.0 {
                                self.kernel.phys.focused = idx;
                            }
                        }
                    } else {
                        ev.push(egui::Event::PointerMoved(p(pos)));
                        ev.push(egui::Event::PointerButton {
                            pos: p(pos),
                            button: if secondary {
                                egui::PointerButton::Secondary
                            } else {
                                egui::PointerButton::Primary
                            },
                            pressed: false,
                            modifiers: egui::Modifiers::default(),
                        });
                    }
                    self.pinch_mode = GrabMode::None;
                }
                HandIntent::Scroll { pos, dy }
                    if !self
                        .egui_ctx
                        .layer_id_at(egui::pos2(pos[0], pos[1]))
                        .is_some_and(is_ui_layer) =>
                {
                    // ✌ over the empty stage zooms the camera; over UI it
                    // scrolls (modality parity with the mouse wheel).
                    if let Some(gfx) = self.kernel.gfx.as_mut() {
                        gfx.camera.zoom(dy * 0.06);
                    }
                }
                HandIntent::Scroll { pos, dy } => {
                    ev.push(egui::Event::PointerMoved(p(pos)));
                    // Natural scrolling: hand moves down → content moves down.
                    ev.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, dy * 2.0),
                        phase: egui::TouchPhase::Move,
                        modifiers: egui::Modifiers::default(),
                    });
                }
                HandIntent::GrabStart(pos) => {
                    // ✊ owns the SPACE, 👌 owns the OBJECTS (user decision
                    // 2026-08-02): a fist over UI drags the window, anywhere
                    // else it orbits the camera. Objects are pinch-grabbed.
                    if self.egui_ctx.layer_id_at(p(pos)).is_some_and(is_ui_layer) {
                        self.hand_grab_mode = GrabMode::Ui;
                        ev.push(egui::Event::PointerMoved(p(pos)));
                        ev.push(egui::Event::PointerButton {
                            pos: p(pos),
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::default(),
                        });
                    } else {
                        self.hand_grab_mode = GrabMode::Orbit;
                    }
                    self.hand_grab_last = pos;
                }
                HandIntent::GrabMove(pos) => {
                    match self.hand_grab_mode {
                        GrabMode::Ui => ev.push(egui::Event::PointerMoved(p(pos))),
                        GrabMode::Prop => self.kernel.move_grab(pos, viewport),
                        GrabMode::Orbit => {
                            if let Some(gfx) = self.kernel.gfx.as_mut() {
                                let dpr = window.scale_factor() as f32;
                                gfx.camera.orbit(
                                    (pos[0] - self.hand_grab_last[0]) * dpr,
                                    (pos[1] - self.hand_grab_last[1]) * dpr,
                                );
                            }
                        }
                        GrabMode::None | GrabMode::Pan => {}
                    }
                    self.hand_grab_last = pos;
                }
                HandIntent::GrabEnd(pos) => {
                    match self.hand_grab_mode {
                        GrabMode::Ui => ev.push(egui::Event::PointerButton {
                            pos: p(pos),
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::default(),
                        }),
                        GrabMode::Prop => self.kernel.release_grab(),
                        _ => {}
                    }
                    self.hand_grab_mode = GrabMode::None;
                }
            }
        }

        // Disjoint field borrows: shell and kernel are separate fields.
        let shell = self.shell.as_mut().unwrap();
        let kernel = &mut self.kernel;
        let today = js_sys::Date::new_0()
            .to_iso_string()
            .as_string()
            .unwrap_or_default();
        let today = today.get(..10).unwrap_or("today").to_string();
        self.egui_ctx.begin_pass(raw_input);
        shell.update(&self.egui_ctx, kernel, camera_feed, rt_tex, &today);
        let output = self.egui_ctx.end_pass();
        sync_browser_iframe(&self.shell.as_ref().unwrap().browser_view);

        // Mirror egui copies to the real browser clipboard — egui-winit's
        // wasm clipboard is internal-only, so Ctrl+C would otherwise never
        // leave the canvas (UI spec §3: text is selectable everywhere).
        for cmd in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = cmd {
                if !text.is_empty() {
                    copy_to_clipboard(text);
                }
            }
        }
        egui_state.handle_platform_output(window, output.platform_output);
        let primitives = self
            .egui_ctx
            .tessellate(output.shapes, output.pixels_per_point);
        self.kernel.render_frame(
            &primitives,
            &output.textures_delta,
            output.pixels_per_point,
            time,
            self.frame_dt,
        );
    }
}

impl ApplicationHandler for OsApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("stage"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("#stage canvas");

        let attrs = Window::default_attributes().with_canvas(Some(canvas.clone()));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        self.canvas = Some(canvas);
        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        ));
        self.window = Some(window.clone());

        // wgpu init is async on the web; hand the result back through a cell.
        let pending = self.pending_gfx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::BROWSER_WEBGPU,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let surface = instance.create_surface(window.clone()).expect("surface");
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&surface),
                    ..Default::default()
                })
                .await
                .expect("adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .expect("device");
            let size = window.inner_size();
            let mut config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .expect("surface config");
            config.present_mode = wgpu::PresentMode::Fifo;
            surface.configure(&device, &config);
            *pending.borrow_mut() = Some(Gfx::new(device, queue, surface, config));
            window.request_redraw();
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let (Some(window), Some(egui_state)) = (&self.window, &mut self.egui_state) else {
            return;
        };
        let egui_wants = egui_state.on_window_event(window, &event).consumed;

        match event {
            WindowEvent::RedrawRequested => self.frame(),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.kernel.gfx.as_mut() {
                    gfx.resize(size.width, size.height);
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            // Stage camera controls. Routing hit-tests the MOUSE's own
            // position (layer_id_at) instead of egui's shared pointer state:
            // the hand cursor also drives that pointer, and a tracked hand
            // hovering any UI (or holding a pinch) used to make egui consume
            // every mouse press — stage controls went dead (user-reported).
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.mouse_left_down = state == ElementState::Pressed;
                let ppp = self.egui_ctx.pixels_per_point();
                let pos_pts = self
                    .last_cursor
                    .map(|(x, y)| [x / ppp, y / ppp])
                    .unwrap_or([0.0, 0.0]);
                let on_ui = self
                    .egui_ctx
                    .layer_id_at(egui::pos2(pos_pts[0], pos_pts[1]))
                    .is_some_and(is_ui_layer);
                if state == ElementState::Pressed && !on_ui {
                    let t = now_secs();
                    if t - self.last_press < 0.35 {
                        if let Some(gfx) = self.kernel.gfx.as_mut() {
                            gfx.camera.reset(); // double-click empty space
                        }
                    }
                    self.last_press = t;
                    // Modality parity: the mouse can grab props too (UI §3.2).
                    let viewport = self.viewport_pts();
                    self.mouse_mode = if self.shift_down {
                        GrabMode::Pan
                    } else if self.kernel.try_grab_prop(pos_pts, viewport) {
                        GrabMode::Prop
                    } else {
                        GrabMode::Orbit
                    };
                    log::info!(
                        "mouse press at {pos_pts:?} vp {viewport:?} → {:?}",
                        self.mouse_mode
                    );
                } else if state == ElementState::Pressed {
                    let layer = self
                        .egui_ctx
                        .layer_id_at(egui::pos2(pos_pts[0], pos_pts[1]))
                        .map(|l| l.short_debug_format());
                    log::info!("mouse press at {pos_pts:?} on UI layer {layer:?}");
                } else if state == ElementState::Released {
                    if self.mouse_mode == GrabMode::Prop {
                        self.kernel.release_grab();
                    }
                    self.mouse_mode = GrabMode::None;
                }
            }
            // Middle-drag pans the stage camera.
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Middle,
                ..
            } => {
                let on_ui = self
                    .last_cursor
                    .map(|(x, y)| {
                        let ppp = self.egui_ctx.pixels_per_point();
                        self.egui_ctx
                            .layer_id_at(egui::pos2(x / ppp, y / ppp))
                            .is_some_and(is_ui_layer)
                    })
                    .unwrap_or(false);
                self.mouse_mode = if state == ElementState::Pressed && !on_ui {
                    GrabMode::Pan
                } else {
                    GrabMode::None
                };
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.shift_down = mods.state().shift_key();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let pos = (position.x as f32, position.y as f32);
                match self.mouse_mode {
                    // Once an orbit began on empty stage it follows until
                    // release — crossing over a window must not stall it.
                    GrabMode::Orbit => {
                        if let (Some(last), Some(gfx)) =
                            (self.last_cursor, self.kernel.gfx.as_mut())
                        {
                            gfx.camera.orbit(pos.0 - last.0, pos.1 - last.1);
                        }
                    }
                    GrabMode::Pan => {
                        if let (Some(last), Some(gfx)) =
                            (self.last_cursor, self.kernel.gfx.as_mut())
                        {
                            gfx.camera.pan(pos.0 - last.0, pos.1 - last.1);
                        }
                    }
                    GrabMode::Prop => {
                        let ppp = self.egui_ctx.pixels_per_point();
                        let viewport = self.viewport_pts();
                        self.kernel.move_grab([pos.0 / ppp, pos.1 / ppp], viewport);
                    }
                    _ => {}
                }
                self.last_cursor = Some(pos);
                self.last_mouse_move = now_secs();
                self.kernel
                    .input
                    .pointer_moved([pos.0, pos.1], pmos_abi::InputSource::Mouse);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let on_ui = self
                    .last_cursor
                    .map(|(x, y)| {
                        let ppp = self.egui_ctx.pixels_per_point();
                        self.egui_ctx
                            .layer_id_at(egui::pos2(x / ppp, y / ppp))
                            .is_some_and(is_ui_layer)
                    })
                    .unwrap_or(false);
                log::info!("wheel: on_ui={on_ui} cursor={:?}", self.last_cursor);
                if !on_ui {
                    let dy = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                    };
                    if let Some(gfx) = self.kernel.gfx.as_mut() {
                        gfx.camera.zoom(dy);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                use winit::keyboard::{Key, NamedKey};
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Home)
                {
                    if let Some(gfx) = self.kernel.gfx.as_mut() {
                        gfx.camera.reset();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Continuous rendering: the stage is alive (galaxy drift, icon bob).
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
