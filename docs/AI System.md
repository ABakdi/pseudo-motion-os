# AI System
**Pseudo Motion OS — Specification v0.3** · part of [[Pseudo Motion OS]]

How AI works in PMOS: the agent model, multi-provider support (remote API keys and local LLMs), how agents interface with the kernel, the app-generation workflow, the voice pipeline, and safety. The kernel-side owner is the AI Agent Manager ([[Architecture#4.5 AI Agent Manager (`ai`)]]).

---

## 1. Model: agents as kernel objects

An **agent** is a kernel-managed object: `AgentId` + configuration + conversation state + capability set. Agents are *not* processes themselves; they are services that processes (and the shell) talk to via syscalls (`AiPrompt`, `AiSpawnAgent`, stream events). Multiple agents exist concurrently, each with its own provider, system prompt, and permissions.

**Built-in agent roles** (instantiated from templates, user-editable):

| Agent | Role | Default capabilities |
|---|---|---|
| **System Assistant** | The palette's `>` mode: answer questions, control the system, orchestrate | `sys:query`, `win:manage`, `proc:spawn-app`, `notes:read` (asks for more) |
| **App Smith** | Conjure-app generation & repair ([[App DSL]]) | `proc:spawn-app` only — it writes documents, it does not touch the system |
| **Note Assistant** | [[Notes System#6. AI integration]] flows | `notes:read`, `notes:link` |
| *(user-defined…)* | Any custom role/system prompt | chosen at creation via consent sheet |

**Why multiple agents instead of one:** separation of privilege (the app generator needs no filesystem access; the note assistant needs no window control), separation of context (conversations don't pollute each other), and it makes "agents" a real, demonstrable OS primitive rather than a chat window.

---

## 2. Providers

All providers sit behind one kernel trait; agents bind to a **provider profile** by name. Adding a provider never touches agent logic.

```rust
trait LlmProvider {
    fn chat(&self, req: ChatRequest) -> EventStream<ChatChunk>;  // streaming always
    fn capabilities(&self) -> ProviderCaps;  // tools? vision? max context?
}
```

| Profile kind | Transport | Notes |
|---|---|---|
| **Anthropic** | direct browser `fetch` (CORS opt-in header `anthropic-dangerous-direct-browser-access`) | Recommended default for App Smith — generation quality is the bottleneck for the whole conjuring experience. |
| **OpenAI-compatible** | `fetch` to any base URL | Covers OpenAI, OpenRouter, Groq, and **any local server speaking the protocol (Ollama, LM Studio, llama.cpp server)** — this one profile is how self-hosted local models arrive for free, especially in Tauri mode (no CORS pain on localhost). |
| **WebLLM (in-browser)** — **THE DEFAULT** *(implemented 2026-07-31, ABI 1.6 kind 2)* | `webllm.js`, WebGPU inference (PMOS requires WebGPU anyway), library lazy-loaded from CDN on first use | **Free, no key, works out of the box** — AI is alive on first launch with zero setup. Model downloads once and is cached; then fully offline, prompts never leave the machine. Three performance tiers in Settings → AI: Fast (Qwen2.5-0.5B, ~0.6 GB) · Balanced (Llama-3.2-1B, ~0.9 GB, the default) · Quality (Qwen2.5-3B, ~1.9 GB). Download progress streams into the palette via `'\r'`-replace AiChunk deltas (ABI 1.6) so it never pollutes conversation history. Honest note: small models handle chat/tools well; for App Smith conjuring quality, the Quality tier or a remote provider works markedly better. |

**Key & profile management:** provider profiles live at `/sys/ai/providers.json` in the VFS (name, kind, base URL, model, key). Keys are entered in Settings → AI, stored locally, masked in UI, and **never readable by userland apps** — no capability exposes provider secrets; only the kernel's provider layer touches them. (Honest caveat: client-side storage cannot be truly secret from the user's own machine; the model here is protection from *apps*, not from the device owner.)

Per-agent settings: provider profile, model override, temperature, max tokens, monthly budget cap (token counting with a soft warning and hard stop).

---

## 3. The tool interface: how agents act on the system

Agents act through **tool use** (function calling). The kernel translates each agent's capability set into a tool schema offered to the model — an agent literally cannot be offered a tool its capabilities don't cover.

```
Agent capability          → Tool exposed to the model
sys:query                 → sys_query(path)            // read /sys/* (fps, processes, memory)
win:manage                → win_action(op, target)     // open/close/move/focus
fs:read:<scope>           → fs_read(path)              // scope-checked at dispatch
fs:write:<scope>          → fs_write(path, content)
notes:read / notes:write  → notes_search/read/write(…)
proc:spawn-app            → conjure_app(document)      // validated by pmos-conjure first
phys:spawn                → stage_spawn(object_desc)   // put objects in the 3D stage
timer:schedule            → schedule(task, when)       // reminders, background prompts
```

Dispatch flow: model emits tool call → kernel checks capability *again* at execution (defense in depth) → risk-tier check (§6) → execute → result returned to the model → stream continues. All tool calls are logged to `/sys/ai/log/` (inspectable in the terminal — transparency is a feature).

### 3.1 v1 implementation *(2026-07-31)*

The System Assistant acts through a **prompt-level tool protocol** rather than provider-native function calling — one mechanism that works identically on Anthropic, OpenAI-compatible, and local (Ollama/LM Studio) models:

- The model ends its reply with one line: `@@tool {"tool":"fs_read","args":{"path":"/notes/todo.md"}}`. The shell parses it, executes it **as an ordinary ABI client through capability-checked syscalls** (the kernel dispatcher re-checks every call — defense in depth held for free), and sends the outcome back as an `@@tool_result {json}` message; the model then continues. Budget: **4 tool calls per user request**.
- v1 tools: `sys_query` · `fs_list` · `fs_read` (truncated at 4 KB) · `fs_write` · `app_open`. All read/reversible-write (Tier 0–1): reads are silent, every call shows a `🔧 tool args` line in the palette, and writes additionally raise a toast — **nothing acts invisibly**. No delete/destructive tools are exposed (Tier 2 waits for the consent sheet).
- Known limitations: tool calls execute only from the **palette** surface (the terminal's `>` mode shares the agent but doesn't run tools); provider-native function calling and the capability→schema derivation stay future work, as do `/sys/ai/log` (needs the synthetic-dir VFS extension) and `notes_search`/`stage_spawn`/`schedule` tools.

---

## 4. App generation workflow (App Smith)

The signature flow — "conjure a timer app":

```
1. User: palette (voice or text) → "create a pomodoro timer with a 25 min default"
2. App Smith prompt = system prompt (embeds the Conjure spec + few-shot examples)
                      + user request
3. Model streams a Conjure JSON document
4. pmos-conjure validator: schema check, expression parse, capability audit, limits
   ├─ valid   → App Host spawns it as a new process; window appears; done (target: < 10 s)
   └─ invalid → validation errors are fed back to the model (repair loop, max 3 rounds)
                → still failing: apologetic toast + raw document saved to /apps/drafts
5. User accepts implicitly by using it. "Save" persists to /apps/<name>.conjure
   (relaunchable from the Launcher; editable — "make the button bigger" reopens
    App Smith with the current document as context for modification)
```

Design notes:
- The **repair loop** is why generation feels reliable: schema errors are machine-readable and models fix their own JSON well.
- **Modification of existing apps** is the same path with the current document in context — apps are living documents, which is the deepest point of the whole demo.
- Generated apps start with the **minimal capability set**; if the model requested more (e.g. `fs:read` for a note-reading widget), the consent sheet appears *before first run* ([[UI#5. Consent & Safety UI]]).

---

## 5. Voice pipeline

```
mic (getUserMedia / Web Speech API)
  → streaming STT (interim results)
  → command palette voice mode (live transcript — voice never acts invisibly, [[UI#2.4 The Command Palette]])
  → end of utterance → route:
       “system-ish” phrasing → System Assistant (with tools)
       everything else       → active palette conversation
  → G8-hold path bypasses routing entirely → raw transcript → notes inbox ([[Notes System#5. Voice capture]])
```

- **Default STT: Whisper in-browser** *(implemented 2026-07-31 — promoted from v2 to default the same day: Web Speech turned out to require a Google/Apple speech backend that non-branded Chromium builds — Brave, distro Chromium — don't ship)*. `whisper-worker.js` runs `whisper-tiny` via transformers.js (WebGPU, WASM fallback; multilingual model auto-picked for non-English `navigator.language`): **works in any browser, fully offline after a one-time ~40 MB cached model download, audio never leaves the machine**. `speech.js` captures the mic through an AudioWorklet, does energy-based endpointing (utterance ends after ~1.1 s of silence; 6 s no-speech and 15 s max-utterance guards), live-transcribes the buffer every ~1.5 s for interim text, and runs a final pass at utterance end. You can start speaking while the model still downloads — audio buffers and the palette shows progress.
- **Fallback STT: Web Speech API** — only when the Whisper worker cannot initialize (e.g. CDN unreachable on the very first run, before the model is cached); zero-download but needs a browser speech backend.
- Kernel interface (engine-agnostic — swapping engines touched zero kernel code): `VoiceCapture` syscall / `VoiceStatus`+`VoiceTranscript` events (ABI 1.5, `voice:input` capability), one utterance per 🤙-hold. **Only text crosses into the kernel — audio never does**, the mirror of the camera landmarks-only boundary.
- Routing (implemented): transcripts run through the *same* palette brain as typed input — app names (with spoken "open/launch/start the …" verbs stripped) launch apps, `make/create/build …` conjures via the App Smith, and anything unrecognized goes to the System Assistant, because spoken input is conversational (typed input keeps the explicit `>` prefix and the "unknown command" hint).
- TTS (optional, off by default): Web Speech synthesis for assistant replies in presentation mode.
- Still deferred: the G8-hold **voice note** path into the notes inbox; a Settings toggle for model size (`tiny` → `base`/`small` for better accuracy at a bigger download).

---

## 6. Safety

Layered, matching [[Architecture#4.7 Process & Capability Manager (`proc`)]] and [[UI#5. Consent & Safety UI]]:

1. **Capability ceiling** — an agent's tools are derived from its capability set; nothing outside it is even visible to the model.
2. **Double-check at dispatch** — every tool call re-validated at execution time (prompt injection cannot mint capabilities).
3. **Risk tiers** — Tier 0 (read-only): silent. Tier 1 (reversible writes: open window, create note): action toast with undo. Tier 2 (destructive/external: delete file, network beyond LLM, spawning agents): explicit confirmation (👍/Enter).
4. **Generated code cannot escape** — Conjure apps are interpreted; there is no `eval`, no DOM access, no raw syscall surface beyond the action catalog ([[App DSL#9. Security model]]).
5. **Injection awareness** — content read via `notes:read`/`fs:read` is data, not instructions; system prompts state this and Tier 2 confirmation is the structural backstop (a poisoned note can at worst *ask*; the user must still approve).
6. **Auditability** — full tool-call log under `/sys/ai/log/`; `ai-log` terminal command tails it.

---

## 7. Failure behavior

- Provider unreachable / key invalid → palette shows the error inline with a "fix in Settings" shortcut; the OS itself never degrades (AI is a service, not a dependency).
- Stream interrupted → partial output preserved, retry offered.
- Budget cap reached → agent paused with a clear notice; hard caps are never silently exceeded.
- Model refuses / can't produce valid Conjure after repairs → honest failure toast, draft saved; never a broken half-app on screen.

---

*Changes to this document must be recorded in [[Changelog]].*
