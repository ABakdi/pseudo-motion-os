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
- [x] **Hand Tracker app** *(added by user request, 2026-07-30)* — dock app ✋ opening bottom-right: camera preview with landmark-skeleton overlay, privacy mode (feed off = landmarks on black, pixel streaming fully stopped), detection settings (hands, confidences, smoothing preset, pinch thresholds), Enable-camera re-request. New plumbing: ABI 1.2 (`HandsViewer`/`HandsTune`/`CameraStart` syscalls, `RawHands` events, **capability delegation** on `ProcRegister`); preview pixels bypass the kernel entirely. *(Spec: [[Hand Gestures#8. The Hand Tracker app]]; verified in Chrome incl. landmark overlay via synthetic frames — a note: hidden/occluded tabs suspend rAF so rendering pauses, which is expected browser behavior.)*

## M4 — Gesture Control ✅ *(core scope; deferred items annotated)*
*Goal: operate the whole desktop by hand: click, open, move, throw.*

- [x] **Pose → pointer-intent fusion** (new kernel module `input/fusion`) — pinch = primary press/release, middle-pinch = secondary, two-finger = scroll with parked pointer (natural direction), grab = whole-hand drag; held buttons always release on tracking loss; unit-tested (press/release ordering, loss-release, grab lifecycle, scroll). Intents are synthesized into egui pointer events by the platform glue, so **every existing widget works with hands** — sliders, buttons, checkboxes, window titlebars. *(Spec: [[Hand Gestures#3, #5]])*
- [x] **Window manipulation** — grab (✊) routes by context: over UI it presses-and-drags (windows move by titlebar); over the open stage it orbits the camera. Snap zones and velocity toss deferred (egui windows have no toss physics — revisit with M7).
- [x] **Shell gestures** — open-palm hold (0.6 s) toggles the new **launcher overlay** (dimmed backdrop, app grid, Esc/backdrop-click closes, dock ◆ button); *(verified end-to-end with synthetic gestures in Chrome: palm → launcher, point → tile hover, pinch → press)*. Deferred until their targets exist: swipe (needs workspaces), double-palm show-stage (needs minimize-all), thumbs confirm/dismiss (needs consent dialogs, M5).
- [x] **Gesture feedback** — click ripple at the cursor on pinch onset, on top of the existing morphing-cursor forms.
- [ ] *(moved to M7)* **3D object interaction** — grab/throw stage objects: the stage currently has no objects (floating icons were removed); lands with rapier props.
- *Verification note:* full pinch-click cycles need real framerate (egui's 0.8 s click window) — remote frame-pumped testing confirms hover/press/release mechanics; live-webcam click/drag confirmation by user.

## M5 — LLM Integration & Conjuring ✅ *(core slice; deferrals annotated)*
*Goal: the AI is alive — palette conversations, system control, and apps conjured from natural language.*

- [x] **Conjure runtime** — `pmos-conjure` is real: document model (strict schema), expression language (tokenizer + precedence parser + evaluator, templates, 20+ builtins, ternary, divide-by-zero-safe), action interpreter with step/if/emit-depth budgets, 7-stage validator with machine-readable errors, and the App Host rendering the v1 widget subset (column/row/group/scroll/label/button/text_input/slider/checkbox/progress/if) with live bindings. 13 native tests. *(Spec: [[App DSL]] — list_view/canvas/dropdown/map-actions are the remaining spec surface.)*
- [x] **Provider layer** — kernel builds complete requests (keys never leave it); Anthropic (direct-browser CORS header) + OpenAI-compatible (covers Ollama/LM Studio) with SSE streaming through a thin JS fetch bridge (`llm.js`). *(Spec: [[AI System#2]])*
- [x] **Agent manager** — System Assistant + App Smith as kernel agents with system prompts (the App Smith prompt embeds the compact Conjure contract), bounded per-agent history, one uniform streaming path (errors arrive as terminal chunks). Capability-derived tool schemas + `/sys/ai/log` deferred to M6 (needs the /sys tree).
- [x] **Settings → AI** — provider/base-URL/model/masked-key form issuing `AiConfigure`; config persists across reloads (localStorage until the VFS; documented caveat). Budget caps deferred.
- [x] **Command palette** — dock ✨ / `Ctrl+K` / 🤙 tap; three modes: fuzzy commands (app names, `demo`, `launcher`), `>` assistant chat with streaming, `make/create/build/conjure …` App Smith flow. Voice wiring deferred to M6 (with notes/whisper work).
- [x] **App Smith loop** — generate → extract JSON → validate → auto-repair (≤2 rounds, errors+document fed back) → spawn as a real process via `ProcRegister`/`WinCreate`. Save-to-`/apps` + modify-existing deferred to M6 (VFS).
- [ ] **Consent sheets** — deferred: conjured apps currently receive only the default capability set (requested capabilities are ignored), so no consent surface is required yet. *(Spec: [[UI#5]])*
- *Verified in Chrome offline:* palette open (dock ✨ — new, also specced §2.2), `demo` command → pomodoro Conjure app spawned through the full validate→syscalls→App Host path, timer live (25:00 → 24:59), Start→Pause ternary re-render, toasts/notify path in place. **Real LLM streaming needs an API key — user to confirm via Settings → AI.**

## M6 — VFS & Core Apps ✅ *(core slice; deferrals annotated)*
*Goal: real persistent files and the built-in userland.*

- [x] **VFS** — kernel-resident POSIX-like tree persisted **write-through to OPFS** via `storage.js` (main-thread async API; the sync-access-handle worker from the spec proved unnecessary at these file sizes — spec note). Boot loads everything back (`vfs ready (persistent: true)` verified in Chrome). `/sys` synthetic (`fps` live EMA, `abi`), read-only. `FsDelete`/`FsMkdir` + `Bytes`/`Entries` replies (ABI 1.3). **Scoped fs capabilities enforced** — prefix-matched `FsRead`/`FsWrite` per process, delegated per app. 3 native tests. *(Deferred: IndexedDB fallback, `/sys/processes`, `/sys/ai/log`.)*
- [x] **Terminal** — real command set over syscalls: `ls·cd·cat·write·mkdir·rm·apps·run·fps·clear·about·help` + `>` natural-language mode streaming from the System Assistant into the log; the reference ABI client at last. *(verified: help/session in Chrome)*
- [x] **File Explorer** — breadcrumb browser, typed icons, ✨ `.conjure` bundles launch on click, delete, new-folder. *(Deferred: drag-and-drop, context menus.)*
- [x] **Motion Notes MVP** — sidebar list (recursive /notes scan), editor, save, new note, 📅 daily note, `[[wikilink]]` extraction with click-to-follow (ghost links create the note — the Obsidian idiom), backlinks panel. **Notes runs capability-scoped to `/notes` only** — the scoping model's reference user. *(Deferred: 3D graph view → M7 physics; 🤙-hold voice capture → with the speech engine.)*
- [x] **Conjured apps persist** — every successful conjure saves `/apps/<id>.conjure`; relaunchable from Files (click), terminal (`run <id>`), across reloads.

## M7 — Physics & Ray Tracer ✅ *(code complete; in-browser visual pass pending user run)*
*Goal: the stage becomes alive and the showcase renderer lands.*

- [x] **rapier3d integration** — fixed 120 Hz timestep with a render-rate accumulator; ~~8 stage props (cubes + spheres, palette-colored)~~ **demo props removed by user decision (2026-08-01)** — the stage boots clean; `spawn_prop` and the whole grab/throw pipeline remain for notes-as-bodies, conjured objects, and the future `stage_spawn` tool; floor collider matched to the grid plane; **grab = ray-pick + damped spring force** (body stays dynamic so collisions keep resolving), release inherits velocity — throwing works; both ✊ hand-grab and mouse-drag pick props (modality parity), falling back to camera orbit on empty space. 2 native tests (settle-on-floor, ray-pick). *(Spec: [[Architecture#4.2]])*
- [x] **Prop rendering** — depth buffer added to the render graph (sky renders behind everything, props depth-tested, floor blends over, egui on top); instanced cube/sphere meshes with per-instance position/rotation/scale/color and Blinn-lambert + rim shading.
- [x] **Ray tracer** — Whitted WGSL compute pass (512×384 into a storage texture): mirror + glass spheres, orbiting diffuse spheres, checkered plane, point light with hard shadows, iterative reflect/refract up to 5 bounces, sky gradient; displayed live in the new **◇ Ray Tracer app** with bounce/animate controls via the `RtConfig` syscall (ABI 1.4). *(Deferred: progressive accumulation/soft shadows, scene editing beyond the controls; per-frame full trace is cheap at this size.)*
- [ ] *(carried)* **Notes 3D graph view** — still pending; wants notes-as-bodies on the stage.
- *Verification:* native tests green; browser run (props visible/grabbable, RT window) awaits the user — the extension disconnected during final checks.

## M8 — Polish, Browser App, Desktop & Release ✅ *(deferrals annotated)*
- [x] **Browser app** — egui chrome (URL bar, honest embed-refusal notice) + a real DOM iframe the platform overlays on the window's content rect each frame (sandboxed; egui points ≡ CSS px). Known limitation documented: the iframe always stacks above the canvas, so overlapping egui windows render beneath it; real browsing lands with Tauri child webviews. *(Spec: [[Pseudo Motion OS]] §4.8 caveats)*
- [x] **Tauri shell** — `crates/pmos-desktop` scaffold (Tauri 2, wraps the same `dist/`), deliberately **outside the workspace** so native GUI deps never gate the web build; build steps in [[Running Locally]]. *(Untested here — needs webkit2gtk; native FS mounts + child-webview browsing remain follow-ups.)*
- [x] **Performance pass** — release bundle with `wasm-opt -z` + LTO + `opt-level=s`: **6.5 MB wasm** (from 51 MB debug), ~6.6 MB total static bundle.
- [x] **Demo golden path** — [[Demo]]: the scripted five-minute showcase with speaker lines and fallbacks.
- [x] **Deploy** — GitHub Actions CI: tests + wasm check on every push; release build deployed to GitHub Pages from master (`--public-url /pseudo-motion-os/`). *(Pages enabled 2026-08-01 — the missing one-time setting was why every deploy failed; the site is live.)*

## Post-release — Field-testing stabilization
*Bugs and tuning found by using the OS for real, after the roadmap's core shipped.*

- [x] **Cursor stabilization** — fixed the cursor teleporting on pinch/fist: the anchor used to switch landmarks per pose (index tip ↔ palm), so gesturing moved the cursor at the exact moment of commit. Researched and specced the proper approach ([[Hand Gestures#2.1 Cursor stabilization|spec §2.1]]): one articulation-invariant anchor (palm centroid — wrist + MCP knuckles, the same stability MediaPipe's palm detector relies on), a commit lock with a 12 pt soft deadband engaging the instant the pinch starts forming (Ultraleap TouchFree's pattern), and ~80 ms release easing so letting go never jumps either. 4 native tests (pose-switch invariance, sub-deadzone freeze, drag follow-through, release easing). *(User-verified: much better; deadband tuning to revisit.)*
- [x] **Voice command palette** — 🤙 **held ≥ 0.6 s** (deliberate dwell — can't fire by accident; tap still toggles the text palette) opens the palette in voice mode: live transcript streams into the input line as you speak, end of utterance executes through the same routing as typed input (apps → launch with spoken "open the …" verbs handled, `make …` → App Smith, everything else → System Assistant). Engine: **Whisper in-browser by default** (`whisper-worker.js`, transformers.js, WebGPU→WASM; ~40 MB cached model) — works in **any browser** (Brave/Firefox included), fully offline after first download, audio never leaves the machine; Web Speech API kept only as an init-failure fallback. `speech.js` (AudioWorklet capture + energy endpointing) + `VoiceCapture` syscall + `VoiceStatus`/`VoiceTranscript` events (ABI 1.5, `voice:input` capability); **text-only kernel boundary** — audio never crosses. Esc cancels (and discards an in-flight transcription); engine errors and model-download progress surface in the palette. 1 native test (capability gating + directive generations). *(Deferred: voice notes → notes inbox; Settings model-size toggle.)*
- [x] **Assistant tool use** — the System Assistant can now ACT ([[AI System#3.1 v1 implementation|spec §3.1]]): prompt-level `@@tool`/`@@tool_result` protocol (provider-agnostic — works on Anthropic, OpenAI-compatible and local models alike), executed by the shell as a plain ABI client so every call passes the kernel's capability checks; 4-call budget per request. Tools: `sys_query`, `fs_list`, `fs_read`, `fs_write`, `app_open`. Transparency: 🔧 lines in the palette per call, toasts on writes. Works end-to-end from voice: *hold 🤙, say "what's in my notes?"*. 3 native tests (tool-call extraction). *(Deferred: Tier-2 destructive tools behind consent, /sys/ai/log, provider-native function calling, terminal-surface tool runs.)*
- [x] **Files redesign** — places sidebar (Home/Notes/Apps/Settings/System), grid ⊞ and list ☰ views, per-type icons, click-to-select with a resizable preview panel (text preview, ▶ Launch for `.conjure`, delete), breadcrumbs with ⬆ up, new file + new folder. Folders open on single click; `.conjure` launches on double click.
- [x] **Browser: load a working page immediately** — an empty iframe read as "broken"; the Browser now opens on Wikipedia with quick links to known-embeddable sites (HN, OSM embed, MDN). The X-Frame-Options limitation (undetectable cross-origin) stays documented; real browsing still lands with Tauri child webviews.
- [x] **Stage navigation** — camera pan added (`OrbitCamera::pan`, view-plane target with clamps): shift+drag or middle-drag over empty stage; ✌ over the stage zooms by hand (over UI it still scrolls); ✊ orbit/grab/throw and wheel zoom were already live. Full control map in the Settings footer. *(Deferred: two-fist rotate T2, hand pan.)*
- [x] **Stage control by gesture & voice** — 👍 hold (0.6 s) drops a cube, 👎 hold removes the newest object ([[Hand Gestures]] G9 v1 binding; consent dialogs will take precedence later); ✊/mouse grab-and-throw already covered props. Voice fast-path in the palette — "drop a cube", "spawn a ball", "clear the stage", "remove the last object" — executes instantly without an LLM round-trip; richer requests ("build me a tower") still flow to the assistant's stage tools.
- [x] **Stage objects & lighting (ABI 1.8)** — `StageSpawn/StageRemove/StageClear/StageImpulse/StageList` (PhysSpawn-gated) + `StageLight` (sun dir/intensity/ambient packed into the props-pass uniform); Settings → Stage panel (drop cubes/spheres, clear, sun sliders); **AI stage tools** — `stage_spawn/list/remove/clear/push/light`, budget raised to 8 so the assistant composes models from primitives and "animates" via physics impulses. *(Deferred: syncing stage objects into the ray-tracer scene; keyframe animation beyond physics.)*
- [x] **Appearance: backgrounds, color schemes, selectable text** — Settings → Appearance ([[UI#6.1 Appearance settings|spec §6.1]]): 4 sky presets (Deep Space / Ember Nebula / Aurora / Void — one sky shader, per-preset palette via uniform, `Background` syscall ABI 1.7) + 4 accent schemes (Ion / Ember / Verdant / Rose — runtime theme tokens, cursor/dock/palette restyle instantly); persisted to `/settings/appearance.json`, shell re-applies at boot. Text selectable everywhere with real clipboard copy (`navigator.clipboard` bridge); Notes sidebar resizable; Settings scrolls. *(Deferred: light theme, reduced-motion toggle.)*
- [x] **In-browser LLM is the default provider** — AI works with ZERO setup: WebLLM (ABI 1.6 kind 2) runs the model on the user's GPU in the browser, free, no API key, downloaded once and cached, offline afterwards, prompts never leave the machine. Three performance tiers in Settings → AI (Fast 0.5B ~0.6 GB · Balanced 1B ~0.9 GB default · Quality 3B ~1.9 GB); download progress streams live into the palette via the new `'\r'`-replace AiChunk semantics; remote providers (Anthropic / OpenAI-compatible / Ollama) remain one dropdown away. *(Deferred: WebLLM worker offload, per-agent provider binding.)*

---

*When a task completes: tick it here, log it in [[Changelog]], commit incrementally.*
