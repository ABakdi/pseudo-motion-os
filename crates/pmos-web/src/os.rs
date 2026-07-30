//! The OS runtime: winit event loop on the stage canvas, async wgpu init,
//! per-frame glue between kernel, shell and egui (Architecture §7).

use pmos_apps::shell::Shell;
use pmos_kernel::wgpu;
use pmos_kernel::{gfx::Gfx, Kernel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys};
use winit::window::{Window, WindowId};

/// Called by the landing page after the Launch click and permission
/// onboarding (Architecture §9.1). `permissions_json` carries the onboarding
/// results: `{"camera":bool,"microphone":bool,"notifications":bool}`.
#[wasm_bindgen]
pub fn pmos_launch(permissions_json: String) {
    // Idempotent: a second launch would spawn a second event loop and a
    // second kernel fighting over the same canvas.
    static LAUNCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if LAUNCHED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        log::warn!("pmos_launch called again — ignored");
        return;
    }
    log::info!("launch requested, permissions: {permissions_json}");

    let doc = web_sys::window()
        .and_then(|w| w.document())
        .expect("document");
    if let Some(landing) = doc.get_element_by_id("landing") {
        let _ = landing.set_attribute("hidden", "");
    }
    if let Some(os_root) = doc.get_element_by_id("os-root") {
        let _ = os_root.remove_attribute("hidden");
    }

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.spawn_app(OsApp::new());
}

fn now_secs() -> f64 {
    web_sys::window().unwrap().performance().unwrap().now() / 1000.0
}

// ---------- gesture-worker bridge ----------
// gesture.js delivers landmark frames here; the frame loop drains them into
// the kernel. Only the freshest frame matters (the recognizer is stateful).

thread_local! {
    static HAND_FRAME: RefCell<Option<(Vec<f32>, u32)>> = const { RefCell::new(None) };
    static CAMERA_STATUS: RefCell<Option<(bool, String)>> = const { RefCell::new(None) };
    static CAMERA_PIXELS: RefCell<Option<(Vec<u8>, u32, u32)>> = const { RefCell::new(None) };
}

/// Called by gesture.js with `hands * 63` floats (21 landmarks × x,y,z).
#[wasm_bindgen]
pub fn pmos_hands_frame(data: Vec<f32>, hands: u32) {
    HAND_FRAME.with(|f| *f.borrow_mut() = Some((data, hands)));
}

/// Called by gesture.js when the camera pipeline comes up, fails, or is
/// starting; `reason` explains disabled states to the user.
#[wasm_bindgen]
pub fn pmos_camera_status(enabled: bool, reason: Option<String>) {
    CAMERA_STATUS.with(|s| *s.borrow_mut() = Some((enabled, reason.unwrap_or_default())));
}

/// Preview pixels for the Hand Tracker viewer (RGBA, mirrored). These go
/// straight into an egui texture for the shell — deliberately NEVER through
/// the kernel (Hand Gestures spec §7 privacy boundary).
#[wasm_bindgen]
pub fn pmos_camera_frame(data: Vec<u8>, w: u32, h: u32) {
    CAMERA_PIXELS.with(|p| *p.borrow_mut() = Some((data, w, h)));
}

/// Apply kernel hands-directives to the JS pipeline (configure + start).
fn apply_hands_directives(d: &pmos_kernel::HandsDirectives, camera_start: bool) {
    let Some(win) = web_sys::window() else { return };
    let Ok(g) = js_sys::Reflect::get(&win, &JsValue::from_str("pmosGestures")) else {
        return;
    };
    if g.is_undefined() {
        return;
    }
    let opts = js_sys::Object::new();
    let set = |k: &str, v: JsValue| {
        let _ = js_sys::Reflect::set(&opts, &JsValue::from_str(k), &v);
    };
    set("streamFeed", JsValue::from_bool(d.stream_feed));
    set("numHands", JsValue::from_f64(d.tuning.num_hands as f64));
    set("detConf", JsValue::from_f64(d.tuning.det_conf as f64));
    set("trackConf", JsValue::from_f64(d.tuning.track_conf as f64));
    if let Ok(f) = js_sys::Reflect::get(&g, &JsValue::from_str("configure")) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = f.call1(&g, &opts);
        }
    }
    if camera_start {
        if let Ok(f) = js_sys::Reflect::get(&g, &JsValue::from_str("start")) {
            if let Some(f) = f.dyn_ref::<js_sys::Function>() {
                let _ = f.call0(&g);
            }
        }
    }
}

