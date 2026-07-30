//! Browser entry point. Trunk builds this binary to WASM and injects it into
//! `index.html`. Page load stays lightweight — the landing page is plain
//! HTML/CSS and must paint instantly (UI spec §2.0); the OS boots only when
//! the visitor presses Launch, which calls `pmos_launch` below through
//! `window.wasmBindings`.

#[cfg(target_arch = "wasm32")]
mod os;

fn main() {
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        let _ = console_log::init_with_level(log::Level::Debug);
        log::info!(
            "pmos core loaded — WebGPU available: {}",
            pmos_platform::webgpu_available()
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    // Native binary exists only so `cargo check/test` cover this crate.
    println!("pmos-web targets wasm32; run `trunk serve` in crates/pmos-web.");
}
