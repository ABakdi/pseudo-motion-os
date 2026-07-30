# Todo — Roadmap
**Pseudo Motion OS** · part of [[Pseudo Motion OS]]

Milestones → tasks → subtasks. `[x]` = done, `[~]` = in progress, `[ ]` = pending. Every completed item gets its [[Changelog]] entry. Specs referenced per task.

---

## M0 — Project Setup ✅
*Goal: a compiling Rust workspace that builds natively and for wasm32, with the dev tooling ready.*

- [x] **Cargo workspace** — six crates per [[Architecture#10. Crate Layout]], shared deps pinned once at workspace level.
    - [x] `pmos-abi` — syscall/event/capability types, `ABI_VERSION`, all serde-serializable (the future process-isolation discipline).
    - [x] `pmos-kernel` — `Kernel` root object + subsystem module stubs (`gfx`, `input`, `proc`, `vfs`), each doc-commented with its spec section and landing milestone.
    - [x] `pmos-platform` — the only `web-sys` importer; `webgpu_available()` boot check implemented (via `Reflect`, avoiding unstable-API cfg).
    - [x] `pmos-apps` — userland skeleton; crate graph physically prevents it from touching kernel internals (depends on `pmos-abi` only).
    - [x] `pmos-conjure` — validator skeleton with machine-readable `ValidationError {path, code, message, hint}` + version gate + 3 passing tests.
    - [x] `pmos-web` — trunk entry point; placeholder page proves WASM boots and the kernel constructs.
- [x] **Toolchain** — `rust-toolchain.toml` (stable + wasm32 target), wasm32 target installed.
- [x] **trunk** installed (prebuilt binary — `cargo install` fails locally on `libdeflate-sys` C build); `trunk build` produces `dist/` successfully.
- [x] **Verification** — `cargo check --workspace` (native + wasm32) green; `cargo test` green.
- [x] **README** with dev instructions and crate map.

## M1 — Landing & Launch Experience ✅
*Goal: visitor opens the URL → impressive home page with logo → presses Launch → grants permissions → arrives on the desktop. The landing page is plain HTML/CSS (instant paint, works even without WebGPU); WASM boots on Launch.*

- [x] **Logo & identity** — PMOS logomark (animated SVG: core dot with two counter-orbiting gradient arcs), gradient wordmark, design tokens as CSS variables (to be mirrored in the egui theme at M2). *(Spec: [[UI#2.0 Boot & Launch experience]])*
- [x] **Home page** — full-viewport hero: canvas starfield (3 depth layers, twinkle + parallax drift, reduced-motion aware) + animated nebula glows, logo, tagline, **LAUNCH** CTA (enabled when the core loads, glow on hover); three feature cards; footer; "browser not supported" notice replaces the CTA when WebGPU is absent.
- [x] **Launch flow** — Launch click → onboarding → fade → `pmos_launch(permissions)` boots the kernel:
    - [x] Sequential permission cards (camera 📷 / microphone 🎤 / notifications 🔔) with reason lines, `Enable`/`Later`, progress dots; browser prompts fire from the explicit Enable click.
    - [x] Permission state persisted (localStorage for now — migrates to `/sys/permissions` when the VFS lands in M6); returning users skip onboarding entirely. *(verified in Chrome)*
- [x] **Transition** — landing fades out (0.9 s) into the OS root; kernel constructs and reports ABI v1.0 (real desktop arrives in M2). *(verified end-to-end in Chrome, including console logs)*

## M2 — Desktop: 3D Space, Galaxy, App Icons ✅
*Goal: the OS desktop — a 3D space wrapped in a distant galaxy that can never be reached, floating app icons that open windows, full mouse/keyboard control.*

- [x] **wgpu foundation** — device/queue/surface on the canvas (async init handed back through a cell), render graph (sky → floor → egui overlay), per-frame canvas/surface size sync (CSS-sized canvas; initial Resized events can predate gfx — learned the hard way: a stale tiny surface renders as one uniform stretched color). Frame timing to `/sys/fps` deferred to M6 with the `/sys` tree.
- [x] **egui integration** — `egui-wgpu` 0.35 renderer as the overlay pass; PMOS dark theme tokens applied ([[UI#6]]).
- [x] **Galaxy skybox** — WGSL fullscreen pass: 3-layer hashed starfield with twinkle, two drifting fbm nebulae (ion blue / violet), tilted milky band; driven by a **rotation-only** inverse view-projection so zoom/translation never changes parallax — unreachable by construction. *(verified in Chrome)*
- [x] **Stage camera** — orbit (drag), zoom (wheel, clamped 5–26 units), pitch clamps, `Home` + double-click reset. *(verified)*
- [x] ~~**Floating app icons**~~ — shipped, then **removed by user decision (2026-07-30)**: redundant with the dock; the Stage stays reserved for content, not chrome ([[UI#2.8 App launching surfaces]]). *(Implementation note kept for posterity: 🗂 is missing from egui's emoji font — use glyphs that render, e.g. 📁.)*
- [x] **Window manager v1** — egui windows (drag/resize/collapse/close), focus, bottom dock mirroring the apps with running dots. Built-in app stubs: Terminal (help/about/clear), Files, Notes scratchpad, Settings, Browser.
- [x] **Kernel ABI v1 live** — `KernelApi` trait in `pmos-abi`; dispatcher with per-syscall capability checks; process table (shell = Pid 1 with shell grant, apps get minimal default); each opened app registers as a real process and opens its window via `ProcRegister`/`WinCreate` syscalls. *(verified: console shows Terminal=Pid(2), Settings=Pid(3))*
- [x] **Keyboard & mouse** — winit events → egui first, unconsumed input drives the camera; pointer moves flow through the kernel input pipeline with source tags (fusion foundation for M3).
- *Known quirk for M4:* releasing a camera-orbit drag over an icon can register as a click — proper press-origin routing arrives with the M4 input work.

## M3 — Camera & Hand Gesture Detection ✅
*Goal: webcam on, hands tracked, gestures recognized — and a living cursor that morphs with the hand.*

- [x] **Gesture pipeline** — video capture on the main thread (getUserMedia is worker-unavailable — spec corrected), ImageBitmap transfer to the JS gesture worker running MediaPipe `tasks-vision` HandLandmarker (GPU delegate, 2 hands, camera-paced via `requestVideoFrameCallback`); landmarks → WASM bridge → kernel. Only landmarks cross into the kernel. *(Spec: [[Architecture#3]], [[Hand Gestures#2]])*
- [x] **Landmark ingestion** — One-Euro filtering (Balanced preset), control-box mapping with x-mirror, tracking-loss timeout (0.5 s → cursor freezes, never jumps).
- [x] **Pose classifier** — rule-based on landmark topology: Point, Pinch, MiddlePinch, Grab, OpenPalm, TwoFinger, CallSign, ThumbsUp/Down, Rest; pinch hysteresis (enter 0.35 / exit 0.55 × palm scale) and frame debouncing; unit-tested. *(Spec: [[Hand Gestures#3]])*
- [~] **Temporal layer** — hysteresis + debounce done; tap-vs-hold (🤙), swipe velocity and the shell-gesture routing land with **M4** (they only matter once gestures control things).
- [x] **Morphing cursor** — ring + dot (rest/point) tightening with pinch progress, solid dot (pinch, accent-B for middle-pinch), ✊ grab glyph, ✋ palm bloom pulse, voice ring with pulsing red core (call sign), 👍/👎 badges, frozen dim blink on tracking loss. Delivered to the shell via new ABI 1.1 events (`HandUpdate`, `CameraStatus`). *(Spec: [[UI#4]])*
- [x] **Camera status UI** — tray indicator: 📷 off / on · no hands / tracking · N hands. First-run calibration screen deferred to **M5** (Settings app).
- *Verified in Chrome:* camera-denied degradation (tray 📷 off, zero errors) and the full pipeline via synthetic landmark injection through the real JS→WASM bridge — Point ring, Grab fist, tracking-lost freeze all rendered; classifier covered by native unit tests. **Live-webcam testing needs a machine with a camera — user to confirm.*

## M4 — Gesture Control
*Goal: operate the whole desktop by hand: click, open, move, throw.*

- [ ] **Pointer-layer gestures** — pinch = click, pinch-hold = drag (windows, sliders), middle-pinch = context menu, two-finger = scroll; hand-source hit-target tolerance + dwell-as-hover. *(Spec: [[Hand Gestures#3]], [[UI#4]])*
- [ ] **Window manipulation** — grab (✊) a titlebar to move a window; snap zones; release inherits velocity for a gentle toss.
- [ ] **3D object interaction** — grab stage objects (app icons re-arrangeable, decorative props) via ray-picking + kinematic spring; throwing works. *(Physics arrives fully in M7; M4 uses simple kinematics.)*
- [ ] **Shell gestures** — open-palm hold = launcher, swipe = desktop switch, double-palm push = show stage, thumbs up/down = confirm/dismiss. *(Spec: [[Hand Gestures#3, #4]])*
- [ ] **Gesture feedback** — recognition glyph flash at cursor; undo toast for gesture-initiated actions. *(Spec: [[UI#6]])*

## M5 — LLM Integration & Conjuring
*Goal: the AI is alive — palette conversations, system control, and apps conjured from natural language.*

- [ ] **Provider layer** — `LlmProvider` trait; Anthropic (direct-browser CORS) + OpenAI-compatible (covers Ollama/LM Studio local) backends; streaming (SSE). *(Spec: [[AI System#2]])*
- [ ] **Settings → AI** — provider profiles UI, key entry (masked, kernel-side only), model pick, budget caps.
- [ ] **Agent manager** — agents as kernel objects; System Assistant + App Smith templates; capability-derived tool schemas; tool-call log to `/sys/ai/log`. *(Spec: [[AI System#1, #3]])*
- [ ] **Command palette** — `Ctrl+K` / 🤙 tap; command mode + AI mode with streaming; voice mode wiring (Web Speech). *(Spec: [[UI#2.4]])*
- [ ] **Conjure runtime** — complete `pmos-conjure`: full schema, expression parser/evaluator, action interpreter, limits; App Host process. *(Spec: [[App DSL]])*
- [ ] **App Smith loop** — generate → validate → repair (≤3 rounds) → spawn; save to `/apps`; modify-existing-app flow. *(Spec: [[AI System#4]])*
- [ ] **Consent sheets** — capability requests from agents and conjured apps. *(Spec: [[UI#5]])*

## M6 — VFS & Core Apps
*Goal: real persistent files and the built-in userland.*

- [ ] **OPFS VFS** — storage worker with sync access handles, POSIX-like tree, watch events, `/sys` synthetic tree; IndexedDB fallback. *(Spec: [[Architecture#4.6]])*
- [ ] **Terminal** — command parser (`ls`, `cd`, `create-app`, `ai-log`…), NL mode (`>` prefix), the reference ABI client. *(Spec: [[Pseudo Motion OS]] §4.6 heritage, [[UI]])*
- [ ] **File Explorer** — tree view, drag-and-drop, context menus, app-bundle launch.
- [ ] **Motion Notes MVP** — markdown editor, wikilinks + backlink index, inbox + daily notes, 🤙-hold voice capture, 3D graph view (physics-laid-out). *(Spec: [[Notes System]])*

## M7 — Physics & Ray Tracer
*Goal: the stage becomes alive and the showcase renderer lands.*

- [ ] **rapier3d integration** — fixed-timestep world, stage props as rigid bodies, grab/throw upgraded from kinematic to physical. *(Spec: [[Architecture#4.2]])*
- [ ] **Ray tracer** — WGSL compute Whitted tracer (spheres, planes, reflect/refract, area lights), progressive accumulation, budgeted dispatch, scene-editor window. *(Spec: [[Architecture#4.3]])*

## M8 — Polish, Browser App, Desktop & Release
- [ ] **Browser app** — best-effort iframe browsing (browser mode). *(Spec: [[Pseudo Motion OS]] §4.8 caveats)*
- [ ] **Tauri shell** — `pmos-desktop` crate, native FS mounts, child-webview browsing.
- [ ] **Performance pass** — frame budget audit, wasm size (`opt-level=s`, `wasm-opt`), load time.
- [ ] **Demo golden path** — scripted demo: launch → gestures → conjure an app by voice → manipulate it in 3D.
- [ ] **Deploy** — static hosting + CI (build, test, deploy on push).

---

*When a task completes: tick it here, log it in [[Changelog]], commit incrementally.*
