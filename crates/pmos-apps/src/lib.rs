//! Userland: the shell (window manager, dock, launcher, palette) and the
//! built-in apps (terminal, file explorer, notes, settings, browser).
//! Spec: docs/Architecture.md §5, docs/UI.md.
//!
//! Everything here talks to the kernel exclusively through `pmos-abi` —
//! the crate graph enforces it (no `pmos-kernel` dependency).

pub mod app_host;
pub mod apps;
pub mod cursor;
pub mod voicekit;
pub mod hand_tracker;
pub mod palette;
pub mod shell;
pub mod theme;
