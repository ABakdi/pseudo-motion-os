# Voice Kit
**Pseudo Motion OS** · part of [[Pseudo Motion OS]] · engine: [[AI System#5. Voice pipeline]] · signs: [[Computer Sign Language]]

The Voice Kit is PMOS's always-on voice layer: the OS **continuously captures and transcribes speech** (once the user grants the mic at onboarding), shows the live transcript in a corner widget, lets a hand sign mark utterances as **commands**, persists everything searchably, and feeds commands to the AI *with context*. Voice stops being a mode you enter and becomes ambient — like the cursor.

---

## 1. The widget (top-right)

A small always-on-top panel, top-right of the overlay:

- **Collapsed** (default): a single status chip — `● REC` pulsing red while capturing · `⏸ paused` · `✕ off` (permission missing) · `⚠` (engine error, reason on hover). The chip is **never hidden while the mic is live** — the trust rule: capture is always visible.
- **Expanded** (click the chip): live transcript stream — interim text in dim ink, finalized utterances in full ink, **commands in the accent color** with a `⌘` marker; a search field over history; per-session actions (**→ note**, copy, clear).
- The widget is shell chrome (like the dock), not an app window: no titlebar, always on top, draggable along the top edge.

## 2. Capture lifecycle

| State | Enter by | Notes |
|---|---|---|
| **capturing** | boot (if mic granted at onboarding) · RECORD sign ✋→✊ · widget click | continuous: Whisper sessions loop — VAD segments utterances, each finalizes into the transcript, the engine restarts immediately |
| **paused** | RECORD sign again · widget click | mic tracks stopped (hardware indicator off) — paused is real, not muted |
| **off** | no mic permission | widget shows `✕ off`, click re-requests |

Engine = the existing on-device pipeline ([[AI System#5]]): Whisper in-browser by default, text-only kernel boundary — **audio never crosses**; transcription is local, so always-on does not mean always-uploading. Continuous mode reuses the endpointing (silence closes an utterance) but auto-restarts instead of stopping.

## 3. Commands inside the stream

- The **COMMAND sign** ([[Computer Sign Language#4]]) — or, fallback, holding 🤙 — marks the *next utterance* as a command. The widget shows a listening-for-command state (accent ring).
- Command utterances render in accent color with `⌘`, are appended to the session's **command list**, and route through the palette brain: app names launch, stage phrases execute instantly, everything else goes to the assistant **with the context envelope (§5)**.
- Non-command speech is transcript only — it never triggers anything (anti-Midas for voice).

## 4. Persistence & search

- Sessions live in the VFS: `/voice/<YYYY-MM-DD>/<HHMMSS>.json` —
  `{ "started": …, "segments": [{ "t": secs, "text": "...", "command": bool }…] }`, written incrementally (crash-safe) through normal `FsWrite` syscalls, so OPFS persistence and the Files app come free.
- **Search**: the widget's search field scans `/voice` (text + commands); the terminal gets `voice <query>`.
- **→ note**: converts a session (or selection) to markdown in `/notes/voice/<date>-<time>.md` — transcript as prose, commands as a checklist — entering the normal notes/backlinks world.
- Retention: sessions kept until deleted (Files app or `rm`); a Settings retention cap is future work.

## 5. Context envelope (AI grounding)

Every voice command sent to the assistant is wrapped:

```json
{ "focused_object": {"index":2,"shape":"cube","color":"#6ee7ff"},
  "stage": "3 objects (2 cubes, 1 sphere)",
  "open_windows": ["Files", "Notes"],
  "recent_transcript": ["…last ~5 non-command lines…"] }
```

— so "make it red", "note that down", "what did I just say?" resolve. The envelope is assembled by the shell (it owns all of this state), prepended to the `AiPrompt` as a context block, and the assistant's tools (including [[AI System#3.1|stage tools]] and the web tools) act on it.

## 6. ABI surface (planned v1)

- `VoiceCapture { start }` gains continuous semantics (platform loops sessions); `VoiceStatus` adds a `capturing` state distinct from per-utterance listening.
- `VoiceTranscript { text, is_final }` unchanged — the shell stamps time + command flag when persisting.
- No new capability: `voice:input` covers it; persistence uses the shell's fs grants.

---

*Changes to this document must be recorded in [[Changelog]].*
