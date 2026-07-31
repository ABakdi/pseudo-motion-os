# Demo — The Golden Path
**Pseudo Motion OS** · part of [[Pseudo Motion OS]]

The scripted five-minute showcase. Everything below uses shipped features only. Prerequisites: a WebGPU browser, a webcam, an Anthropic API key entered once in Settings → AI.

---

## 0. The landing *(30 s)*
Open the URL. Let the starfield and the orbiting logomark breathe for a beat — the landing page **is** part of the product. Press **LAUNCH**, grant camera (and mic if asked). The page dissolves into the Stage: the unreachable galaxy, the holographic grid, colored physics props resting mid-stage.

**Say:** "Everything you're about to see runs in this browser tab — Rust compiled to WebAssembly, rendered with WebGPU. Nothing is installed, nothing leaves this machine except the AI calls."

## 1. Hands *(60 s)*
Raise a hand. Point — the ring cursor follows. Pinch — the ring tightens and clicks. Open the **Hand Tracker** (✋ in the dock): show the live skeleton, then untick *Show camera feed* — landmarks on black.

**Say:** "The camera never leaves this machine — the OS only ever sees these 21 points. Watch the cursor: it always shows what the system sees."

## 2. Physics *(45 s)*
Make a fist over a cube. Drag it — knock the pile over. Release mid-swing to **throw**. Fist over empty space orbits the galaxy; wheel zooms (and never reaches it).

**Say:** "These are real rigid bodies — 120 Hz simulation. Grabbing is a spring, so collisions still resolve while I hold it."

## 3. The conjuring *(90 s — the centerpiece)*
Open the palette (✨ or 🤙 tap). Type or dictate:
> **make me a tip calculator with a slider for the percentage**

Watch it stream, validate, repair if needed, and open as a window. Use it — with the mouse, then with pinches. Then:
> **make me a quiz about the solar system with 3 questions**

**Say:** "The model writes a sandboxed app definition — it can't touch files, network, or anything else without a grant. It failed validation? Watch: the errors go back and it fixes itself. These apps are saved — they'll still be in /apps tomorrow."

## 4. The OS underneath *(60 s)*
Terminal (🖥): `ls /apps` — the conjured apps are files. `cat /sys/fps`. `> what can you see in this OS?` streams the assistant. Files (📁): click a ✨ bundle — it relaunches. Notes (📝): create a note, type `[[ideas]]`, click the ghost link — it creates the note; show the backlink.

**Say:** "Real filesystem in the browser's origin-private storage, real capability checks per process — Notes literally cannot read anything outside /notes."

## 5. The flourish *(30 s)*
Open the **◇ Ray Tracer**: glass sphere orbiting a mirror, real-time Whitted tracing in a compute shader. Then stand back, open palm → launcher, and close with the wordmark visible.

**Say:** "Ray tracing, physics, hand tracking, an LLM shell — one browser tab, one Rust binary. This is what the web can be."

---

*Fallbacks:* no webcam → mouse does everything (modality parity); no API key → `demo` in the palette spawns the canned pomodoro; site refuses to iframe in Browser → use wikipedia.org.