struct OsApp {
    window: Option<Arc<Window>>,
    canvas: Option<web_sys::HtmlCanvasElement>,
    /// Filled by the async wgpu init task, drained on the next frame.
    pending_gfx: Rc<RefCell<Option<Gfx>>>,
    kernel: Kernel,
    shell: Option<Shell>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    boot_time: f64,
    /// Hand Tracker preview texture (pixels never touch the kernel).
    camera_tex: Option<egui::TextureHandle>,
    applied_hands_generation: u32,
    // camera interaction
    dragging: bool,
    last_cursor: Option<(f32, f32)>,
    last_press: f64,
}

impl OsApp {
    fn new() -> Self {
        Self {
            window: None,
            canvas: None,
            pending_gfx: Rc::new(RefCell::new(None)),
            kernel: Kernel::new(),
            shell: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            boot_time: now_secs(),
            camera_tex: None,
            applied_hands_generation: 0,
            dragging: false,
            last_cursor: None,
            last_press: 0.0,
        }
    }

    fn frame(&mut self) {
        if let Some(gfx) = self.pending_gfx.borrow_mut().take() {
            self.kernel.install_gfx(gfx);
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Some(el) = doc.get_element_by_id("boot-status") {
                    let _ = el.set_attribute("hidden", "");
                }
            }
            log::info!("gfx online — render graph live");
        }
        let (Some(window), Some(egui_state)) = (&self.window, &mut self.egui_state) else {
            return;
        };
        if self.kernel.gfx.is_none() {
            return;
        }
        // The canvas is CSS-sized; keep winit and the surface in sync with its
        // layout size every frame (initial Resized events can predate gfx).
        if let Some(canvas) = &self.canvas {
            let dpr = web_sys::window().unwrap().device_pixel_ratio();
            let (cw, ch) = (
                (canvas.client_width() as f64 * dpr) as u32,
                (canvas.client_height() as f64 * dpr) as u32,
            );
            if cw > 0 && ch > 0 {
                let inner = window.inner_size();
                if (cw, ch) != (inner.width, inner.height) {
                    let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(cw, ch));
                }
                if let Some(gfx) = self.kernel.gfx.as_mut() {
                    gfx.resize(cw, ch);
                }
            }
        }
        if self.shell.is_none() {
            self.shell = Some(Shell::new(&mut self.kernel));
        }

        // Drain the gesture bridge into the kernel input pipeline.
        let now = now_secs();
        let ppp = self.egui_ctx.pixels_per_point();
        let viewport = [
            window.inner_size().width as f32 / ppp,
            window.inner_size().height as f32 / ppp,
        ];
        if let Some((enabled, reason)) = CAMERA_STATUS.with(|s| s.borrow_mut().take()) {
            self.kernel.set_camera_status(enabled, reason);
        }
        if let Some((data, hands)) = HAND_FRAME.with(|f| f.borrow_mut().take()) {
            self.kernel.hand_frame(&data, hands, viewport, now);
        }
        self.kernel.tick_hands(now);

        // Apply changed hands directives to the JS pipeline (exactly once
        // per generation), then upload any fresh preview pixels.
        if self.kernel.hands_directives.generation != self.applied_hands_generation {
            self.applied_hands_generation = self.kernel.hands_directives.generation;
            let start = self.kernel.hands_directives.camera_start;
            self.kernel.hands_directives.camera_start = false;
            apply_hands_directives(&self.kernel.hands_directives, start);
        }
        if let Some((data, w, h)) = CAMERA_PIXELS.with(|p| p.borrow_mut().take()) {
            let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &data);
            match &mut self.camera_tex {
                Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                None => {
                    self.camera_tex = Some(self.egui_ctx.load_texture(
                        "camera-preview",
                        img,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
        }
        let camera_feed = self.camera_tex.as_ref().map(|t| t.id());

        let time = (now_secs() - self.boot_time) as f32;
        let raw_input = egui_state.take_egui_input(window);

        // Disjoint field borrows: shell and kernel are separate fields.
        let shell = self.shell.as_mut().unwrap();
        let kernel = &mut self.kernel;
        self.egui_ctx.begin_pass(raw_input);
        shell.update(&self.egui_ctx, kernel, camera_feed);
        let output = self.egui_ctx.end_pass();

        egui_state.handle_platform_output(window, output.platform_output);
        let primitives = self
            .egui_ctx
            .tessellate(output.shapes, output.pixels_per_point);
        if let Some(gfx) = self.kernel.gfx.as_mut() {
            gfx.render(
                &primitives,
                &output.textures_delta,
                output.pixels_per_point,
                time,
            );
        }
    }
}

impl ApplicationHandler for OsApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("stage"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("#stage canvas");

        let attrs = Window::default_attributes().with_canvas(Some(canvas.clone()));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        self.canvas = Some(canvas);
        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        ));
        self.window = Some(window.clone());

        // wgpu init is async on the web; hand the result back through a cell.
        let pending = self.pending_gfx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::BROWSER_WEBGPU,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let surface = instance.create_surface(window.clone()).expect("surface");
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&surface),
                    ..Default::default()
                })
                .await
                .expect("adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .expect("device");
            let size = window.inner_size();
            let mut config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .expect("surface config");
            config.present_mode = wgpu::PresentMode::Fifo;
            surface.configure(&device, &config);
            *pending.borrow_mut() = Some(Gfx::new(device, queue, surface, config));
            window.request_redraw();
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let (Some(window), Some(egui_state)) = (&self.window, &mut self.egui_state) else {
            return;
        };
        let egui_wants = egui_state.on_window_event(window, &event).consumed;

        match event {
            WindowEvent::RedrawRequested => self.frame(),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.kernel.gfx.as_mut() {
                    gfx.resize(size.width, size.height);
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            // Stage camera controls — only when egui didn't claim the input
            // (UI spec §3.4).
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if state == ElementState::Pressed && !egui_wants {
                    let t = now_secs();
                    if t - self.last_press < 0.35 {
                        if let Some(gfx) = self.kernel.gfx.as_mut() {
                            gfx.camera.reset(); // double-click empty space
                        }
                    }
                    self.last_press = t;
                    self.dragging = true;
                } else if state == ElementState::Released {
                    self.dragging = false;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let pos = (position.x as f32, position.y as f32);
                if self.dragging && !egui_wants {
                    if let (Some(last), Some(gfx)) = (self.last_cursor, self.kernel.gfx.as_mut()) {
                        gfx.camera.orbit(pos.0 - last.0, pos.1 - last.1);
                    }
                }
                self.last_cursor = Some(pos);
                self.kernel
                    .input
                    .pointer_moved([pos.0, pos.1], pmos_abi::InputSource::Mouse);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if !egui_wants {
                    let dy = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                    };
                    if let Some(gfx) = self.kernel.gfx.as_mut() {
                        gfx.camera.zoom(dy);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                use winit::keyboard::{Key, NamedKey};
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Home)
                {
                    if let Some(gfx) = self.kernel.gfx.as_mut() {
                        gfx.camera.reset();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Continuous rendering: the stage is alive (galaxy drift, icon bob).
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
