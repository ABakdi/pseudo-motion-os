//! Graphics engine (Architecture spec §4.1).
//!
//! Owns the wgpu device and the per-frame render graph:
//!   1. sky pass      — the unreachable galaxy (rotation-only parallax)
//!   2. floor pass    — holographic grid disc (alpha-blended)
//!   3. overlay pass  — egui (shell, windows, dock, projected app icons)
//! Window-content-to-texture and the ray-trace compute pass join the graph in
//! later milestones (docs/Todo.md).

pub mod camera;

use egui_wgpu::wgpu;

pub struct Gfx {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pub camera: camera::OrbitCamera,
    egui_renderer: egui_wgpu::Renderer,
    sky: PassBits,
    floor: PassBits,
}

struct PassBits {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

fn make_pass(
    device: &wgpu::Device,
    label: &str,
    shader_src: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> PassBits {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: 80, // mat4 (64) + time + padding
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    PassBits {
        pipeline,
        uniforms,
        bind_group,
    }
}

impl Gfx {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    ) -> Self {
        let sky = make_pass(
            &device,
            "sky",
            include_str!("sky.wgsl"),
            config.format,
            None,
        );
        let floor = make_pass(
            &device,
            "floor",
            include_str!("floor.wgsl"),
            config.format,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );
        Self {
            device,
            queue,
            surface,
            config,
            camera: camera::OrbitCamera::new(),
            egui_renderer,
            sky,
            floor,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 || (width, height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        log::debug!("surface resized to {width}x{height}");
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    fn write_pass_uniforms(&self, pass: &PassBits, mat: glam::Mat4, time: f32) {
        let mut data = [0u8; 80];
        data[..64].copy_from_slice(bytemuck_cast(&mat.to_cols_array()));
        data[64..68].copy_from_slice(&time.to_le_bytes());
        self.queue.write_buffer(&pass.uniforms, 0, &data);
    }

    /// Execute the render graph for one frame.
    pub fn render(
        &mut self,
        primitives: &[egui::ClippedPrimitive],
        textures_delta: &egui::TexturesDelta,
        pixels_per_point: f32,
        time: f32,
    ) {
        use wgpu::CurrentSurfaceTexture as Cst;
        let frame = match self.surface.get_current_texture() {
            Cst::Success(f) | Cst::Suboptimal(f) => f,
            Cst::Outdated | Cst::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            Cst::Timeout | Cst::Occluded | Cst::Validation => return,
        };
        let view = frame.texture.create_view(&Default::default());

        let aspect = self.aspect();
        self.write_pass_uniforms(&self.sky, self.camera.inv_rot_proj(aspect), time);
        self.write_pass_uniforms(&self.floor, self.camera.view_proj(aspect), time);

        for (id, delta) in &textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point,
        };
        let user_cmds = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            primitives,
            &screen,
        );

        {
            let mut rpass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("stage"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();

            rpass.set_pipeline(&self.sky.pipeline);
            rpass.set_bind_group(0, &self.sky.bind_group, &[]);
            rpass.draw(0..3, 0..1);

            rpass.set_pipeline(&self.floor.pipeline);
            rpass.set_bind_group(0, &self.floor.bind_group, &[]);
            rpass.draw(0..6, 0..1);

            self.egui_renderer.render(&mut rpass, primitives, &screen);
        }

        for id in &textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        self.queue
            .submit(user_cmds.into_iter().chain([encoder.finish()]));
        frame.present();
    }
}

/// f32 slice → bytes without pulling in bytemuck for one call site.
fn bytemuck_cast(v: &[f32; 16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, 64) }
}
