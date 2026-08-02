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

## [2026-08-02] — One cursor, pinch owns objects, fist owns the space
### Code
- **✊/👍 confusion fixed** (user-reported "almost always thumbs-up"): a fist's wrapped thumb passed the weak extension check and fell into the thumbs branch. Thumbs-up/down now require a LONG (>1.35× palm) and clearly VERTICAL thumb; anything less is Grab. 2 new classifier tests.
- **Role split (user decision):** 👌 pinch = click AND object grab — pinching a stage object attaches the spring (release throws), exactly like mouse click-drag; ✊ fist = the space — window drag over UI, camera orbit anywhere else. No more overload, and it matches CSL §5's pinch-select.
- **One cursor for all sources (user decision):** the OS arrow is hidden; the PMOS ring follows the shared pointer everywhere — hand poses morph it while the hand drives, mouse presses tighten it into the solid dot with the same click ripple. Mouse priority: hand pointer motion yields for 1 s after any mouse movement, so the two never fight.
### Specs
- [[Hand Gestures]] G2/G4 rewritten for the pinch/fist role split and classifier rule.

## [2026-08-02] — Stage input round 2: the background hint layer, RECORD off the fist, hover glow
### Code
- **Third input blocker found** (live-debugged): the tray hint — an `Order::Background` egui Area — was returned by `layer_id_at` over a band of the screen, so stage presses there read as "on UI" and orbit/grab/zoom never engaged. The hint and toasts are now `interactable(false)`, and all stage routing treats Background-order layers as backdrops, never UI. **Wheel zoom verified working on the empty stage after the fix.**
- **RECORD remapped (user-reported conflict):** ✋→✊ squeeze collided with ✊ = grab — every grab toggled voice. RECORD is now a **still ✋ hold (1.0 s)** — "raise your hand to speak" — with drift abort (a moving palm is gesticulation), release-before-rearm, and refractory. The fist belongs exclusively to grabbing. Specs updated; 5 sign tests rewritten.
- **Objects are first-class (user request):** a per-frame hover pick (hand while tracking, else mouse; UI positions excluded) makes the prop under the pointer glow; the grabbed prop stays lit while held.
### Specs
- [[Computer Sign Language]] §4 RECORD row rewritten; [[Hand Gestures]] G5/tuning updated.

