# Notes System
**Pseudo Motion OS — Specification v0.3** · part of [[Pseudo Motion OS]]

"**Motion Notes**" — the built-in, Obsidian-style linked note system. Notes are first-class OS citizens: plain markdown files in the VFS, linked into a graph, capturable by voice from anywhere, and readable/writable by AI agents under capability control.

---

## 1. Why notes belong in the OS

The flagship use cases ([[Pseudo Motion OS#3. Use Cases]]) are thinking-out-loud activities: teaching, explaining, creating. Thought capture must therefore be *zero-friction and system-wide* — one gesture away at all times ([[Hand Gestures]] G8-hold), not an app you must find, open, and focus first. Making notes an OS service (not just an app) also gives the AI a grounded, local knowledge base to work with.

---

## 2. Data model

- **A note is a markdown file** under `/notes/`, UTF-8, extension `.md`. No proprietary format, ever — the vault must be trivially exportable (and in Tauri mode, mountable directly into a real Obsidian vault via `/mnt`).
- **Frontmatter** (optional YAML): `title`, `created`, `tags: []`, `source` (`manual` | `voice` | `ai`), plus arbitrary user keys.
- **Wikilinks:** `[[Note Name]]` and `[[Note Name|alias]]` link notes. Links to nonexistent notes are valid ("ghost notes") and render dimmed; opening one creates the note — this is the Obsidian idiom and it is what makes linking cheap.
- **Tags:** `#tag` inline or in frontmatter.
- **Folders** are allowed but secondary; the graph and links are the primary organization.
- **Special folders:**
  - `/notes/inbox/` — voice captures and quick notes land here untriaged.
  - `/notes/daily/YYYY-MM-DD.md` — daily note, auto-created on first use each day.

## 3. Indexing

A kernel-adjacent **note index** service (running in the Notes process, rebuilt incrementally from `FsWatch` events — see [[Architecture#4.6 Virtual File System (`vfs`)]]) maintains:
- forward links & **backlinks** per note,
- tag index, full-text search index (simple trigram/inverted index — no external deps),
- unresolved ("ghost") link set.

The index is a cache: derivable from files alone at any time. Corruption = delete and rebuild.

---

## 4. UI

The Notes app is a built-in userland app ([[Architecture#5. Layer 3 — Userland]]) with three views:

1. **Editor** — split or toggle markdown edit/preview (egui text editor v1: plain markdown with syntax highlight; WYSIWYG is out of scope). `[[` triggers link autocomplete from the index. Backlinks panel at the bottom of every note.
2. **Graph view** — *the showcase*: notes as nodes in the **3D Stage**, links as edges, force-directed layout using the physics engine itself (`rapier` springs — the OS's own physics lays out your knowledge). Grab a node (✊) to drag it, pinch to open it, ghost notes shown translucent. A classic 2D graph mode exists for practicality.
3. **Search / quick switcher** — `Ctrl+O` in-app, and note search is also folded into the global command palette ("open note …").

## 5. Voice capture

The system-wide capture path (see [[Hand Gestures]] G8):

```
🤙 hold ≥1.2 s  → capture HUD appears (live transcript, red dot)
speak…          → streaming STT ([[AI System#Voice pipeline]])
release 🤙      → transcript saved to /notes/inbox/<timestamp>.md
                  (frontmatter: source: voice, created, duration)
                → toast: "Note captured" [Open] [Append to daily] [Discard]
```

- Capture works **from anywhere** — mid-presentation, over any app — without stealing window focus.
- If an AI agent with `notes:link` capability is configured, it may post-process inbox captures: title suggestion, `[[link]]` suggestions to existing notes, tag suggestions — always written as *suggestions block* in the note, never silent rewrites ([[AI System#Safety]]).

## 6. AI integration

Agent capabilities over notes are scoped and consented like all capabilities ([[Architecture#4.7 Process & Capability Manager (`proc`)]]):

| Capability | Grants |
|---|---|
| `notes:read` | Full-text read of `/notes/**` for context (Q&A over your vault: "what did I write about shaders?") |
| `notes:write` | Create/modify notes — every AI edit is marked (`source: ai` or an edit annotation) |
| `notes:link` | Inbox post-processing: titles, links, tags as suggestion blocks |

Typical flows: summarize a long note, "turn this voice ramble into bullet points", auto-suggest links after capture, generate a study-guide note from a set of tagged notes.

## 7. Sync & portability

- v1: local-only (OPFS). Export = zip of `/notes` (one palette command).
- Tauri mode: the vault can live directly on the native FS under `/mnt/...` — then it *is* an Obsidian vault; PMOS and Obsidian can be used on the same files.
- Cloud sync is explicitly **out of scope** (philosophy: local and transparent).

---

*Changes to this document must be recorded in [[Changelog]].*
