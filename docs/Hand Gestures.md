# Hand Gestures
**Pseudo Motion OS — Specification v0.3** · part of [[Pseudo Motion OS]]

The complete gesture vocabulary: what each gesture does, why it was chosen, and the ergonomic and technical rules that make camera-based hands pleasant instead of exhausting. Consumed by the input pipeline ([[Architecture#4.4 Input Pipeline (`input`)]]); UI feedback rules in [[UI#4. Pointer-Source Adaptation]].

---

## 1. Design principles

1. **One hand is enough.** Every essential function is operable with a single hand (either hand — handedness is a setting, default auto-detect). Two-hand gestures only *enhance* (precision, scale, rotate); they never gate a feature.
2. **Easy on the hand.** No gesture requires: sustained mid-air holds > 2 s, fully splayed fingers under tension, wrist extremes, or arms raised above shoulder level. The tracking volume is centered at *relaxed desk/lap height* — "gorilla arm" is designed out, not warned about.
3. **Relaxed is neutral.** A resting, half-open hand is the *no-op pose*. Every active gesture is a deliberate departure from rest, so noise while thinking/talking (crucial for the teaching use case) triggers nothing.
4. **Distinct under a webcam.** Gestures are chosen to be separable from MediaPipe's 21 landmarks under mediocre lighting at 640×480: they differ in *topology* (which fingertips touch / which fingers are extended), not in subtle angles.
5. **Feedback within 100 ms.** Every recognition flashes its glyph at the cursor ([[UI#6. Visual Design]]). Users must always know what the system saw.
6. **Hysteresis everywhere.** Enter and exit thresholds differ (Schmitt trigger) so a gesture near its boundary never flickers.
7. **Small vocabulary, deep coverage.** 9 core gestures + 4 two-hand enhancers. Fewer things to learn beats more things to demo.

---

## 2. Recognition pipeline

```
camera (getUserMedia, main thread — unavailable in workers)
  → ImageBitmap transfer → gesture worker
  → MediaPipe HandLandmarker (21 landmarks × ≤2 hands, GPU delegate, ~30 fps)
  → One-Euro filter per landmark (jitter removal, low latency)
  → feature extraction (fingertip distances, finger extension states, palm normal, velocity)
  → static pose classifier (rule-based on features — deterministic, debuggable)
  → temporal layer (state machine: pose + hold-time + motion → gesture events)
  → kernel input events (source-tagged, fused with mouse/voice)
```

- **Rule-based, not ML classification**, for the pose layer: 9 topologically distinct poses need no trained classifier, and deterministic rules are tunable per-user and explainable when a gesture misfires. Raw landmarks remain available to userland apps via `input:raw-hands` capability for experiments.
- **Coordinate mapping:** the cursor anchor (§2.1) maps to screen space through an adjustable *control box* (a subregion of camera view ≈ 40 cm × 25 cm of real space) — small comfortable hand motions cover the whole screen.
- **Confidence gating:** below landmark-confidence threshold, gestures freeze rather than glitch (cursor holds position; a subtle "tracking lost" hint appears after 500 ms).

### 2.1 Cursor stabilization *(added 2026-07-31 after field testing)*

**The failure this section exists to prevent:** if the cursor anchor is a fingertip — or worse, *switches* landmarks per pose — then performing any gesture moves the cursor, because a gesture IS finger motion. The cursor teleports exactly at the moment of commit, defeating pointing entirely. (An early PMOS build anchored Point/Pinch to the index tip and everything else to the palm; pinching or making a fist jumped the cursor between the two anchors.)

Three layers, applied in order:

1. **One articulation-invariant anchor — never switched.** The cursor tracks the **palm centroid**: the mean of the wrist (0) and the four finger-base knuckles (MCP 5/9/13/17). These joints barely move under finger flexion — the same property MediaPipe exploits by detecting *palms*, "informed by the stability of palm region appearance under joint rotation and flexion." Fingertip anchors are rejected on principle, not tuned around.
2. **Commit lock (soft deadband).** The instant a commit gesture starts *forming* — pinch-distance hysteresis latching, or a stateful pose (Pinch / MiddlePinch / Grab) becoming active — the cursor's current position becomes the **hold origin**. While held: hand motion within `HOLD_DEADZONE` (12 pt) is fully suppressed (the click lands where the user aimed); motion beyond it follows as `origin + delta·(1 − R/|delta|)` — a radially *soft* boundary with no snap, so pinch-drags and window-grabs still track naturally. This is the same commit-point stabilization industry deployments use (Ultraleap TouchFree's cursor stabilization + pinch hysteresis).
3. **Release easing.** On gesture exit, the residual offset between the held output and the filtered position decays exponentially (~80 ms time constant) — releasing never jumps either.

One-Euro smoothing (§2) remains the base layer beneath all three. Tuning: `HOLD_DEADZONE` 12 pt · release ease τ 80 ms · pinch latch enter/exit from §6.

*Sources:* [MediaPipe Hands palm-stability design](https://www.emergentmind.com/topics/mediapipe-hands) · [Ultraleap TouchFree interaction settings](https://docs.ultraleap.com/TouchFree/touchfree-user-manual/interaction-settings.html) · [Ultraleap pinch hysteresis guidance](https://docs.ultraleap.com/xr-and-tabletop/xr/unity/plugin/features/pinch-and-grab-detection.html) · field reports of pinch-drift with fingertip anchors ([example](https://chernando.com/blog/2023/07/23/hand-tracking-for-mouse-input.html)).

---

## 3. Core vocabulary — one hand

| # | Gesture | Pose (topology) | Action | Why this mapping |
|---|---------|-----------------|--------|------------------| 
| G1 | **Point** ☝️ | index extended, others curled | Move the system pointer | The universal deictic gesture; zero learning. |
| G2 | **Pinch** 👌 | thumb–index fingertips touch | Primary select / click; **pinch-hold + move = drag** | Highest-precision fingertip event a webcam can see; maps to "pick precisely". Enter < 25 mm, exit > 40 mm (hysteresis). |
| G3 | **Middle pinch** | thumb–middle fingertips touch | Secondary click / context menu | Same motor pattern as G2, adjacent finger → trivially learnable as "the other click". |
| G4 | **Grab** ✊ | all fingers curled to fist | Grab window / 3D object (kinematic spring attach); release = drop, release-with-motion = **throw** | Whole-hand closure for whole-object manipulation mirrors real grasping; velocity inheritance makes throwing feel free. |
| G5 | **Open-palm hold** ✋ (0.6 s) | all five extended, palm to camera | Open the **Launcher** | "Show me everything" — palm-out is a natural stop/attention pose; the hold prevents accidental fires while gesturing in speech. |
| G6 | **Swipe** 🖐️→ | open hand, lateral motion > 0.7 m/s | Switch virtual desktop (left/right); in scrollable context: page | Broad motion for broad navigation; the velocity floor separates it from repositioning. |
| G7 | **Two-finger drag** ✌️+move | index+middle extended, move vertically | Scroll focused view | Direct steal from trackpad muscle memory. Palm-tilt scroll is an optional alt mode in settings. |
| G8 | **Call sign** 🤙 | thumb+pinky extended, others curled | **Tap (< 0.5 s): toggle the command palette.** **Hold (≥ 0.6 s): voice command mode** — palette opens listening, the transcript streams live into the input line, end of utterance executes it *(implemented 2026-07-31)*. The dwell makes the voice trigger impossible to hit by accident. Voice **notes** (long dictation → notes inbox, [[Notes System#Voice capture]]) remain a follow-up on this same anchor. | The "call me" sign is the most iconic "talk" gesture that exists; tap-vs-hold gives palette and voice capture one memorable anchor. Topologically unique (only pose using the pinky alone with thumb). |
| G9 | **Thumbs up / down** 👍👎 (0.6 s hold) | thumb extended up/down, fist otherwise | **When a dialog/consent sheet is focused:** confirm / dismiss. **Otherwise (v1 stage binding, implemented 2026-08-01):** 👍 drops a cube onto the stage · 👎 removes the newest object — grab what you dropped with ✊ and throw it. Dialogs take precedence once consent sheets land. | Universally understood judgment pair; used constantly in the AI-approval flow ([[UI#5. Consent & Safety UI]]); add/remove is the natural idle meaning. |

**Deliberately absent:** finger-counting menus (undiscoverable), air-tap without pinch (no tactile self-confirmation), wrist-rotation dials (fatiguing), face/head input (out of scope).

## 4. Two-hand enhancers

Never required; always a superset of a one-hand or mouse path.

| # | Gesture | Pose | Action | One-hand fallback |
|---|---------|------|--------|-------------------|
| T1 | **Spread / squeeze** | two pinches (G2×2), change inter-hand distance | Zoom camera; or scale selected object/window | Scroll-zoom; corner-drag resize |
| T2 | **Two-fist rotate** | two grabs (G4×2), orbit around midpoint | Rotate selected 3D object / orbit stage camera | Context menu "rotate" + drag |
| T3 | **Double palm push** | two open palms, push toward camera | Minimize all → show the Stage ("clear my desk") | Palette: "show desktop" |
| T4 | **Frame** 📷 | two L-shapes (thumb+index) forming corners | Region screenshot → saved to `/home/captures`, toast with copy/annotate | Palette: "screenshot" |

## 5. Gesture → system routing

- G1–G3, G7 are **pointer-layer** gestures: they become standard pointer events any app receives ([[UI#3.1 One pointer, many sources]]).
- G4 is contextual: on a 3D object → physics grab; on a window titlebar → window drag; elsewhere → stage camera pan.
- G5, G6, G8, G9, T3, T4 are **shell-reserved** — apps cannot rebind them (consistency beats flexibility; this is also the anti-abuse rule so a DSL app can't hijack the voice palette).
- Raw landmark streams (both hands, 21×3 floats @ 30 Hz) are available to apps holding `input:raw-hands` — the escape hatch for gesture research apps, granted via consent sheet.

---

## 6. Tuning parameters (defaults; all user-adjustable in Settings → Gestures)

| Parameter | Default | Range |
|---|---|---|
| Pinch enter / exit distance | 25 mm / 40 mm | 15–60 |
| Open-palm launcher hold | 0.6 s | 0.3–1.5 |
| 🤙 tap vs. hold boundary | 0.5 s / 0.6 s | fixed ratio, scalable |
| Swipe velocity floor | 0.7 m/s | 0.4–1.2 |
| Dwell-as-hover | 800 ms | 400–2000 |
| Control box size | 40×25 cm | resizable in calibration UI |
| Cursor smoothing (One-Euro) | β=0.007, mincutoff=1.0 | presets: Precise / Balanced / Smooth |
| Handedness | auto | left / right / auto / both |

First-run includes a 60-second **calibration & tutorial**: control-box fit, pinch distance calibration (hand sizes differ), then each core gesture practiced once with live feedback.

---

## 7. Failure & fatigue behavior

- **Tracking lost:** cursor freezes (never jumps), hint after 500 ms, mouse takes over instantly if moved (source fusion).
- **Ambiguity:** unresolved poses do nothing (rest-is-neutral). Misfire correction: any gesture-initiated action within the toast window is undoable ([[UI#5. Consent & Safety UI]]).
- **Fatigue guard:** if hands are actively controlling for > 10 continuous minutes, a subtle rest suggestion appears (dismissable, off by default in presentation mode).
- **Privacy:** camera frames exist only in the capture element and the gesture worker's inference call; only landmarks cross into the kernel, and nothing camera-derived is ever stored or transmitted ([[Architecture#3. Layer 1 — Platform Bridge]]). A status indicator (off / on / tracking) lives in the system tray at all times.

---

## 8. The Hand Tracker app

A built-in dock app (✋) — the gesture system's viewer, tuner, and debugger. Its window opens bottom-right, above the dock.

- **Viewer** — the live camera preview (mirrored selfie view) with the hand skeleton overlaid (21 points + bones per hand, one accent color per hand). Two independent toggles:
  - *Show camera feed* — off = **privacy mode**: landmarks drawn on plain black; the platform stops streaming preview pixels entirely (not merely hiding them).
  - *Show hand landmarks* — off = clean camera preview.
- **Status line** — live recognized pose, hand count; when the camera is off, an *Enable camera* button re-requests permission from a real user click (covers the "Later" path from onboarding).
- **Detection settings** — hands (1–2), MediaPipe detection/tracking confidence, cursor smoothing preset (Precise/Balanced/Smooth), pinch enter/exit thresholds; reset to the spec §6 defaults. Worker-side values rebuild the landmarker; recognizer values apply instantly.
- **Plumbing (Architecture §3/§6):** preview pixels flow platform → shell texture and **never pass through the kernel**; landmarks reach the shell as `RawHands` events gated on the viewer being open and on the `InputRawHands` capability, which the shell **delegates** to the app process at registration (ABI 1.2 delegation rule: a process may grant only capabilities it holds itself). Tuning flows back through `HandsTune` syscalls.

---

*Changes to this document must be recorded in [[Changelog]].*
