# UI
**Pseudo Motion OS — Specification v0.3** · part of [[Pseudo Motion OS]]

How the user interface works: the shell elements that exist, how they behave, and how the user interacts with them across all input modalities. The rendering machinery behind this is specified in [[Architecture#4.1 Graphics Engine (`gfx`)]]; the gesture vocabulary in [[Hand Gestures]].

---

## 1. The Two Planes

PMOS UI exists on two composited planes:

1. **The Stage (3D plane)** — a persistent 3D space rendered behind everything: a ground plane, ambient props, physics objects, and any windows the user has *pinned into space*. The stage is wrapped in a **distant galaxy**: a procedural starfield/nebula rendered at infinite depth (a fullscreen shader driven by view *rotation only*, so no amount of zooming or moving ever brings it closer — it is the unreachable horizon that gives the space its sense of scale). It drifts and twinkles slowly; it is scenery, not geometry. The stage camera has a default "desk view"; the user can orbit/zoom it (mouse drag/wheel, two-hand gestures) within clamped zoom limits, and there is always a one-action "reset view" (`Home` / double-click empty space).
2. **The Overlay (2D plane)** — classic flat UI rendered on top: floating windows, dock, palette, notifications, cursors. This is where work happens by default; the stage is where things become spatial when it helps (presenting, arranging, playing).

A window can move between planes at any time ("pin to stage" / "bring to overlay"). Because every window renders into its own texture ([[Architecture]]), this is a cheap re-parenting, not a rewrite.

**Why two planes:** pure-3D desktops historically fail because reading and clicking at arbitrary angles is miserable. PMOS keeps productive work flat and effortless, and makes spatiality *optional and purposeful* — the demo magic without the usability tax.

---

## 2. Shell Elements

### 2.0 Boot & Launch experience
The OS is entered through a deliberate, cinematic sequence:

1. **Home page** — a plain HTML/CSS landing page (instant paint, zero WASM): full-viewport hero with the animated PMOS logomark over a starfield backdrop, one-line pitch, and a single **Launch** call-to-action; feature highlights below. If WebGPU is missing, the Launch button is replaced by a friendly capability notice — the landing page itself works everywhere.
2. **Launch** — clicking Launch boots the WASM kernel (progress shown on the button itself), then runs **permission onboarding**: sequential cards for **camera** (hand gestures), **microphone** (voice), and **notifications**, each with a one-line reason and `Enable` / `Later`. Browser permission prompts fire from these explicit clicks (user-gesture requirement); anything skipped is re-requested on first relevant use. Onboarding state persists to `/sys/permissions`, so returning users go straight to the desktop.
3. **Arrival** — a fade from the landing page into the Stage: the galaxy backdrop and the dock. The OS is live.

**Why a landing page at all:** PMOS is also its own advertisement — the first impression is part of the product, and keeping it WASM-free means it loads instantly and degrades gracefully.

### 2.1 Windows
- Standard chrome: title, drag region, minimize / close, resize handles (all corners+edges). Minimum useful hit targets: 32 px (larger tolerance when the pointer source is a hand — see §4).
- States: normal, minimized (to dock), maximized, **pinned-to-stage** (rendered on a quad in 3D, still fully interactive via raycast picking).
- Focus: single focused window receives keyboard/gesture events; focus follows click/pinch; `Alt-Tab` / swipe cycles.
- Windows are owned by processes; closing the last window of a DSL app terminates its process.

### 2.2 The Dock
Bottom-center bar: running apps (with window previews on hover/point-dwell), pinned favorites, and three system items — **Launcher**, **AI palette**, **System tray**. Auto-hides in presentation mode.

### 2.3 The Launcher
Fullscreen-overlay grid of installed apps (built-ins + saved Conjure apps from `/apps`). Opened by: dock button, `Super` key, or *open-palm hold* gesture. Type-to-filter immediately.

### 2.4 The Command Palette *(the centerpiece)*
One unified palette (`Ctrl+K` / dock / 🤙 gesture) with three interleaved modes:
- **Command mode** — fuzzy actions ("close window", "new note", "raytrace quality high").
- **AI mode** (prefix `>` or just speak) — natural language to the system agent: questions, app conjuring, system control. Streaming responses render inline; generated apps open on acceptance. See [[AI System]].
- **Voice mode** — opened by the 🤙 gesture or mic button; shows live transcript, executes on end-of-utterance. The palette is *the* voice surface — voice never acts without its transcript being visible here first (trust through visibility).

### 2.5 System Tray
Top-right cluster: gesture status (camera on/off, hands detected indicator), AI agent status (provider, streaming activity), performance mode (Quality / Balanced / Battery — controls RT budget & physics rate), clock, notifications bell.

### 2.6 Notifications
Toast stack top-right: app messages, AI task completions, capability consent prompts (see §5), voice-note confirmations. Toasts are actionable (buttons) and queue into a history panel.

### 2.7 Context Menus
Right-click / *middle-finger-pinch* opens context menus on windows, files, stage objects. Every context action also exists in the command palette (discoverability rule: **nothing is context-menu-only**).

### 2.8 App launching surfaces
Apps are launched from the **dock** (§2.2) and the **launcher** (§2.3) — one flat, fast surface and one browsable surface. *Design note (2026-07-30): an earlier revision floated the system app icons inside the 3D Stage; this was removed as redundant with the dock — the Stage stays reserved for content (physics objects, pinned windows, notes graph), not chrome.*

---

## 3. Interaction Model

### 3.1 One pointer, many sources
There is exactly **one system pointer**, fed by mouse *or* hand (*point* gesture) — whichever moved last wins, with a short hysteresis to prevent flicker. The pointer carries a source tag; UI adapts (see §4) but apps receive identical events ([[Architecture#4.4 Input Pipeline (`input`)]]).

### 3.2 The modality parity rule
Every action in the system MUST be reachable by (a) mouse/keyboard, (b) one hand, and (c) voice, unless physically nonsensical. This is the **one-hand-is-enough / no-hands-is-enough policy** from [[Pseudo Motion OS#2. Philosophy]] made testable: the spec for any new feature must list its three bindings.

| Action | Mouse/KB | Hand | Voice |
|---|---|---|---|
| Select / click | left click | pinch | "click / select the …" |
| Drag window | drag titlebar | pinch-hold on titlebar | "move … to the left" |
| Grab 3D object | drag | grab (fist) | "put the cube on the table" |
| Scroll | wheel | two-finger drag / palm tilt | "scroll down" |
| Switch desktop | `Ctrl+←/→` | open-hand swipe | "next desktop" |
| Launcher | `Super` | open-palm hold | "open the launcher" |
| Palette / voice | `Ctrl+K` | 🤙 tap | (already voice) |
| Confirm / cancel | Enter / Esc | thumbs-up / thumbs-down | "yes / cancel" |

### 3.3 Text input
Physical keyboard is primary. Voice dictation into any focused text field via palette voice mode. An on-screen keyboard is **post-v1** (gesture typing is deliberately out of scope — it is a research problem and off the demo path).

### 3.4 The Stage interactions
- **Picking:** pointer ray from camera through cursor; hover highlights; pinch/click selects.
- **Grab physics objects:** *grab* attaches the object kinematically to the hand with a spring (so collisions still resolve); release inherits hand velocity → throwing works naturally.
- **Two-hand:** spread/pinch scales the selected object or zooms the camera; two-fist rotate orbits it. See [[Hand Gestures#Two-hand enhancers]].
- **Pinned windows** face the camera by default (billboard) unless the user rotates them intentionally.

### 3.5 Desktops (workspaces)
Virtual desktops each with their own overlay window set but **sharing one stage**. Swipe / `Ctrl+arrows` to switch. Presentation mode is a special desktop: dock hidden, notifications muted, gesture cursor rendered large and high-contrast for the audience.

---

## 4. Pointer-Source Adaptation

When the active pointer source is a hand:
- Hit targets gain a +8 px tolerance ring; small controls magnify on hover-dwell.
- Dwell (800 ms) acts as hover; dwell-progress ring shown.
- Snap: near-window-edge drags snap to halves/quarters (more aggressive snapping than with a mouse).

**The morphing cursor.** With a hand as the source, the cursor is not an arrow — it is a live glyph that mirrors the recognized pose ([[Hand Gestures#3]]), so the user always sees what the system sees:

| Hand state | Cursor form |
|---|---|
| Rest / Point | open ring with a center dot |
| Pinch forming | ring tightens continuously with pinch confidence (pre-touch feedback — you feel the click coming) |
| Pinch (click/drag) | ring closed to a solid dot; drag draws a short motion trail |
| Grab ✊ | fist glyph; grabbed object highlighted and tethered by a faint line |
| Open palm | palm bloom (radial pulse) counting down the launcher hold |
| 🤙 Call sign | microphone glyph; pulsing red while a voice note records |
| Thumbs up/down | ✓ / ✕ badge flashed at the focused dialog |
| Hover over interactive element | ring gains a soft halo |
| Tracking lost | cursor frozen, dimmed, slow blink until the hand returns |

Cursor forms are drawn in the overlay pass at 60 FPS from recognizer state — never from raw landmarks — so the glyph is always consistent with what the input pipeline will actually do.

When source is mouse: standard precise cursor, no magnification, no morphing.

---

## 5. Consent & Safety UI

Capability requests ([[Architecture#4.7 Process & Capability Manager (`proc`)]]) surface as a **consent sheet**: which app, which capability, why (the app's declared reason), Allow once / Always / Deny. AI-initiated system actions above a risk threshold (file deletion, network, closing unsaved work) show an **action preview toast** with undo where possible. Nothing AI-driven touches the filesystem destructively without either a standing grant or a visible confirmation.

---

## 6. Visual Design

- **Theme:** dark, high-contrast default (projector-friendly for the teaching use case); light theme included; both defined as egui style tokens in one place.
- **Depth cues:** overlay windows carry soft shadows; stage lighting is warm-neutral; the ray-traced showcase objects are the visual jewelry, not the whole room.
- **Motion:** 150–250 ms ease-out for all shell transitions; physics provides the rest of the life. Reduced-motion setting disables nonessential animation.
- **Feedback for gestures:** every recognized gesture flashes a small glyph near the cursor (👌, ✊, 🤙…) — instant confirmation of *what the system saw*, which is the single most important trust feature of camera input.

### 6.1 Appearance settings *(implemented 2026-07-31)*

Settings → Appearance, applied live and persisted to `/settings/appearance.json` in the VFS (the shell re-applies it at boot):

- **Backgrounds** — four sky-shader presets selected per-frame via a sky uniform (`Background` syscall, ABI 1.7): **Deep Space** (ion blue/violet nebulae — the default) · **Ember Nebula** (warm dust) · **Aurora** (green/teal curtains) · **Void** (near-black, sparse dim stars — the projector/minimalist option).
- **Color schemes** — accent-pair palettes applied across every shell surface (cursor ring, dock, palette, selection, hover): **Ion** (cyan/violet) · **Ember** (amber/rose) · **Verdant** (green/sky) · **Rose** (pink/violet). The scheme is a runtime token (`theme::accent_a()/accent_b()`), so switching restyles everything instantly. *(The spec'd light theme remains future work.)*
- **Text is selectable everywhere** (`selectable_labels`), and Ctrl+C copies to the real system clipboard — the platform mirrors egui copy commands through `navigator.clipboard` (egui's wasm clipboard is internal-only).
- App windows are resizable (including panel splitters, e.g. the Notes sidebar); long content scrolls (Settings, Terminal, Files, Notes).

---

## 7. Accessibility

- Modality parity (§3.2) is the backbone: no gesture-only or voice-only functions.
- All egui UI is keyboard-navigable; focus outlines always visible.
- Adjustable: gesture hold-times, dwell times, cursor size, UI scale, high-contrast mode.
- Voice control covers the full command set (palette command grammar = voice grammar).

---

*Changes to this document must be recorded in [[Changelog]].*
