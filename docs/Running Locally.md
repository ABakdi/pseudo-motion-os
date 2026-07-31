# Running Locally
**Pseudo Motion OS** · part of [[Pseudo Motion OS]]

How to build and run PMOS on your machine, from a fresh clone to the OS in your browser.

---

## 1. Prerequisites

| Tool | Version | Install |
|---|---|---|
| **Rust** | stable (1.97+) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` — `rust-toolchain.toml` pins the channel and auto-installs the wasm target on first build |
| **wasm32 target** | — | `rustup target add wasm32-unknown-unknown` (usually automatic via `rust-toolchain.toml`) |
| **trunk** | 0.21+ | `cargo install trunk --locked` — **if that fails** (known issue: its `libdeflate-sys` C dependency doesn't compile with some local toolchains), use the prebuilt binary: |

```sh
# prebuilt trunk fallback (Linux x86_64)
curl -sL https://github.com/trunk-rs/trunk/releases/latest/download/trunk-x86_64-unknown-linux-gnu.tar.gz \
  | tar xz -C ~/.cargo/bin trunk
trunk --version
```

**Browser:** anything with WebGPU — Chrome/Edge 113+, Safari 26+, Firefox 141+ (Windows) / 145+ (macOS). Without WebGPU the landing page still renders, but the OS won't launch.

**Webcam & microphone** are optional: they power hand gestures and voice (milestone 3+); every feature has mouse/keyboard fallbacks.

## 2. Clone & verify

```sh
git clone https://github.com/ABakdi/pseudo-motion-os
cd pseudo-motion-os

cargo check --workspace                                  # native compile check
cargo test  --workspace                                  # run tests
cargo check --workspace --target wasm32-unknown-unknown  # browser-target check
```

All three should finish green before you run anything.

## 3. Run the OS

```sh
cd crates/pmos-web
trunk serve
```

Open **http://127.0.0.1:8080**. You should see the landing page (starfield, logo, LAUNCH button). Press **Launch**, answer the permission cards (all skippable), and you're in.

- `trunk serve` rebuilds and hot-reloads on every file change.
- Port/dist settings live in `crates/pmos-web/Trunk.toml`.

### Production bundle

```sh
cd crates/pmos-web
trunk build --release        # output in dist/ — static files, host anywhere
# for GitHub Pages project sites:
trunk build --release --public-url /pseudo-motion-os/
```

CI (`.github/workflows/ci.yml`) runs tests + the wasm check on every push and deploys the release bundle to GitHub Pages from master.

### Desktop (Tauri) build

`crates/pmos-desktop` wraps the same `dist/` bundle in a native window. It is **outside the workspace** (native GUI system deps). On a machine with them (Linux: `webkit2gtk-4.1`, `libayatana-appindicator`; or Windows/macOS):

```sh
cargo install tauri-cli
cd crates/pmos-desktop
cargo tauri build            # runs trunk build --release first, then bundles
```

## 4. Useful during development

| What | How |
|---|---|
| Re-trigger the permission onboarding | DevTools → Application → Local Storage → delete `pmos.permissions.v1` (moves to `/sys/permissions` once the VFS lands, M6) |
| Kernel logs | Browser DevTools console — the kernel logs boot, ABI version, and syscall traffic at debug level |
| Format / lint | `cargo fmt --all` · `cargo clippy --workspace` |

## 5. Troubleshooting

- **"This browser can't run Pseudo Motion OS"** — your browser lacks WebGPU (or has it disabled). On Linux Firefox check `about:config` → `dom.webgpu.enabled`; on older Chrome try `chrome://flags/#enable-unsafe-webgpu`.
- **`cargo install trunk` fails on `libdeflate-sys`** — use the prebuilt binary above.
- **Blank page after Launch** — check the DevTools console; a panic in the WASM module logs there (panics are routed through `console_error_panic_hook`).
- **Port 8080 busy** — edit `[serve] port` in `crates/pmos-web/Trunk.toml` or run `trunk serve --port 8081`.

---

*Changes to this document must be recorded in [[Changelog]].*
