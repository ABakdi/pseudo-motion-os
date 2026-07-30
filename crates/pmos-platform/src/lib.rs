//! Platform bridge (Architecture spec §3).
//!
//! The ONLY crate allowed to import `web-sys`/`js-sys` (or Tauri IPC later).
//! It exposes Rust traits (`GpuSurface`, `StorageBackend`, `CameraTracker`,
//! `SpeechIn`, `HttpClient`, `NativeHost`) that the kernel consumes, keeping
//! the kernel pure Rust and natively testable.

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::webgpu_available;

/// Native builds (tests, tooling) report no WebGPU; the real check is web-only.
#[cfg(not(target_arch = "wasm32"))]
pub fn webgpu_available() -> bool {
    false
}
