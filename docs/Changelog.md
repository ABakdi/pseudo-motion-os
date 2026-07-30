# Changelog
**Pseudo Motion OS** · part of [[Pseudo Motion OS]]

Running log of every change to the specification and (once building starts) the code. Newest first. **Every update to any file in this vault or to the codebase gets an entry here — no exceptions.**

Entry format:
```
## [version or date] — short title
### Specs | Code
- what changed, in which file(s), and why (one line each)
```

---

## [2026-07-30] — Hand Tracker app (user request)
### Code
- `pmos-apps/hand_tracker`: new dock app ✋ — window opens bottom-right with the mirrored camera preview, hand-skeleton overlay (bones + points per hand), privacy mode (feed toggle off = landmarks on black and the platform stops streaming pixels entirely), live pose/hand-count status, Enable-camera re-request, and detection settings (hands, MediaPipe confidences, smoothing preset, pinch enter/exit, reset).
- ABI 1.2: `HandsViewer { open, stream_feed }`, `HandsTune(HandsTuning)`, `CameraStart` syscalls; `RawHands` event (gated on viewer-open + `InputRawHands`); **capability delegation** — `ProcRegister` now carries requested caps, honored only if the registering caller holds them itself (the shell delegates `InputRawHands` to the Hand Tracker process).
- Kernel: hands-directives block (generation-counted platform intent), runtime-tunable pinch thresholds and smoothing presets in the recognizer; `gesture.js`/worker: mirrored 320×240 preview streaming (only while requested) and landmarker rebuild on configure.
- Platform: preview pixels flow JS → `pmos_camera_frame` → egui texture → shell, deliberately bypassing the kernel (privacy boundary intact).
- Verified in Chrome: window layout/controls, camera-off state, skeleton overlay + Point classification via synthetic frames, capability delegation. Debug note: occluded/hidden tabs suspend rAF so the render loop pauses — looked like a freeze during testing; expected browser behavior, harmless in real use.
### Specs
- [[Hand Gestures]]: new §8 documenting the app and its plumbing; [[Todo]] annotated under M3.

## [2026-07-30] — Milestone 3: hand tracking and the morphing cursor
### Code
- `pmos-web/gesture.js` + `gesture-worker.js`: capture pipeline — getUserMedia on the main thread (unavailable in workers), ImageBitmap transfer to a module worker running MediaPipe tasks-vision HandLandmarker (GPU delegate, ≤2 hands), camera-paced via requestVideoFrameCallback; landmarks posted back as flat Float32Arrays into the WASM bridge (`pmos_hands_frame` / `pmos_camera_status`).
- `pmos-kernel/input/hands`: One-Euro filter, control-box mapping (x-mirrored), rule-based pose classifier (9 poses + Rest) with pinch hysteresis and frame debouncing, 0.5 s tracking-loss freeze; 4 unit tests.
- `pmos-abi` 1.1: `KernelEvent::HandUpdate { pose, pinch, pos, tracking, hands }` and `CameraStatus`; replaced the placeholder `HandPoseChanged`.
- `pmos-apps/cursor`: the morphing cursor — ring tightening with pinch progress, pinch dot, ✊ / ✋ bloom / voice-pulse / 👍 👎 glyphs, frozen dim ring on tracking loss; tray camera indicator in the shell.
- Verified in Chrome: camera-denied path, plus the full pipeline via synthetic landmark injection through the real JS bridge (Point ring, Grab fist, tracking-lost freeze). Live webcam pending user hardware.
### Specs
- [[Architecture]] §3 and [[Hand Gestures]] §2/§7 corrected: video capture is main-thread (getUserMedia is worker-unavailable); privacy boundary restated precisely.
- [[Todo]]: M3 done; calibration screen moved to M5 (Settings); temporal tap/hold/swipe layer moved to M4 where it becomes meaningful.

