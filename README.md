# Pseudo Motion OS

A browser-native, WebAssembly-powered pseudo-operating system: a 3D desktop
with real physics, a WebGPU ray tracer, webcam hand-gesture control, and an
LLM that conjures working applications from natural language.

**Full specification:** [`docs/Pseudo Motion OS.md`](<docs/Pseudo Motion OS.md>) —
an Obsidian vault covering [architecture](docs/Architecture.md),
[UI](docs/UI.md), [hand gestures](<docs/Hand Gestures.md>),
[the notes system](<docs/Notes System.md>), [the AI system](<docs/AI System.md>),
and [the Conjure app DSL](<docs/App DSL.md>).
**Roadmap:** [`docs/Todo.md`](docs/Todo.md) · **History:** [`docs/Changelog.md`](docs/Changelog.md)

## Development

Full guide: [`docs/Running Locally.md`](<docs/Running Locally.md>).
Requirements: Rust (stable, `wasm32-unknown-unknown` target), [trunk](https://trunkrs.dev).

```sh
cargo check --workspace          # native check + tests
cargo test  --workspace
cd crates/pmos-web && trunk serve   # http://127.0.0.1:8080 (WebGPU browser required)
```

## Workspace layout

| Crate | Role |
|---|---|
| `pmos-abi` | Versioned syscall ABI — userland's only kernel dependency |
| `pmos-kernel` | Graphics, physics, ray tracer, input, AI, VFS, processes |
| `pmos-platform` | All browser/Tauri interop (the only `web-sys` importer) |
| `pmos-apps` | Shell + built-in apps (ABI clients only) |
| `pmos-conjure` | The Conjure DSL: parser, validator, interpreter |
| `pmos-web` | Browser entry point (trunk) |
