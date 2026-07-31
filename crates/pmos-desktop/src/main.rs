//! The Tauri desktop shell (Architecture §9.2): the identical WASM bundle
//! from `dist/` runs inside the native webview. Native FS mounts under /mnt
//! and child-webview browsing are the planned follow-ups (`NativeHost`).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Pseudo Motion OS");
}