## [2026-08-02] — Stage interaction unbroken: the zero viewport and the poisoned pointer
### Code
Two root causes found by live-debugging with console instrumentation (user-reported: can't grab objects, stage controls dead):
- **`window.inner_size()` returns 0×0 inside web event handlers** — the pick ray for grabbing was built with a zero viewport → NaN ray → **every prop grab has silently missed since M7** (the native tests passed because they cast rays directly, never through screen coordinates). All viewport math now derives from the canvas client size (`viewport_pts()`), which is the truth on web.
- **The shared egui pointer poisoned mouse routing**: the hand cursor drives the same egui pointer, so a tracked hand hovering any UI — or holding a half-closed pinch — made egui consume every mouse press; orbit/zoom/pan went dead whenever the camera saw a hand. Stage routing now hit-tests the event's own position (`layer_id_at`), decoupling mouse and hand; an orbit in progress also no longer stalls when crossing a window.
- Verified via console logs: presses on empty stage route to Orbit with a real viewport. Grab/zoom/pan feel needs the user's hands+mouse.

## [2026-08-01] — Deferred-item sweep + the first CSL sign is live
### Code
- **Conjure widgets (M5 deferral):** `dropdown` (bind + literal/expression options) and `list_view` (items list + `template` string with `item`/`i` locals, optional `on_remove` handler receiving them) — validator whitelist, App Host rendering, App Smith contract updated.
- **/sys/processes + /sys/ai/log (M6 deferral):** live process table refreshed on register/kill; a 200-line AI activity ring (prompts in, replies done) — both `cat`-able in the terminal and visible in Files; `/sys/ai` is a real synthetic directory.
- **Whisper model size (voice deferral):** Settings → Voice picks tiny/base/small (~40/80/250 MB), persisted to `/settings/voice.json`; the platform reconfigures the speech engine (warm worker torn down, next session loads the new model).
- **M9 task 3 (partial): the CSL sign engine + RECORD** — `input/signs.rs`: per-sign FSMs over the debounced pose stream with the spec's Midas guards (0.3 s entry hold, 0.8 s squeeze window, 0.5 s refractory, Rest tolerated mid-squeeze, tracking loss aborts); ✋→✊ RECORD toggles voice capture with toasts both ways; new `Sign{CslSign}` event (ABI 1.10). 5 native tests (fire, too-short, expired, abort+refractory, loss). COMMAND/CANCEL follow with their milestones.

## [2026-08-01] — M9 tasks 1–2: the browser finally works; the Launcher is gone
### Code
- **Browser root cause** (live-debugged in Chrome against the deployed site): `windows()` computed the Browser's content rect into a local that was dropped — `self.browser_view` stayed `None`, the platform iframe never got a URL or rect, broken since M8 for every site. One assignment fixes it. Verified live: Wikipedia renders inside the window, scrolls natively, and the iframe tracks the window as it moves.
- **Launcher removed**: overlay, palm-hold trigger, dock ◆, palette command — all deleted per user decision; verified live (dock renders 8 icons, no ◆).

## [2026-08-01] — M9 specced: the Voice OS, Computer Sign Language, and face/eye roadmap
### Specs
- **[[Computer Sign Language]] (new):** motion signs as words — four-parameter sign anatomy (handshape/location/movement/orientation), the sign-FSM engine with Midas guards, v1 vocabulary (RECORD ✋→✊ squeeze · COMMAND ☝ chin-outward flick, ASL-inspired with attribution · CANCEL palm push), the two-handed 3D grammar (dominant selects, non-dominant modifies; two-palm zoom), and the M10 face layer (FaceLandmarker: 478 landmarks + 52 blendshapes, face gestures, honest coarse-gaze limits).
- **[[Voice Kit]] (new):** always-on transcription after onboarding, top-right widget (collapsed status chip never hidden while live / expanded live transcript with accent-colored commands), continuous Whisper session loop, `/voice` persistence with search and →note, and the AI context envelope (focused object, stage, windows, recent transcript).
- [[Hand Gestures]]: **Launcher removed entirely** (user decision) — palm belongs to RECORD; sign engine added to the pipeline; G5/tuning updated. [[UI]]: dock loses the launcher entry, §2.9 Voice Kit widget added. [[AI System]] §5: continuous mode, context envelope, and the three web tools (`web_open`/`web_search`/`web_fetch`) with their honest CORS constraints.
- [[Todo]]: **M9** (9 ordered tasks, immediate fixes first) and **M10** (face & eyes) added.

## [2026-08-01] — Stage navigation: camera pan and hand zoom
### Code
- `OrbitCamera::pan` — view-plane target panning with clamps (the galaxy stays unreachable); shift+left-drag or middle-drag over empty stage pans, "grab the world" direction.
- Hand ✌ (TwoFinger) over the empty stage now ZOOMS the camera; over UI it still scrolls — modality parity with the mouse wheel. ✊ orbit/grab/throw unchanged.
- Settings footer documents the full control map.

## [2026-08-01] — GitHub Pages live: deploy failures diagnosed and fixed
### Infra
- Every CI run since M8 built and tested green but died on `actions/deploy-pages`: **GitHub Pages was never enabled on the repo** (the documented one-time setting). Enabled via the API (`build_type: workflow`), re-ran the failed deploy job — **the site is live at <https://abakdi.github.io/pseudo-motion-os/>** (7.6 MB wasm served). Future master pushes deploy automatically.

## [2026-08-01] — "Create a rotating cube" goes to the stage, and it actually rotates
### Code
- Routing (user-reported): "create a rotating cube" started the App Smith (2D-app generator), which then failed on the small in-browser model ("returned no JSON"). Stage-object phrases (cube/sphere/ball + make/create/build/drop/spawn) now route to the stage BEFORE the App Smith; say "app" ("make me a cube app") to force conjuring.
- "rotating"/"spinning" spawns now spin: `StageImpulse` gains a `torque` field (ABI 1.9, `apply_torque_impulse`); the assistant's `stage_push` tool accepts `rx/ry/rz` torque too.
- The App Smith no-JSON error now says why and what to do (small model → Quality tier or remote provider).
### Specs
- [[AI System]] §3.1 `stage_push` doc updated.

## [2026-08-01] — Stage control by gesture and voice
### Code
- Gestures: 👍 held 0.6 s drops a cube (deterministic scatter, palette colors, toast confirms); 👎 held removes the newest object — G9's v1 idle binding; dialogs will take precedence when consent sheets land. Grab/throw with ✊ already worked (modality parity).
- Voice/typed fast path in the palette: "drop a cube", "spawn a ball", "clear the stage", "remove the last object" execute instantly through new palette outcomes — no LLM round-trip; anything richer still routes to the assistant's stage tools.
### Specs
- [[Hand Gestures]] G9 updated (0.6 s hold; stage binding + dialog precedence note); [[Todo]] entry.

## [2026-08-01] — Files redesign, working browser start page, and a controllable stage
### Code
- **Files**: full redesign — places sidebar, grid/list views with per-type icons, selection + resizable preview panel (text preview, `.conjure` manifest ▶ Launch, delete), breadcrumbs with ⬆, new-file/new-folder footer; folders open on single click, apps on double click.
- **Browser**: opens on Wikipedia instead of an empty frame (an X-Frame-Options refusal is a blank page and undetectable cross-origin — starting on a known-embeddable page proves the app works); quick links to embeddable sites.
- **Stage (ABI 1.8)**: `StageSpawn`/`StageRemove`/`StageClear`/`StageImpulse`/`StageList` (PhysSpawn-gated, physics-backed — objects fall, collide, grab, throw) and `StageLight` (sun direction/intensity/ambient, packed into the existing 96-byte props uniform); Settings → Stage panel with drop-cube/drop-sphere/clear and sun sliders; assistant gains the six stage tools with an 8-call budget — "build me a tower", "make it sunset", "throw the red cube" all work through capability-checked syscalls.
### Specs
- [[AI System]] §3.1: stage tools + raised budget. [[Todo]]: three entries under post-release.

## [2026-08-01] — Clean stage, real typography, and text selection that works
### Code
- Demo props removed (decision logged): the stage boots empty; `spawn_prop`, grab/throw and the physics pipeline remain for future content. Physics tests spawn their own fixtures.
- Typography: **Inter** (with an Inter Medium heading cut) and **JetBrains Mono** embedded as the UI typefaces (egui built-ins remain fallbacks for emoji/symbols); full text-style scale, wider item spacing/margins/button padding, 8px widget rounding, soft window/popup shadows, quiet raised widget fills with hairline strokes — accent color appears only on interaction.
- **Text selection fixed** (user-reported): while a hand was tracked, the fusion layer pushed a synthetic `PointerMoved` every frame, yanking the egui pointer away from the mouse mid-drag — selection was impossible with the camera on. Hand Move events are now suppressed while the left mouse button is down (the mouse owns the pointer during drags/selection) and deduplicated below 0.5 pt so a resting hand's jitter no longer floods the pointer stream.
### Specs
- [[Todo]] M7: props removal annotated; Decisions Log entry added.

## [2026-08-01] — WebLLM: fit the model to the machine; fix engine switching
### Code
- Fixed "PackedFunc has already been disposed" (user-reported when switching to the Quality tier): switching models created a second MLCEngine over the live one, corrupting the TVM runtime — there is now ONE engine, switched with `engine.reload()`, and a failed runtime is fully torn down before any retry.
- Machine probe (user request): at boot `webllm.js` reads RAM (`navigator.deviceMemory`) and GPU headroom (`adapter.limits.maxBufferSize`) and reports the fitting tier to the kernel — surfaced as `/sys/llm_tier`; Settings marks that tier "★ fits this machine" and bigger ones "may not fit", and when the user never saved an AI config the default model is fitted to the tier automatically.
- Runtime step-down ladder: a model that fails to load or infer falls back automatically — fp16→fp32, then tier by tier — with the progress line explaining each step ('\r'-reset keeps output clean); the error surfaces only when every candidate fails.
### Specs
- [[AI System]] §2 WebLLM row: machine-fit behavior noted.

## [2026-07-31] — Appearance: pickable backgrounds and color schemes; selectable, copyable text
### Code
- Sky shader: four background presets behind one uniform (`style`) — Deep Space (default), Ember Nebula, Aurora, Void; new `Background{style}` syscall (ABI 1.7, `SysQuery`-gated) sets it live.
- `theme.rs`: accent colors became runtime tokens (`accent_a()/accent_b()` over an atomic scheme id) with four schemes — Ion, Ember, Verdant, Rose; `set_scheme` restyles every egui style at once, so the cursor, dock, palette and selection recolor instantly.
- Settings → Appearance: background + scheme radio pickers with color swatches, applied live and persisted to `/settings/appearance.json` through ordinary FsWrite syscalls (Settings gained scoped `/settings` fs caps + SysQuery); the shell re-applies persisted appearance at boot (retrying while OPFS loads).
- Text: `selectable_labels` on everywhere; egui copy commands (Ctrl+C) are mirrored to the real browser clipboard via a `navigator.clipboard` bridge — egui-winit's wasm clipboard is internal-only.
- Responsiveness: Settings content scrolls; the Notes sidebar splitter is resizable (app windows were already resizable).
### Specs
- [[UI]] §6.1 (new): appearance settings, schemes, selectability/clipboard behavior.
- [[Todo]]: appearance work logged under post-release.

## [2026-07-31] — WebLLM: auto-select fp32 model variants on GPUs without shader-f16
### Code
- `webllm.js`: q4f16 model builds need WebGPU's `shader-f16` feature, which many Linux Vulkan drivers lack — pipeline creation died with "Invalid ShaderModule" (user-reported on Brave/Linux). The bridge now probes adapter features up front and swaps `q4f16`→`q4f32` in the model id when f16 is unsupported, and if a driver *claims* f16 but still rejects the shaders, retries once on the fp32 variant and remembers for the session.

## [2026-07-31] — AI with zero setup: in-browser WebLLM becomes the default provider
### Code
- ABI 1.6: provider kind 2 = in-browser WebLLM; `AiChunk` deltas starting with `'\r'` REPLACE accumulated text (transient progress lines that never pollute history).
- `pmos-kernel/ai.rs`: default config is now WebLLM Balanced (Llama-3.2-1B) — "no AI provider configured" is gone; kind-2 requests carry only model+messages (no URL/key); `'\r'` handled in the accumulator so history stays clean.
- `pmos-web/webllm.js` (new): lazy-loads @mlc-ai/web-llm from CDN on first prompt, streams OpenAI-style deltas through the existing bridge, serializes requests, maps failures to actionable errors; model download progress streams as `'\r'` chunks.
- Settings → AI: "In-browser (free, no key)" is the first provider option with three performance tiers (Fast ~0.6 GB / Balanced ~0.9 GB / Quality ~1.9 GB radio picker) replacing the URL/model/key fields; remote providers unchanged one dropdown away.
- Palette, terminal: `'\r'`-replace rendering for streamed lines.
### Specs
- [[AI System]] §2: WebLLM row promoted from v2 to implemented default, tiers and honest App-Smith-quality note documented.
- [[Todo]]: zero-setup AI logged under post-release.

## [2026-07-31] — The assistant gets hands: tool use over the syscall ABI
### Code
- `pmos-kernel/ai.rs`: new System Assistant prompt — prompt-level tool protocol (`@@tool` / `@@tool_result`), chosen over provider-native function calling so one mechanism works on Anthropic, OpenAI-compatible, and local models; "you cannot run system commands" is gone.
- Palette: parses trailing `@@tool` lines from replies (fence-tolerant), strips them from display, logs every call as a `🔧 tool args` line, enforces a 4-call budget per request, and feeds `@@tool_result` back to continue the stream; 3 extraction tests.
- Shell `execute_tool`: runs `sys_query`/`fs_list`/`fs_read`(4 KB cap)/`fs_write`/`app_open` as an ordinary ABI client — every tool call passes the kernel's capability checks like any process's syscall; writes raise a "🤖 assistant wrote …" toast (Tier-1 transparency).
- Voice→AI now closes the loop: hold 🤙, ask "what's in my notes?", the assistant lists /notes and answers.
### Specs
- [[AI System]] §3.1 (new): the v1 tool implementation, its transparency rules, and honest limitations (palette-only execution, no Tier-2 tools yet, /sys/ai/log pending).
- [[Todo]]: assistant tool use logged under post-release.

## [2026-07-31] — Voice works in any browser: Whisper in-browser becomes the default engine
### Code
- `pmos-web/whisper-worker.js` (new): `whisper-tiny` via transformers.js — WebGPU with WASM fallback, multilingual model auto-picked from `navigator.language`, ~40 MB model downloaded once and cached, transcription fully on-device. Motivation: Web Speech needs a Google/Apple backend that Brave and distro Chromium don't ship (user-reported).
- `pmos-web/speech.js` rewritten as an engine manager: AudioWorklet mic capture, energy-based endpointing (1.1 s silence ends the utterance; 6 s no-speech / 15 s max guards), interim transcription every 1.5 s for live text, final pass at utterance end; speaking may begin while the model still downloads (audio buffers, progress streams to the palette); Esc discards in-flight results so a cancelled command can't execute late; Web Speech kept as init-failure fallback.
- Platform: transcripts drain before statuses (final text must precede session-end); palette shows engine notes while listening (model download %, "transcribing…"). Kernel/ABI untouched — the engine swap happened entirely behind the platform boundary, as designed.
### Specs
- [[AI System]] §5: Whisper promoted from v2 to default engine; Web Speech demoted to fallback; deferred: model-size toggle in Settings.
- [[Todo]]: voice palette entry updated to the any-browser engine.

## [2026-07-31] — Voice: surface engine failures instead of ending silently
### Code
- Fixed a status race that could swallow the speech engine's error reason: `onerror` + `onend` land in the same frame, and the latest-wins status cell let the clean-end overwrite the error — the platform bridge now queues statuses, and `speech.js` suppresses the generic end after an error.
- `speech.js`: `[pmos-voice]` console diagnostics on every engine event; the `network` error message now names the real-world cause (non-branded Chromium builds ship without Google's speech keys — use Chrome/Edge).
- Palette: a session that ends with no transcript and no error says "🎤 didn't hear anything" — voice never ends silently.

## [2026-07-31] — Voice command palette: 🤙 hold, live transcript, free Web Speech engine
### Code
- ABI 1.5: `VoiceCapture{start}` syscall, `VoiceStatus`/`VoiceTranscript` events, `voice:input` capability (granted to the shell).
- `pmos-kernel`: `VoiceDirectives` (generation-counted, same contract as hands), `voice_status`/`voice_transcript` platform entry points; engine self-termination syncs capture intent without re-dispatch; capability-gating test.
- `pmos-web/speech.js` (new): Web Speech API wrapper — free, zero-download, no key; one utterance per activation, interim results for real-time text, mapped error reasons (mic denied / unsupported / offline); **only text reaches the kernel, audio never does** (mirror of the camera landmarks-only rule).
- Shell: 🤙 tap (< 0.5 s) still toggles the palette; 🤙 **hold ≥ 0.6 s** — a deliberate dwell that can't fire accidentally — opens the palette listening.
- Palette: voice mode with pulsing 🎤 indicator, live interim transcript streaming into the input line, Esc cancel; one routing brain for typed + spoken input — spoken launch verbs ("open the terminal") stripped for app matching, unrecognized speech flows to the System Assistant instead of "unknown command".
### Specs
- [[Hand Gestures]] G8 + §6: tap/hold boundary now 0.5 s / 0.6 s; hold = voice command mode (implemented); voice *notes* remain a follow-up.
- [[AI System]] §5: v1 pipeline marked implemented (engine, ABI surface, routing, text-only boundary); Whisper stays the v2 path.
- [[Todo]]: voice command palette logged under post-release; cursor stabilization marked user-verified.

## [2026-07-31] — Cursor stabilization: no more jumping on pinch/fist
### Specs
- [[Hand Gestures]] §2.1 (new): the researched stabilization design — one articulation-invariant anchor (palm centroid: wrist + MCP knuckles; fingertip anchors rejected on principle since every gesture *is* finger motion), a commit lock with a 12 pt soft radial deadband that engages the instant a pinch starts forming, and ~80 ms release easing; sources cited (MediaPipe palm-stability rationale, Ultraleap TouchFree interaction settings and pinch hysteresis).
- [[Todo]]: new *Post-release — Field-testing stabilization* section; cursor stabilization ticked (user live verification pending).
### Code
- `pmos-kernel/input/hands.rs`: removed the pose-dependent anchor switch (the root cause of the user-reported cursor teleport when pinching/grabbing); cursor now always tracks the palm centroid; new `stabilize()` — hold origin latched from the previous cursor position (seamless entry), sub-deadzone motion fully suppressed, radial-excess follow beyond it (no snap at the boundary), exponential release easing; hold state cleared on tracking loss. 4 new tests → 17 kernel-native tests total.

## [2026-07-31] — Milestone 8: browser app, desktop scaffold, CI and release
### Code
- Browser app: egui chrome + a sandboxed DOM iframe overlaid by the platform on the window's content rect (position synced every frame); embed-refusal caveat surfaced in the UI. Known z-order limitation documented (iframe stacks above the canvas).
- `crates/pmos-desktop`: Tauri 2 scaffold wrapping the same `dist/` bundle — outside the workspace so native GUI deps never break `cargo test --workspace`.
- Release pipeline: `wasm-opt -z` via trunk; the bundle is **6.5 MB wasm / ~6.6 MB total** (51 MB debug).
- `.github/workflows/ci.yml`: tests + wasm check on every push/PR; on master, release build (`--public-url /pseudo-motion-os/`) deployed to GitHub Pages.
### Specs
- [[Demo]] (new): the scripted five-minute golden path with speaker lines and fallbacks.
- [[Running Locally]]: release/Pages/Tauri build instructions; README links the live URL and demo script.
- [[Todo]]: M8 done — the roadmap's core is complete; remaining carried items are annotated in place.

## [2026-07-31] — Milestone 7: physics on the stage and the ray tracer
### Code
- `pmos-kernel/phys`: rapier3d at a fixed 120 Hz with an accumulator; 8 palette-colored cube/sphere props on a floor collider; ray-picking; grab as a critically-damped spring force on a still-dynamic body (collisions resolve mid-grab, release inherits velocity → real throwing); 2 tests.
- `pmos-kernel/gfx`: depth buffer joins the render graph (sky → props → blended floor → egui, egui renderer rebuilt with a depth format); instanced prop pass (generated cube/UV-sphere meshes, per-instance transform+color, Blinn-lambert+rim); `screen_ray` unprojection for picking.
- Ray tracer: Whitted WGSL compute (512×384) — mirror and glass spheres, orbiting diffuse spheres, checkered plane, hard-shadowed point light, iterative reflect/refract, sky gradient — registered as an egui texture and shown in the new ◇ Ray Tracer app with bounce/animate controls (`RtConfig`, ABI 1.4).
- Input routing: closed-hand grab and mouse-drag now try UI → prop → camera-orbit in that order, on both modalities.
### Specs
- [[Todo]]: M7 code complete; notes 3D graph still carried; browser visual pass pending user run (extension disconnected during verification).

## [2026-07-31] — Milestone 6: the virtual file system and real core apps
### Code
- `pmos-kernel/vfs`: kernel-resident POSIX-like tree with write-through persistence ops drained by the platform; `/sys` synthetic files (live fps EMA, abi); parent auto-creation, 4 MB file cap, empty-dir-only deletes; 3 tests. ABI 1.3 adds `FsDelete`/`FsMkdir` and `Bytes`/`Entries` replies.
- **Scoped filesystem capabilities enforced**: `FsRead(scope)`/`FsWrite(scope)` prefix-match the requested path per process; apps get per-kind delegated scopes (Notes: `/notes` only).
- `pmos-web/storage.js`: OPFS mirror — recursive boot load into the kernel, then write/delete/mkdir write-through (main-thread async API; the spec's sync-handle worker was unnecessary at these sizes). Verified: `vfs ready (persistent: true)`.
- Terminal rebuilt as a real syscall client: `ls·cd·cat·write·mkdir·rm·apps·run·fps·clear·about·help` plus `>` NL mode streaming the System Assistant into the log (per-app event routing added to the shell).
- Files app: breadcrumb browser, typed icons, click-to-launch ✨ `.conjure` bundles, delete, new-folder.
- Motion Notes MVP: /notes sidebar, editor+save, daily note, `[[wikilinks]]` with ghost-note creation, backlinks panel.
- Conjured apps now persist to `/apps/<id>.conjure` and are relaunchable from Files, `run`, and across reloads.
### Specs
- [[Todo]]: M6 core done; deferred: Files drag-and-drop/context menus, notes 3D graph (M7), voice capture, IndexedDB fallback, `/sys/processes`, `/sys/ai/log`.

## [2026-07-30] — Milestone 5: LLM integration and app conjuring (core slice)
### Code
- `pmos-conjure` fully implemented: strict document model, the Conjure expression language (precedence parser, templates, 20+ builtins, ternaries, safe division), interpreter with step/if/emit budgets (a runaway emit chain can neither loop nor overflow the stack), 7-stage validator emitting machine-readable errors for the repair loop; pomodoro example doc; 13 tests.
- `pmos-kernel/ai`: agents as kernel objects (System Assistant, App Smith) — the kernel composes full provider requests so API keys never cross into JS as config; Anthropic + OpenAI-compatible bodies; bounded history; `AiConfigure`/`AiPrompt` syscalls; streamed deltas fan out as `AiChunk` events.
- `pmos-web`: `llm.js` fetch+SSE bridge (both provider framings, friendly error chunks); provider config persisted to localStorage and restored at boot.
- `pmos-apps`: **App Host** rendering Conjure documents with live bindings; **command palette** (dock ✨ / Ctrl+K / 🤙 tap) with fuzzy commands, streaming `>` assistant chat, and the `make …` conjuring flow with auto-repair (validation errors + previous document fed back, ≤2 rounds); Settings → AI provider form (masked key); notify effects surface as toasts.
- Verified offline in Chrome: `demo` in the palette spawned the pomodoro through validate → ProcRegister/WinCreate → App Host; timer ticked live and the Start/Pause ternary re-rendered. Live LLM streaming awaits a user API key.
### Specs
- [[Todo]]: M5 core done; deferred to M6+: consent sheets, /apps persistence + modify-existing, voice, budget caps, tool-calling, remaining widget surface (list_view/canvas/dropdown).

## [2026-07-30] — Milestone 4: gesture control
### Code
- `pmos-kernel/input/fusion` (new): pose → pointer-intent state machine — pinch = primary press/release, middle-pinch = secondary, two-finger = scroll (pointer parks, natural direction), grab = whole-hand drag; guaranteed release of held buttons on tracking loss; 4 unit tests.
- `pmos-web`: intents are synthesized into egui pointer events each frame, so hands operate every existing widget (buttons, sliders, titlebars). Grab routes by context: over UI it drags; over the open stage it orbits the camera.
- `pmos-apps`: launcher overlay (UI spec §2.3) — dimmed backdrop + app grid, opened by open-palm hold (0.6 s) or the dock ◆ button, closed by Esc/backdrop/tile launch; click-ripple feedback at the cursor on pinch onset.
- Verified in Chrome with synthetic gestures through the full pipeline: palm-hold opened the launcher, pointing hovered a tile, pinch pressed it; fusion mechanics covered by unit tests. Full-speed click/drag confirmation on live webcam (egui's 0.8 s click window can't be met by remote frame-pumping).
### Specs
- [[Todo]]: M4 done with deferrals annotated (3D object grabbing → M7 with rapier props; swipe/double-palm/thumbs → when workspaces, minimize-all and consent dialogs exist).

## [2026-07-30] — Fix: gesture detection dying permanently after tuning changes (user bug report)
### Code
- **Root cause:** dragging a detection slider sent a `HandsTune` syscall for every intermediate value, each triggering a landmarker rebuild; concurrent async rebuilds in the worker raced (closing/clobbering each other's instances), which could leave the landmarker permanently broken — feed alive, landmarks and gestures dead until reload.
- `pmos-apps/hand_tracker`: tuning syscalls now fire only when the pointer is released — one rebuild per adjustment, not dozens per drag.
- `gesture-worker.js`: rebuilds are strictly serialized — while one build runs, the newest requested config queues and applies after; the initial build participates in the same lock.

## [2026-07-30] — Fix: opening the Hand Tracker froze all hand tracking (user bug report)
### Code
- **Root cause:** opening the viewer sent a `configure` to the gesture worker on every directives change, which rebuilt the MediaPipe landmarker; frames arriving mid-rebuild were dropped *without a reply*, wedging the main thread's `busy` flag forever — tracking, cursor and preview all froze until a full restart.
- `gesture.js`: configure only rebuilds when worker-side tuning (hands/confidences) actually changed — viewer open/close and feed toggles never interrupt tracking; 2 s watchdog recovers a lost reply; the preview stream no longer depends on the worker round-trip.
- `gesture-worker.js`: hard contract — every frame message gets a reply (zero hands when dropped or on detect failure); configure failures report instead of dying silently.

## [2026-07-30] — Camera pipeline fixes (user bug report: "camera always off")
### Code
- **Root cause 1:** MediaPipe's wasm loader calls `importScripts()`, which module workers forbid — the hand-tracking model silently failed to load on every machine. Fixed with a synchronous-XHR + indirect-eval shim in `gesture-worker.js`.
- **Root cause 2:** the capture `<video>` element was never attached to the DOM, so `video.play()` could be rejected ("media was removed from the document"). Now appended invisibly (opacity 0, 2×2 px — never `display:none`, which stalls frames).
- **Failure visibility:** camera errors were silent. `CameraStatus` now carries a reason (ABI 1.2); the Hand Tracker shows it in orange with actionable guidance — permission blocked (padlock instructions), no device, camera in use, model load failure — plus "requesting camera…" / "loading model…" progress states.
- **GPU fallback:** the worker falls back to the CPU delegate when the GPU delegate fails to initialize.
- **Permission re-ask:** the landing page now shows "↺ re-ask permissions on launch" for returning users (clears the saved onboarding state) — fixes "it no longer asks for camera/mic/notifications".
- Verified end-to-end in Chrome with a real webcam: onboarding re-runs after reset; Enable camera → live feed renders in the viewer; tray shows on/no-hands.
### Specs
- Behavior covered by [[UI]] §2.0 (permission control) and [[Hand Gestures]] §8; no spec text changes needed beyond this record.

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
| 2026-08-01 | Launcher removed entirely | Dock + palette cover launching; the open palm is reassigned to the RECORD sign |
| 2026-08-01 | Voice is always-on after onboarding (local Whisper) | Ambient voice OS; on-device transcription keeps always-on ≠ always-uploading; visible ● REC chip is the trust anchor |
| 2026-08-01 | Dynamic gestures = Computer Sign Language (own spec) | Static poses saturated; ASL-inspired motion signs scale the vocabulary; face/eyes layer planned on the same worker |
| 2026-08-01 | Stage boots clean — no scattered demo props | Clunky first impression; physics/grab stays for notes-as-bodies & conjured objects |
| 2026-07-29 | Custom wgpu layer, no Bevy | Full render-graph control (RT compute pass + egui-to-texture); small binary |
| 2026-07-29 | AI apps = interpreted declarative format ("Conjure") | No in-browser rustc; host-enforced sandbox; reliable LLM output; evolvable |
| 2026-07-29 | LLM: remote API first, WebLLM later, one provider abstraction | App generation needs frontier quality; local = backend swap later |
| 2026-07-29 | WebGPU-only, no WebGL2 fallback | WebGL2 lacks compute; WebGPU now mainstream |
| 2026-07-29 | Physics on CPU (rapier); GPU physics = stretch | rapier has no GPU backend; CPU ample for desktop scenes |
| 2026-07-29 | Single-threaded WASM + JS workers in v1 | COOP/COEP conflicts with iframe browsing; workers cover real needs |
| 2026-07-29 | Notes = plain markdown in VFS, Obsidian-compatible | Zero lock-in; Tauri mode can share a real Obsidian vault |
| 2026-07-29 | 🤙 gesture anchors all voice entry (tap=palette, hold=note) | One memorable "talk" gesture; topologically unique for the tracker |
| 2026-07-29 | Rule-based gesture classification (no trained model) | 9 topologically distinct poses; deterministic, tunable, debuggable |
