//! Browser entry point. Trunk builds this binary to WASM and injects it into
//! `index.html`. Page load stays lightweight — the landing page is plain
//! HTML/CSS and must paint instantly (UI spec §2.0); the kernel boots only
//! when the visitor presses Launch, which calls `pmos_launch` below through
//! `window.wasmBindings`.

#[cfg(target_arch = "wasm32")]
mod web_entry {
    use wasm_bindgen::prelude::*;

    pub fn init() {
        console_error_panic_hook::set_once();
        let _ = console_log::init_with_level(log::Level::Debug);
        log::info!(
            "pmos core loaded — WebGPU available: {}",
            pmos_platform::webgpu_available()
        );
    }

    /// Called by the landing page after the Launch click and permission
    /// onboarding (Architecture §9.1). `permissions_json` carries the
    /// onboarding results: `{"camera":bool,"microphone":bool,"notifications":bool}`.
    #[wasm_bindgen]
    pub fn pmos_launch(permissions_json: String) {
        log::info!("launch requested, permissions: {permissions_json}");

        let _kernel = pmos_kernel::Kernel::new();

        // Swap the landing page for the OS root. The desktop itself arrives
        // with milestone 2 (docs/Todo.md); until then the root shows status.
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(landing) = doc.get_element_by_id("landing") {
                let _ = landing.set_attribute("hidden", "");
            }
            if let Some(os_root) = doc.get_element_by_id("os-root") {
                let _ = os_root.remove_attribute("hidden");
            }
            if let Some(status) = doc.get_element_by_id("boot-status") {
                status.set_text_content(Some(
                    "kernel ready — the 3D desktop lands in milestone 2",
                ));
            }
        }
    }
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    web_entry::init();

    #[cfg(not(target_arch = "wasm32"))]
    // Native binary exists only so `cargo check/test` cover this crate.
    println!("pmos-web targets wasm32; run `trunk serve` in crates/pmos-web.");
}
