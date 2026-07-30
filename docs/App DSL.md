# App DSL — "Conjure"
**Pseudo Motion OS — Specification v0.3** · part of [[Pseudo Motion OS]]

**Conjure** is the declarative format in which AI-generated (and hand-written) PMOS applications are expressed, and the sandboxed expression language embedded in it. Conjure documents are interpreted by the **App Host** ([[Architecture#5. Layer 3 — Userland]]); they are never compiled and can never contain native code. Generation workflow lives in [[AI System#4. App generation workflow (App Smith)]].

---

## 1. Goals & non-goals

**Goals**
1. **LLM-reliable** — a frontier model must produce a valid document on the first or second try. Hence: JSON (the format models emit most reliably), a small orthogonal vocabulary, and machine-readable validation errors for the repair loop.
2. **Instant** — parse + validate + first frame in < 50 ms. No compilation.
3. **Sandboxed by construction** — the interpreter is the security boundary. The language *cannot express* anything outside its action catalog; capability checks gate the few actions that touch the system.
4. **Evolvable** — versioned format; future tiers (richer scripting, real WASM apps) slot in behind the same process model without breaking v1 documents.

**Non-goals:** Turing-complete general programming (no user-defined loops/recursion — see §6 rationale), pixel-perfect custom rendering (v1 has a fixed widget catalog + a simple canvas), 3D apps (post-v1: a `stage` section).

---

## 2. Document structure

A Conjure document is one JSON object, MIME `application/x-conjure+json`, file extension `.conjure`:

```json
{
  "conjure": "1.0",
  "manifest": { … },     // identity & requirements     (§3)
  "state":    { … },     // typed reactive variables     (§4)
  "ui":       { … },     // widget tree                  (§5)
  "handlers": { … },     // named action sequences       (§7)
  "timers":   [ … ],     // optional                     (§8)
  "resources":{ … }      // optional embedded assets     (§3.2)
}
```

Top-level unknown keys → validation error (strictness keeps the repair loop honest). `conjure` is the format version (semver major.minor; interpreter accepts same-major, ≤ its minor).

## 3. Manifest

```json
"manifest": {
  "id": "pomodoro-timer",            // kebab-case, unique per install
  "name": "Pomodoro Timer",
  "icon": "🍅",                      // emoji (v1: emoji only — universal, tiny)
  "description": "25-minute focus timer",
  "window": { "size": [320, 420], "resizable": true, "min_size": [240, 300] },
  "capabilities": []                  // requested beyond the default set (§10)
}
```

### 3.2 Resources
Optional embedded assets: `"resources": { "logo": { "kind": "image/png", "base64": "…" } }` referenced by name from `image` widgets. Hard cap 512 KB total (apps are documents, not bundles).

## 4. State

The app's only mutable data. Typed, declared with initial values; every mutation goes through actions (§7), and any state change re-renders the UI (egui is immediate-mode, so this is free).

```json
"state": {
  "remaining":  { "type": "number", "value": 1500 },
  "running":    { "type": "bool",   "value": false },
  "task_name":  { "type": "string", "value": "" },
  "laps":       { "type": "list",   "item": "number", "value": [] },
  "settings":   { "type": "map",    "value": { "long_break": 15 } }
}
```

Types: `number` (f64), `string`, `bool`, `list` (homogeneous, `item` type required), `map` (string keys, mixed primitive values), `color` (`"#rrggbb"` string subtype). No null — optionality is modeled with sentinel values or `has(map, key)`. Caps: ≤ 64 state entries, ≤ 256 KB total state (see §11).

## 5. UI tree

A nested tree of widget nodes: `{ "w": "<widget>", …props, "children": [...] }`. The catalog maps 1:1 onto egui and is deliberately small and composable:

**Layout** — invisible, children-bearing:

| Widget | Props | Purpose |
|---|---|---|
| `column` | `spacing`, `align` (`start\|center\|end\|stretch`) | vertical stack (root default) |
| `row` | `spacing`, `align`, `wrap` | horizontal stack |
| `group` | `title`, `frame` (bool) | framed/titled box |
| `scroll` | `axis` (`v\|h\|both`) | scrollable region |
| `grid` | `columns` | simple table layout |
| `spacer` | `size` or `flex` | fixed or flexible gap |
| `separator` | — | thin rule |

**Content & input:**

| Widget | Key props | Events |
|---|---|---|
| `label` | `text`, `size` (`small\|body\|heading\|title`), `color`, `bold` | — |
| `button` | `text`, `color`, `fill` (bool), `enabled` | `on_click` |
| `text_input` | `bind` (state ref), `placeholder`, `multiline` | `on_change`, `on_submit` |
| `slider` | `bind`, `min`, `max`, `step` | `on_change` |
| `checkbox` / `toggle` | `bind`, `text` | `on_change` |
| `dropdown` | `bind`, `options` (list expr) | `on_change` |
| `progress` | `value` (0..1 expr), `text` | — |
| `image` | `resource`, `fit` | — |
| `list_view` | `items` (list expr), `item_ui` (widget template; current element = `item`, index = `i`) | — |
| `canvas` | `width`, `height`, `draw` (list of draw-op exprs: `line/rect/circle/text`) | `on_pointer` |
| `if` | `cond` (expr), `then` (node), `else` (node, optional) | — (conditional rendering) |

Every text/numeric prop accepts either a literal or a **binding expression** (§6) written as `"${ … }"` (string interpolation allowed: `"Time left: ${fmt.mmss(remaining)}"`). `bind` props create two-way bindings to a state entry. `list_view` + `if` cover the dynamic-UI needs that would otherwise demand loops in the language.

## 6. Expression language

A pure, side-effect-free expression grammar — used in bindings, conditions, and action arguments. **No user-defined loops, functions, or recursion**: evaluation cost is statically boundable (expression tree depth ≤ 32, `list_view` is the only iteration construct and is bounded by list-size caps). This is the core sandbox guarantee: *the language cannot diverge*.

```ebnf
expr     = or ;
or       = and { "||" and } ;
and      = cmp { "&&" cmp } ;
cmp      = add [ ("=="|"!="|"<"|"<="|">"|">=") add ] ;
add      = mul { ("+"|"-") mul } ;
mul      = unary { ("*"|"/"|"%") unary } ;
unary    = [ "!"|"-" ] postfix ;
postfix  = primary { "." ident | "[" expr "]" | "(" [args] ")" } ;
primary  = number | string | "true" | "false" | ident | "(" expr ")" | list_lit ;
list_lit = "[" [ expr { "," expr } ] "]" ;
args     = expr { "," expr } ;
```

- Bare identifiers resolve to state entries; inside `item_ui`, `item` and `i` are bound; in handlers, `event` fields are bound (§7).
- `+` concatenates when either operand is a string. Numeric division by zero yields `0` with a runtime warning (LLM-generated math must not crash apps).
- **Builtin namespaces** (pure functions only):
  - `math.*` — `abs, min, max, floor, ceil, round, clamp, sqrt, pow, rand()` *(rand is host-seeded)*
  - `str.*` — `len, upper, lower, trim, contains, split, join, slice, replace`
  - `list.*` — `len, get, contains, sum, avg, sort, reverse, first, last`
  - `map.*` — `get(m,k,default), has(m,k), keys(m)`
  - `fmt.*` — `mmss(secs), num(x, decimals), date(ts, pattern)`
  - `time.now()` — milliseconds, via kernel clock (the *only* environment read, and it is deterministic per evaluation pass)

## 7. Handlers & the action catalog

Handlers are **named sequences of actions** referenced from widget events and timers. Actions are the only way to cause effects; each is a fixed verb from the catalog:

```json
"handlers": {
  "start_pause": [
    { "do": "set", "target": "running", "value": "${!running}" }
  ],
  "tick": [
    { "do": "if", "cond": "${running && remaining > 0}", "then": [
        { "do": "set", "target": "remaining", "value": "${remaining - 1}" }
    ]},
    { "do": "if", "cond": "${remaining == 0 && running}", "then": [
        { "do": "set", "target": "running", "value": "false" },
        { "do": "notify", "title": "Pomodoro done!", "body": "${task_name}" }
    ]}
  ]
}
```

**State actions:** `set {target, value}` · `toggle {target}` · `inc {target, by}` · `push {target, value}` · `remove_at {target, index}` · `clear {target}` · `set_key {target, key, value}`.
**Control actions:** `if {cond, then[], else[]}` (nestable, depth ≤ 8) · `emit {handler}` (call another handler; static call-graph must be acyclic — validator rejects cycles, so no recursion).
**System actions** (capability-gated where marked):

| Action | Args | Capability |
|---|---|---|
| `notify` | `title`, `body` | — (rate-limited) |
| `window` | `op: set_title\|resize\|close` | — (own window only) |
| `timer` | `op: start\|stop\|reset`, `id` | — |
| `fs_read` / `fs_write` / `fs_append` | `path`, (`into` state target / `value`) | `fs:read:<scope>` / `fs:write:<scope>` |
| `note_create` | `title`, `body` | `notes:write` |
| `ai_ask` | `prompt`, `into` (state target for the streamed answer) | `ai:prompt` |
| `stage_spawn` | `object` (primitive desc) | `phys:spawn` |
| `clipboard` | `op: copy`, `value` | `clipboard:write` |

Async actions (`fs_*`, `ai_ask`) don't block: they complete by writing into their `into` state target and optionally firing an `on_done` handler. Widget events: `on_click`, `on_change`, `on_submit`, `on_pointer`; lifecycle events: `on_init`, `on_close` (reserved handler names).

## 8. Timers

```json
"timers": [ { "id": "second", "every_ms": 1000, "handler": "tick", "autostart": true } ]
```
≤ 4 timers per app, `every_ms ≥ 100`. Timers fire in the frame loop (never mid-render), controlled by the `timer` action.

## 9. Security model

Restating the boundary precisely (see also [[AI System#6. Safety]]):
1. Conjure has **no** general loops, recursion, `eval`, string-to-code, DOM/JS access, or raw syscalls. Effects exist only as catalog actions.
2. The App Host executes each handler under a **step budget** (§11); exhaustion aborts the handler, marks the app "misbehaving" in its titlebar, and never affects other processes ([[Architecture#4.7 Process & Capability Manager (`proc`)]]).
3. Capability-gated actions are checked at *dispatch time* against the app's process capabilities — a document can *declare* anything; it can *do* only what was granted via the consent sheet ([[UI#5. Consent & Safety UI]]).
4. `fs:*` scopes for DSL apps are confined to `/home/appdata/<app-id>/**` by default; broader scopes require explicit consent listing exact paths.

## 10. Capabilities & defaults

Default grant (no consent needed): own window management, own state, timers, `notify` (rate-limited), private appdata folder. Everything else — `fs:*` beyond appdata, `notes:*`, `ai:prompt`, `phys:spawn`, `clipboard:write`, `input:raw-hands` — must be listed in `manifest.capabilities` with a `reason` string, surfaced on the consent sheet before first run.

## 11. Limits (validator-enforced)

| Limit | Value | Why |
|---|---|---|
| Document size | ≤ 256 KB (+512 KB resources) | apps are documents |
| UI nodes | ≤ 512 | render budget |
| State entries / total size | ≤ 64 / ≤ 256 KB | memory cap |
| List length | ≤ 4096 elements | `list_view` bound |
| Handler steps per event | ≤ 4096 actions+evaluations | divergence cap |
| `if` nesting / expr depth | ≤ 8 / ≤ 32 | stack safety |
| Timers | ≤ 4, ≥ 100 ms | frame-loop budget |
| `ai_ask` calls | ≤ 1 concurrent per app | cost control |

## 12. Validation

`pmos-conjure` (host-independent crate, [[Architecture#10. Crate Layout]]) validates in stages, each producing **machine-readable errors** (`{path, code, message, hint}`) designed to be fed back to the model in the repair loop:
1. JSON well-formedness → 2. schema (structure, types, unknown keys) → 3. reference resolution (state refs, handler refs, resource refs) → 4. expression parse + type check → 5. call-graph acyclicity → 6. limits → 7. capability audit (actions used vs. capabilities declared — using an ungated action set is fine; using `fs_write` without declaring `fs:write` is error `CAP001`, which tells App Smith to either drop the action or declare the capability).

## 13. Versioning & evolution

- `"conjure": "1.x"` — additive changes (new widgets, actions, builtins) bump minor; documents are forward-compatible within a major.
- Reserved for v2+: `"stage"` section (3D content), `"script"` tier (bounded Rhai for power apps — a *separate* capability), inter-app messaging (`emit_to`), WASM-module app class behind the same process model.
- The interpreter refuses future majors with a clean "made for a newer PMOS" error.

## 14. Complete example

```json
{
  "conjure": "1.0",
  "manifest": {
    "id": "pomodoro-timer", "name": "Pomodoro Timer", "icon": "🍅",
    "description": "25-minute focus timer with task label",
    "window": { "size": [300, 380], "resizable": false },
    "capabilities": []
  },
  "state": {
    "remaining": { "type": "number", "value": 1500 },
    "running":   { "type": "bool",   "value": false },
    "task_name": { "type": "string", "value": "" }
  },
  "ui": { "w": "column", "spacing": 12, "align": "center", "children": [
    { "w": "label", "text": "🍅 Pomodoro", "size": "title" },
    { "w": "text_input", "bind": "task_name", "placeholder": "What are you working on?" },
    { "w": "label", "text": "${fmt.mmss(remaining)}", "size": "title",
      "color": "${remaining < 60 ? '#e74c3c' : '#ecf0f1'}" },
    { "w": "progress", "value": "${1 - remaining / 1500}" },
    { "w": "row", "spacing": 8, "children": [
      { "w": "button", "text": "${running ? 'Pause' : 'Start'}", "fill": true,
        "on_click": "start_pause" },
      { "w": "button", "text": "Reset", "on_click": "reset" }
    ]}
  ]},
  "handlers": {
    "start_pause": [ { "do": "toggle", "target": "running" } ],
    "reset": [
      { "do": "set", "target": "remaining", "value": "1500" },
      { "do": "set", "target": "running", "value": "false" }
    ],
    "tick": [
      { "do": "if", "cond": "${running && remaining > 0}", "then": [
        { "do": "inc", "target": "remaining", "by": "-1" } ] },
      { "do": "if", "cond": "${running && remaining == 0}", "then": [
        { "do": "set", "target": "running", "value": "false" },
        { "do": "notify", "title": "Pomodoro done! 🎉", "body": "${task_name}" } ] }
    ]
  },
  "timers": [ { "id": "second", "every_ms": 1000, "handler": "tick", "autostart": true } ]
}
```

*(Note: the ternary `? :` shown in `color` is sugar for an `if` expression — included in the grammar as `cmp "?" expr ":" expr` at the `or` level; validator treats it as core.)*

---

*Changes to this document must be recorded in [[Changelog]].*
