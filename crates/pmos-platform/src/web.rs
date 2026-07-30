//! Browser implementations of the platform traits.

use wasm_bindgen::JsValue;

/// WebGPU capability check used by the boot sequence (Architecture §9.1):
/// unsupported browsers must get the friendly failure screen, never a panic.
///
/// Probed via `Reflect` because web-sys still gates `Navigator::gpu()` behind
/// the unstable-APIs cfg; a property lookup needs no such flag.
pub fn webgpu_available() -> bool {
    web_sys::window()
        .map(|w| {
            js_sys::Reflect::get(w.navigator().as_ref(), &JsValue::from_str("gpu"))
                .map(|gpu| !gpu.is_undefined() && !gpu.is_null())
                .unwrap_or(false)
        })
        .unwrap_or(false)
}
