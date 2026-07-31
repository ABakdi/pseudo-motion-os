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
| **WebLLM (in-browser)** *(v2)* | JS worker, WebGPU inference, OpenAI-compatible API surface | Fully offline/private; 1–4 GB model download, cached; small models suit chat/notes, not App Smith. |

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

- v1 STT: **Web Speech API** — pragmatic, zero-download, **free** (no API key); effectively Chrome-only and cloud-processed (documented limitation). *Implemented 2026-07-31:* `speech.js` runs one utterance per 🤙-hold (`continuous=false`, `interimResults=true`); the kernel drives it via the `VoiceCapture` syscall / `VoiceStatus`+`VoiceTranscript` events (ABI 1.5, `voice:input` capability). **Only text crosses into the kernel — audio never does**, the mirror of the camera landmarks-only boundary.
- Routing (implemented): transcripts run through the *same* palette brain as typed input — app names (with spoken "open/launch/start the …" verbs stripped) launch apps, `make/create/build …` conjures via the App Smith, and anything unrecognized goes to the System Assistant, because spoken input is conversational (typed input keeps the explicit `>` prefix and the "unknown command" hint).
- v2 STT: **Whisper in-browser** (transformers.js / WebGPU worker) behind the same `SpeechIn` trait — private and cross-browser, at the cost of a model download.
- TTS (optional, off by default): Web Speech synthesis for assistant replies in presentation mode.
- Still deferred: the G8-hold **voice note** path into the notes inbox.

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
