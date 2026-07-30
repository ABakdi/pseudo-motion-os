//! Browser entry point. Trunk builds this binary to WASM and injects it into
//! `index.html`. The landing page is plain HTML/CSS (instant paint); the WASM
//! kernel boots only when the user presses Launch (UI spec §2.0).

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::prelude::*;

    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).expect("logger init");

    log::info!(
        "Pseudo Motion OS booting — WebGPU available: {}",
        pmos_platform::webgpu_available()
    );

    // Boot sequence (Architecture §9.1): the landing page's Launch button
    // calls into here via the exported `pmos_launch` below. Until milestone 1
    // wires the real flow, boot just proves the kernel constructs.
    let _kernel = pmos_kernel::Kernel::new();

    // Signal the page that WASM is alive (used by the landing page script).
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(el) = doc.get_element_by_id("boot-status") {
            el.set_text_content(Some("kernel ready"));
        }
    }

    #[wasm_bindgen]
    pub fn pmos_launch() {
        log::info!("launch requested");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // Native binary exists only so `cargo check/test` cover this crate.
    println!("pmos-web targets wasm32; run `trunk serve` in crates/pmos-web.");
}
