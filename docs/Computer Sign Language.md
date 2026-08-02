# Computer Sign Language (CSL)
**Pseudo Motion OS** · part of [[Pseudo Motion OS]] · sibling of [[Hand Gestures]]

CSL is PMOS's motion-sign vocabulary: **signs are words**, not buttons. Where [[Hand Gestures]] defines static poses (what the hand *is*), CSL defines **dynamic signs** (what the hand *does over time*) — and, in M10, what the face and eyes add. It is *inspired by* American Sign Language, borrowing individual sign mechanics where they are iconic and camera-recognizable. It is explicitly **not ASL**: no grammar, no claim of equivalence, and signs are adapted for single-camera recognition. (Respect note: ASL is a full natural language; CSL borrows isolated mechanics with attribution, the way keyboard shortcuts borrow letters.)

---

## 1. Why signs, not more poses

The static pose alphabet (9 poses) is saturated — every new feature was competing for the same shapes. Motion signs multiply the vocabulary without adding poses: the same ☝ Point means *cursor* when still and *COMMAND* when it performs the chin-outward motion. Signs also carry **intent** the way words do, which is exactly what an always-listening voice OS needs: the hand marks *what kind* of thing the voice is about to say.

## 2. Anatomy of a sign — the four parameters

Borrowed from sign-language phonology, mapped to what one webcam can track:

| Parameter | ASL meaning | PMOS signal |
|---|---|---|
| **Handshape** | finger configuration | the existing pose classifier ([[Hand Gestures#3]]) |
| **Location** | where the sign happens | control-box region now; **face-anchored** (chin, mouth, brow) once the face mesh lands (§6) |
| **Movement** | trajectory | palm-centroid path + z (toward/away from camera), sampled per frame |
| **Orientation** | palm facing | derivable from landmark geometry (wrist→MCP plane normal) |

## 3. Recognition engine

A **sign FSM layer** above the pose stream, beside (not replacing) the pointer pipeline:

```
landmarks → pose classifier ─→ pointer/fusion (cursor, pinch, grab…)
                    └────────→ sign engine: per-sign state machines
                                stages = (pose, region, motion primitive) + time window
```

- **Motion primitives**: `HOLD(pose, ms)` · `SQUEEZE(open→fist)` · `FLICK_OUT` (crisp forward/outward, z-velocity above threshold — crispness is semantic: ASL distinguishes COMMAND from "tell" by firmness) · `PUSH` (palm toward camera) · `ARC_7` (outward then down, the '7' path).
- **Midas-touch guards**: every sign starts from a held entry pose (≥ 0.25 s); a sign in progress suppresses pointer intents from that hand; completed signs flash their glyph at the cursor (the [[UI#6]] trust rule) and have a refractory period (0.5 s).
- **Confidence**: mid-sign tracking loss aborts silently — a half-sign must never fire.

## 4. v1 vocabulary

| Sign | Mechanics | Meaning | Inspiration |
|---|---|---|---|
| **RECORD** | ✋ open palm held ≥ 0.3 s → close to ✊ fist within 0.8 s (a "squeeze") | Toggle Voice Kit capture on/off ([[Voice Kit]]) | The palm "grabs the air" — catching sound |
| **COMMAND** | ☝ '1' hand held in the upper-center region (chin-anchored after M10) → crisp outward/down `FLICK_OUT` | The next utterance is a **command**, not transcript — highlighted, listed, routed to the AI with context | ASL COMMAND/ORDER: index at chin moving firmly outward ([PocketSign](https://www.pocketsign.org/asl/command), [Lifeprint](https://www.lifeprint.com/asl101/pages-layout/testfirstonehundredsigns.htm)) |
| **CANCEL** | ✋ palm `PUSH` toward camera | Esc-equivalent: cancel listening command, close focused overlay | The universal "stop" push |
| *(reserved)* | NOTE, SEARCH, SAVE, UNDO | future vocabulary, added one at a time with user testing | ASL-inspired where iconic |

Removed binding: the **launcher** (open-palm hold) is gone entirely — user decision 2026-08-01; apps launch from the dock and palette. The palm now belongs to RECORD.

## 5. Two-handed 3D grammar (stage control)

Roles, not duplicates: the **dominant hand points/selects**, the **non-dominant hand modifies**. (Handedness set in Settings; default = first hand seen.)

| Layer | Gesture | Effect |
|---|---|---|
| Select | dominant 👌 pinch-hold on a stage object (0.3 s) | object becomes **focused**: outlined, named in the Voice Kit context, target of property control |
| Move | dominant ✊ grab (existing) | drag / throw with physics (unchanged) |
| Rotate object | object focused + non-dominant ✊ fist twist (wrist roll) | rotates the focused object |
| Scale object | object focused + non-dominant ✌ vertical drag | scales the focused object (clamped) |
| Camera zoom | **two ✋ palms** move apart / together | zoom out / in (pinch-the-world) |
| Camera orbit | ✊ over empty space (existing, single hand) | orbit |
| Deselect | CANCEL sign, or pinch empty space | clears focus |

The **focused object is context**: every AI command carries it (see [[Voice Kit#5]]), so "make it red", "spin this", "delete that" resolve naturally.

## 6. Face layer & eyes (M10)

MediaPipe **FaceLandmarker** runs beside the HandLandmarker in the same tasks-vision worker: [478 3-D landmarks + 52 expression blendshapes + head transform](https://ai.google.dev/edge/mediapipe/solutions/vision/face_landmarker/web_js), landmarks-only across the kernel boundary (the [[Hand Gestures#7]] privacy rule extends unchanged: frames never leave the platform).

- **Face-anchored signs**: COMMAND gains its true chin anchor; future signs can use mouth/brow regions.
- **Face gestures** (blendshape thresholds + debounce): brow-raise = confirm the focused dialog · double-blink = alternative click for motor accessibility · mouth-"O" = push-to-talk without hands. Each is opt-in in Settings (faces move constantly; defaults conservative).
- **Gaze (experimental, honest limits)**: iris landmarks + head pose give **coarse gaze regions** (roughly which third/quadrant of the screen), not pixel accuracy, without per-user calibration. v1 use: *gaze assists focus* — look at a window and it soft-highlights; the hand cursor remains the precision instrument. A calibrated 5-point gaze cursor is a research track, not a promise.

## 7. Modality parity

Every sign keeps a boring equivalent: RECORD = Voice Kit click · COMMAND = `>` prefix / Ctrl+K · CANCEL = Esc · object rotate/scale = context menu + drag. Camera control parity already exists (mouse orbit/zoom/pan).

---

*Changes to this document must be recorded in [[Changelog]].*
