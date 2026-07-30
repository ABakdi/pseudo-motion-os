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