## [2026-07-30] — Removed floating stage icons (user decision)
### Specs
- [[UI]] §2.8 rewritten: the dock and launcher are the app-launching surfaces; the Stage is reserved for content, not chrome. Stage/arrival descriptions updated; [[Todo]] M2 annotated.
### Code
- `pmos-apps/shell`: removed `stage_icons` and `StageView`; the shell no longer needs the camera transform (a projection helper returns with M4's 3D interactions).

## [2026-07-30] — Milestone 2: the 3D desktop
### Code
- `pmos-kernel/gfx`: graphics engine — wgpu 29 render graph (galaxy sky pass → holo-grid floor pass → egui overlay), orbit camera with pitch/zoom clamps, WGSL shaders. The galaxy uses a rotation-only inverse view-projection: it is unreachable by construction (UI spec §1).
- `pmos-abi`: `KernelApi` trait + `Reply` (userland's complete kernel surface), `ProcRegister` syscall.
- `pmos-kernel`: syscall dispatcher with per-call capability checks, process table (shell = Pid 1, shell grant; minimal default grant for apps), window registry, per-process event queues; minimal input pipeline with source tags.
- `pmos-apps`: the shell — floating stage icons (3D positions projected to overlay, bob/hover/labels), egui window manager, dock with running indicators, PMOS theme; built-in app stubs (Terminal with help/about/clear, Files, Notes scratch, Settings, Browser).
- `pmos-web`: winit `ApplicationHandler` on the stage canvas, async wgpu init, camera input routing (egui gets first claim), per-frame canvas/surface size sync, idempotent `pmos_launch`.
- Trunk.toml: watch the whole workspace (was only watching `pmos-web`, silently serving stale builds).
- Verified in Chrome end-to-end: launch → galaxy/floor/icons/dock render, orbit/zoom/reset, icon click → app window (real kernel processes: Terminal Pid 2, Settings Pid 3), terminal input.
- Debugging notes for posterity: a canvas-sized-by-CSS surface configured before layout settles renders as one uniform stretched color (fix: per-frame size sync); egui's bundled emoji font lacks 🗂 (renders tofu; 📁 works).
### Specs
- [[Todo]]: M2 marked done with verification notes and a known input-routing quirk deferred to M4.

## [2026-07-30] — Added local development guide
### Specs
- New [[Running Locally]]: prerequisites (incl. the prebuilt-trunk fallback for the `libdeflate-sys` build failure), verify/run/build commands, dev tips (resetting onboarding, kernel logs), troubleshooting; linked from the README.

## [2026-07-30] — Milestone 1: Landing & launch experience
### Code
- `pmos-web/index.html`: full landing page — animated SVG logomark (counter-orbiting gradient arcs), gradient wordmark, canvas starfield (3 depth layers, twinkle, parallax drift, `prefers-reduced-motion` aware), nebula glows, LAUNCH CTA that enables when the WASM core loads, feature cards, WebGPU-unsupported notice, design tokens as CSS variables.
- Permission onboarding: sequential camera/mic/notifications cards with Enable/Later and progress dots; results persisted (localStorage until the VFS lands, then `/sys/permissions`); returning users skip straight to boot.
- `pmos-web/src/main.rs`: page load stays lightweight; `pmos_launch(permissions_json)` exported to the page boots the kernel and swaps landing → OS root.
- Verified end-to-end in Chrome: landing render, onboarding flow, kernel boot (ABI 1.0 logged), returning-user skip path.
### Specs
- [[Todo]]: M1 marked done with verification notes.

## [2026-07-30] — Milestone 0: Rust workspace scaffolded; roadmap and launch-experience specs
### Code
- Created the cargo workspace with the six crates from Architecture §10: `pmos-abi` (syscall/event/capability types, ABI v1.0), `pmos-kernel` (Kernel root + subsystem stubs), `pmos-platform` (all web-sys interop; WebGPU boot check via `Reflect`), `pmos-apps` (userland, ABI-only by crate graph), `pmos-conjure` (validator skeleton with machine-readable errors + 3 tests), `pmos-web` (trunk entry, placeholder page).
- Toolchain: `rust-toolchain.toml` (stable + wasm32), trunk installed (prebuilt binary; `cargo install` fails locally on libdeflate-sys). `cargo check/test` green natively and for wasm32; `trunk build` produces `dist/`.
- Added README with dev instructions and crate map.
### Specs
- Added [[Todo]]: milestone roadmap M0–M8 with tasks/subtasks; M0 marked done.
- [[UI]]: new §2.0 Boot & Launch experience (WASM-free landing page with logo + Launch CTA, permission onboarding cards for camera/mic/notifications with Enable/Later); Stage now specifies the unreachable galaxy backdrop (rotation-only parallax); new §2.8 floating app icons in the Stage; §4 expanded with the full morphing-cursor form table.
- [[Architecture]]: boot sequence updated for the landing page + onboarding flow; permissions UX changed from purely-lazy to onboarding-with-skip (user decision).

## [2026-07-30] — Repository created
### Specs
- Project moved into the `pseudo-motion-os` GitHub repository (github.com/ABakdi/pseudo-motion-os); the Obsidian vault now lives under `docs/`.
- Added `.gitignore` (Obsidian workspace state, build artifacts).
- Workflow rule adopted: commit incrementally with meaningful messages.

## [v0.3] — 2026-07-29 — Spec restructured into multi-file vault
### Specs
- Split the single spec into: [[Pseudo Motion OS]] (introduction, philosophy, use cases, stack justification), [[Architecture]], [[UI]], [[Hand Gestures]], [[Notes System]], [[AI System]], [[App DSL]], and this changelog.
- Named the app DSL **Conjure** and specified it fully: document structure, widget catalog, expression grammar (EBNF), action catalog, limits, validation stages, security model, complete example.
- Specified the full gesture vocabulary: 9 one-hand core gestures + 4 two-hand enhancers, one-hand-is-enough policy, ergonomics rules, recognition pipeline, tuning parameters. 🤙 tap = voice palette, 🤙 hold = voice note.
- Added the **Motion Notes** system spec (new subsystem): Obsidian-style markdown + wikilinks + backlinks in the VFS, 3D graph view driven by the physics engine, system-wide voice capture to `/notes/inbox`, capability-scoped AI assistance.
- Specified multi-agent AI: agents as kernel objects with per-agent providers/capabilities; provider profiles (Anthropic direct, OpenAI-compatible incl. local servers like Ollama, WebLLM in v2); tool interface derived from capabilities; app-generation repair loop; risk-tiered safety.
- Detailed architecture: 4-layer model, platform bridge crate isolating all JS interop, worker topology, frame loop, syscall ABI shape with versioning, crate layout.
- Added use cases: live teaching / real-time presentation (flagship), 3D/2D artists, kiosks, streamers, knowledge work, HCI research.

## [v0.2] — 2026-07-29 — Technical revision of the original spec
### Specs
- Updated stale versions: wgpu 0.19 → 29+, egui → 0.35+, `@mediapipe/hands` (deprecated) → `@mediapipe/tasks-vision` HandLandmarker.
- Dropped WebGL2 fallback (no compute shaders; WebGPU now default-on in all major browsers).
- Physics moved explicitly to CPU rapier; GPU physics reclassified as post-v1 stretch (rapier has no GPU backend).
- Replaced "AI compiles Rust→WASM on the fly" (infeasible in-browser) with the interpreted declarative app format.
- LLM strategy: remote API first (Anthropic direct-browser CORS), WebLLM as v2 local backend, one abstraction; dropped llama.cpp-WASM (CPU-only, too slow).
- Security rewritten around host-enforced capabilities; same-memory "capability tokens" dropped (not real isolation).
- Flagged WASM-threads ↔ iframe conflict (COOP/COEP); deferred threads, workers-only concurrency in v1.
- Milestones: added Milestone 0 (build infra), moved kernel ABI and AI integration earlier, defined the golden demo path.

## [v0.1] — original — Initial specification
- Single-document spec: vision, two-layer architecture, feature set (desktop, 3D/physics, ray tracer, gestures, AI, terminal, file explorer, browser app), Rust/WASM/wgpu/egui/rapier/Tauri stack, browser + desktop install models, 13-week milestones.

---

## Decisions Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-07-29 | Custom wgpu layer, no Bevy | Full render-graph control (RT compute pass + egui-to-texture); small binary |
| 2026-07-29 | AI apps = interpreted declarative format ("Conjure") | No in-browser rustc; host-enforced sandbox; reliable LLM output; evolvable |
| 2026-07-29 | LLM: remote API first, WebLLM later, one provider abstraction | App generation needs frontier quality; local = backend swap later |
| 2026-07-29 | WebGPU-only, no WebGL2 fallback | WebGL2 lacks compute; WebGPU now mainstream |
| 2026-07-29 | Physics on CPU (rapier); GPU physics = stretch | rapier has no GPU backend; CPU ample for desktop scenes |
| 2026-07-29 | Single-threaded WASM + JS workers in v1 | COOP/COEP conflicts with iframe browsing; workers cover real needs |
| 2026-07-29 | Notes = plain markdown in VFS, Obsidian-compatible | Zero lock-in; Tauri mode can share a real Obsidian vault |
| 2026-07-29 | 🤙 gesture anchors all voice entry (tap=palette, hold=note) | One memorable "talk" gesture; topologically unique for the tracker |
| 2026-07-29 | Rule-based gesture classification (no trained model) | 9 topologically distinct poses; deterministic, tunable, debuggable |
