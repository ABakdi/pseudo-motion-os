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

## M1 — Landing & Launch Experience
*Goal: visitor opens the URL → impressive home page with logo → presses Launch → grants permissions → arrives on the desktop. The landing page is plain HTML/CSS (instant paint, works even without WebGPU); WASM boots on Launch.*

- [ ] **Logo & identity** — PMOS logomark (SVG, animatable), wordmark, color tokens shared between landing CSS and egui theme. *(Spec: [[UI#2.0 Boot & Launch experience]])*
- [ ] **Home page** — full-viewport hero: animated starfield/nebula CSS-or-canvas backdrop, logo, one-line pitch, **Launch** call-to-action; feature blurbs (gestures, AI conjuring, 3D space) beneath; graceful "browser not supported" state when WebGPU is absent (check already implemented in M0).
- [ ] **Launch flow** — Launch click → WASM boot (loading progress on the button itself) → **permission onboarding**:
    - [ ] Sequential permission cards: camera (gestures), microphone (voice), notifications — each with a one-line why, `Enable` / `Later` (browser prompts must be user-gesture-driven; skipped ones re-request on first relevant use).
    - [ ] Permission state persisted to `/sys/permissions` so onboarding shows only once.
- [ ] **Transition** — cinematic fade from landing into the 3D desktop (M2 provides the desktop; until then, a dark "kernel ready" scene).

## M2 — Desktop: 3D Space, Galaxy, App Icons
*Goal: the OS desktop — a 3D space wrapped in a distant galaxy that can never be reached, floating app icons that open windows, full mouse/keyboard control.*

- [ ] **wgpu foundation** — device/queue/surface init on the canvas, render-graph skeleton (clear → stage pass → overlay pass), resize handling, frame timing in `/sys/fps`. *(Spec: [[Architecture#4.1]])*
- [ ] **egui integration** — `egui-wgpu` overlay pass; demo window proves 2D UI composites over 3D.
- [ ] **Galaxy skybox** — procedural starfield + nebula rendered at infinite depth (fullscreen shader driven by view *rotation only* — zoom/translation never changes parallax, so it is provably unreachable); slow drift + twinkle animation. *(Spec: [[UI#1. The Two Planes]])*
- [ ] **Stage camera** — orbit/zoom with mouse (drag = orbit, wheel = zoom), zoom clamped to \[min, max]; `Home`/double-click = reset view.
- [ ] **Floating app icons** — system apps (Terminal, Files, Notes, Settings, Browser) as glowing billboard icons arranged in the space; hover glow, click → opens the app window; label fades in on hover. *(Spec: [[UI#2.8 Floating app icons]])*
- [ ] **Window manager v1** — draggable/resizable/closable egui windows, focus model, taskbar/dock strip. *(Spec: [[UI#2. Shell Elements]])*
- [ ] **Kernel ABI v1 live** — syscall dispatcher + process registry + capability checks wired; shell and each opened app run as registered processes; the ABI stops being a stub. *(Spec: [[Architecture#6]])*
- [ ] **Keyboard & mouse** — full winit event routing through the input pipeline with source tags (foundation for gesture fusion in M3).

## M3 — Camera & Hand Gesture Detection
*Goal: webcam on, hands tracked, gestures recognized — and a living cursor that morphs with the hand.*

- [ ] **Gesture worker** — JS worker: getUserMedia → MediaPipe `tasks-vision` HandLandmarker (GPU delegate) → landmark frames → WASM. Camera frames never leave the worker (privacy boundary). *(Spec: [[Architecture#3]], [[Hand Gestures#2]])*
- [ ] **Landmark ingestion** — One-Euro filtering, control-box mapping to screen space, confidence gating (freeze, never jump).
- [ ] **Pose classifier** — rule-based static poses: Point, Pinch, MiddlePinch, Grab, OpenPalm, TwoFinger, CallSign, ThumbsUp/Down, Rest. *(Spec: [[Hand Gestures#3]])*
- [ ] **Temporal layer** — hold-times, tap-vs-hold (🤙), swipe velocity detection, hysteresis thresholds; all tunables in one config struct. *(Spec: [[Hand Gestures#6]])*
- [ ] **Morphing cursor** — the signature cursor: not an arrow but a **shape-shifting glyph** rendered in the overlay pass — open ring (rest/point) that tightens as pinch confidence rises, closes to a dot on click, becomes a fist glyph while grabbing, a palm bloom for launcher, a mic glyph in 🤙 mode; color/size states for hover, tracking-lost (frozen+dim). *(Spec: [[UI#4. Pointer-Source Adaptation]])*
- [ ] **Camera status UI** — tray indicator (on/off/hands-detected), first-run calibration screen (control box + pinch distance). *(Spec: [[Hand Gestures#6]])*

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
