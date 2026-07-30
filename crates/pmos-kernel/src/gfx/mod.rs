//! Graphics engine (Architecture spec §4.1).
//!
//! Owns the wgpu device and the per-frame render graph:
//!   0. ray-trace compute — the Whitted tracer into its own texture (§4.3)
//!   1. sky pass          — the unreachable galaxy (rotation-only parallax)
//!   2. props pass        — physics-driven instanced bodies (depth-tested)
//!   3. floor pass        — holographic grid disc (alpha-blended)
//!   4. overlay pass      — egui (shell, windows, dock, cursors)

pub mod camera;

use egui_wgpu::wgpu;
use glam::Vec3;

const MAX_INSTANCES: usize = 64;
const INSTANCE_FLOATS: usize = 12; // pos+scale (4) + quat (4) + color (4)
pub const RT_SIZE: (u32, u32) = (512, 384);

pub struct Gfx {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pub camera: camera::OrbitCamera,
    egui_renderer: egui_wgpu::Renderer,
    depth: wgpu::TextureView,
    sky: PassBits,
    floor: PassBits,
    props: PropsPass,
    rt: RayTracerPass,
    /// Ray tracer controls (set through the RtConfig syscall).
    pub rt_bounces: u8,
    pub rt_animate: bool,
}

struct PassBits {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

struct Mesh {
    vertices: wgpu::Buffer,
    count: u32,
}

struct PropsPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    cube: Mesh,
    sphere: Mesh,
    instances: wgpu::Buffer,
}

struct RayTracerPass {
    pipeline: wgpu::ComputePipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    view: wgpu::TextureView,
    egui_id: Option<egui::TextureId>,
}

fn depth_view(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: config.width.max(1),
                height: config.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn depth_state(write: bool, compare: wgpu::CompareFunction) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: Default::default(),
        bias: Default::default(),
    }
}

fn make_pass(
    device: &wgpu::Device,
    label: &str,
    shader_src: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    depth: wgpu::DepthStencilState,
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
        depth_stencil: Some(depth),
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

// ---------- mesh generation ----------

fn cube_mesh(device: &wgpu::Device) -> Mesh {
    // 6 faces × 2 triangles, position + normal interleaved.
    let f: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0., 0., 1.],
            [[-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.]],
        ),
        (
            [0., 0., -1.],
            [
                [1., -1., -1.],
                [-1., -1., -1.],
                [-1., 1., -1.],
                [1., 1., -1.],
            ],
        ),
        (
            [1., 0., 0.],
            [[1., -1., 1.], [1., -1., -1.], [1., 1., -1.], [1., 1., 1.]],
        ),
        (
            [-1., 0., 0.],
            [
                [-1., -1., -1.],
                [-1., -1., 1.],
                [-1., 1., 1.],
                [-1., 1., -1.],
            ],
        ),
        (
            [0., 1., 0.],
            [[-1., 1., 1.], [1., 1., 1.], [1., 1., -1.], [-1., 1., -1.]],
        ),
        (
            [0., -1., 0.],
            [
                [-1., -1., -1.],
                [1., -1., -1.],
                [1., -1., 1.],
                [-1., -1., 1.],
            ],
        ),
    ];
    let mut data: Vec<f32> = Vec::new();
    for (normal, corners) in f {
        for idx in [0usize, 1, 2, 0, 2, 3] {
            data.extend_from_slice(&corners[idx]);
            data.extend_from_slice(&normal);
        }
    }
    mesh_from(device, "cube", &data)
}

fn sphere_mesh(device: &wgpu::Device) -> Mesh {
    let stacks = 10u32;
    let sectors = 16u32;
    let vert = |i: u32, j: u32| -> ([f32; 3], [f32; 3]) {
        let theta = std::f32::consts::PI * i as f32 / stacks as f32;
        let phi = 2.0 * std::f32::consts::PI * j as f32 / sectors as f32;
        let p = [
            theta.sin() * phi.cos(),
            theta.cos(),
            theta.sin() * phi.sin(),
        ];
        (p, p)
    };
    let mut data: Vec<f32> = Vec::new();
    let mut push = |v: ([f32; 3], [f32; 3])| {
        data.extend_from_slice(&v.0);
        data.extend_from_slice(&v.1);
    };
    for i in 0..stacks {
        for j in 0..sectors {
            let a = vert(i, j);
            let b = vert(i + 1, j);
            let c = vert(i + 1, j + 1);
            let d = vert(i, j + 1);
            push(a);
            push(b);
            push(c);
            push(a);
            push(c);
            push(d);
        }
    }
    mesh_from(device, "sphere", &data)
}

fn mesh_from(device: &wgpu::Device, label: &str, data: &[f32]) -> Mesh {
    let vertices = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (data.len() * 4) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    Mesh {
        vertices,
        count: (data.len() / 6) as u32,
    }
    // (buffer written by caller via queue — see Gfx::new)
}

fn make_props_pass(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
) -> PropsPass {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("props"),
        source: wgpu::ShaderSource::Wgsl(include_str!("props.wgsl").into()),
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("props-uniforms"),
        size: 96, // mat4 + camera_pos + light_dir
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("props"),
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
        label: Some("props"),
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("props"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let vertex_layout = wgpu::VertexBufferLayout {
        array_stride: 24,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
    };
    let instance_layout = wgpu::VertexBufferLayout {
        array_stride: (INSTANCE_FLOATS * 4) as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x4, 4 => Float32x4],
    };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("props"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[vertex_layout, instance_layout],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Less)),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let cube = cube_mesh(device);
    let sphere = sphere_mesh(device);
    // Fill mesh buffers.
    let write_mesh = |mesh: &Mesh, gen: &dyn Fn() -> Vec<f32>| {
        queue.write_buffer(&mesh.vertices, 0, cast_f32(&gen()));
    };
    write_mesh(&cube, &|| cube_data());
    write_mesh(&sphere, &|| sphere_data());

    let instances = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("prop-instances"),
        size: (MAX_INSTANCES * INSTANCE_FLOATS * 4) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    PropsPass {
        pipeline,
        uniforms,
        bind_group,
        cube,
        sphere,
        instances,
    }
}

// Raw mesh data twins of the buffer builders above.
fn cube_data() -> Vec<f32> {
    let f: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0., 0., 1.],
            [[-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.]],
        ),
        (
            [0., 0., -1.],
            [
                [1., -1., -1.],
                [-1., -1., -1.],
                [-1., 1., -1.],
                [1., 1., -1.],
            ],
        ),
        (
            [1., 0., 0.],
            [[1., -1., 1.], [1., -1., -1.], [1., 1., -1.], [1., 1., 1.]],
        ),
        (
            [-1., 0., 0.],
            [
                [-1., -1., -1.],
                [-1., -1., 1.],
                [-1., 1., 1.],
                [-1., 1., -1.],
            ],
        ),
        (
            [0., 1., 0.],
            [[-1., 1., 1.], [1., 1., 1.], [1., 1., -1.], [-1., 1., -1.]],
        ),
        (
            [0., -1., 0.],
            [
                [-1., -1., -1.],
                [1., -1., -1.],
                [1., -1., 1.],
                [-1., -1., 1.],
            ],
        ),
    ];
    let mut data = Vec::new();
    for (normal, corners) in f {
        for idx in [0usize, 1, 2, 0, 2, 3] {
            data.extend_from_slice(&corners[idx]);
            data.extend_from_slice(&normal);
        }
    }
    data
}

fn sphere_data() -> Vec<f32> {
    let stacks = 10u32;
    let sectors = 16u32;
    let vert = |i: u32, j: u32| -> [f32; 3] {
        let theta = std::f32::consts::PI * i as f32 / stacks as f32;
        let phi = 2.0 * std::f32::consts::PI * j as f32 / sectors as f32;
        [
            theta.sin() * phi.cos(),
            theta.cos(),
            theta.sin() * phi.sin(),
        ]
    };
    let mut data = Vec::new();
    let mut push = |p: [f32; 3]| {
        data.extend_from_slice(&p);
        data.extend_from_slice(&p);
    };
    for i in 0..stacks {
        for j in 0..sectors {
            let (a, b, c, d) = (
                vert(i, j),
                vert(i + 1, j),
                vert(i + 1, j + 1),
                vert(i, j + 1),
            );
            push(a);
            push(b);
            push(c);
            push(a);
            push(c);
            push(d);
        }
    }
    data
}

fn make_rt_pass(device: &wgpu::Device) -> RayTracerPass {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("rt"),
        source: wgpu::ShaderSource::Wgsl(include_str!("rt.wgsl").into()),
    });
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rt-out"),
        size: wgpu::Extent3d {
            width: RT_SIZE.0,
            height: RT_SIZE.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rt-uniforms"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("rt"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
        ],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rt"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rt"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("rt"),
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("cs_main"),
        compilation_options: Default::default(),
        cache: None,
    });
    RayTracerPass {
        pipeline,
        uniforms,
        bind_group,
        view,
        egui_id: None,
    }
}

fn cast_f32(v: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) }
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
            depth_state(false, wgpu::CompareFunction::Always),
        );
        let floor = make_pass(
            &device,
            "floor",
            include_str!("floor.wgsl"),
            config.format,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            depth_state(false, wgpu::CompareFunction::LessEqual),
        );
        let props = make_props_pass(&device, &queue, config.format);
        let rt = make_rt_pass(&device);
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions {
                depth_stencil_format: Some(wgpu::TextureFormat::Depth32Float),
                ..Default::default()
            },
        );
        let depth = depth_view(&device, &config);
        Self {
            device,
            queue,
            surface,
            config,
            camera: camera::OrbitCamera::new(),
            egui_renderer,
            depth,
            sky,
            floor,
            props,
            rt,
            rt_bounces: 3,
            rt_animate: true,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 || (width, height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth = depth_view(&self.device, &self.config);
        log::debug!("surface resized to {width}x{height}");
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    /// The ray tracer's output as an egui texture (registered lazily).
    pub fn rt_texture(&mut self) -> egui::TextureId {
        if self.rt.egui_id.is_none() {
            self.rt.egui_id = Some(self.egui_renderer.register_native_texture(
                &self.device,
                &self.rt.view,
                wgpu::FilterMode::Linear,
            ));
        }
        self.rt.egui_id.unwrap()
    }

    /// Unproject a screen position (egui points) into a world-space ray.
    pub fn screen_ray(&self, pos: [f32; 2], viewport: [f32; 2]) -> (Vec3, Vec3) {
        let ndc = glam::Vec4::new(
            2.0 * pos[0] / viewport[0] - 1.0,
            1.0 - 2.0 * pos[1] / viewport[1],
            0.0,
            1.0,
        );
        let inv = self.camera.view_proj(self.aspect()).inverse();
        let near = inv * glam::Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
        let far = inv * glam::Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
        let near = near.truncate() / near.w;
        let far = far.truncate() / far.w;
        (near, (far - near).normalize())
    }

    fn write_pass_uniforms(&self, pass: &PassBits, mat: glam::Mat4, time: f32) {
        let mut data = [0u8; 80];
        data[..64].copy_from_slice(cast_f32(&mat.to_cols_array()));
        data[64..68].copy_from_slice(&time.to_le_bytes());
        self.queue.write_buffer(&pass.uniforms, 0, &data);
    }

    /// Execute the render graph for one frame.
    #[allow(clippy::type_complexity)]
    pub fn render(
        &mut self,
        primitives: &[egui::ClippedPrimitive],
        textures_delta: &egui::TexturesDelta,
        pixels_per_point: f32,
        time: f32,
        instances: &[([f32; 3], [f32; 4], u8, f32, [f32; 3])],
    ) {
        use wgpu::CurrentSurfaceTexture as Cst;
        let frame = match self.surface.get_current_texture() {
            Cst::Success(f) | Cst::Suboptimal(f) => f,
            Cst::Outdated | Cst::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.depth = depth_view(&self.device, &self.config);
                return;
            }
            Cst::Timeout | Cst::Occluded | Cst::Validation => return,
        };
        let view = frame.texture.create_view(&Default::default());

        let aspect = self.aspect();
        self.write_pass_uniforms(&self.sky, self.camera.inv_rot_proj(aspect), time);
        self.write_pass_uniforms(&self.floor, self.camera.view_proj(aspect), time);

        // Props uniforms: view_proj + camera pos + light dir.
        {
            let vp = self.camera.view_proj(aspect).to_cols_array();
            let eye = self.camera.eye();
            let mut data = [0u8; 96];
            data[..64].copy_from_slice(cast_f32(&vp));
            data[64..80].copy_from_slice(cast_f32(&[eye.x, eye.y, eye.z, 1.0]));
            data[80..96].copy_from_slice(cast_f32(&[-0.4, -1.0, -0.3, 0.0]));
            self.queue.write_buffer(&self.props.uniforms, 0, &data);
        }

        // Instance buffer: cubes first, then spheres (two draw calls).
        let mut inst_data: Vec<f32> = Vec::with_capacity(instances.len() * INSTANCE_FLOATS);
        let mut cube_count = 0u32;
        for (pos, rot, _, half, color) in instances.iter().filter(|i| i.2 == 0) {
            inst_data.extend_from_slice(&[pos[0], pos[1], pos[2], *half]);
            inst_data.extend_from_slice(rot);
            inst_data.extend_from_slice(&[color[0], color[1], color[2], 1.0]);
            cube_count += 1;
        }
        let mut sphere_count = 0u32;
        for (pos, rot, _, half, color) in instances.iter().filter(|i| i.2 == 1) {
            inst_data.extend_from_slice(&[pos[0], pos[1], pos[2], *half]);
            inst_data.extend_from_slice(rot);
            inst_data.extend_from_slice(&[color[0], color[1], color[2], 1.0]);
            sphere_count += 1;
        }
        if !inst_data.is_empty() {
            self.queue
                .write_buffer(&self.props.instances, 0, cast_f32(&inst_data));
        }

        // Ray tracer uniforms + dispatch.
        self.queue.write_buffer(
            &self.rt.uniforms,
            0,
            cast_f32(&[
                time,
                self.rt_bounces as f32,
                if self.rt_animate { 1.0 } else { 0.0 },
                0.0,
            ]),
        );

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
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            cpass.set_pipeline(&self.rt.pipeline);
            cpass.set_bind_group(0, &self.rt.bind_group, &[]);
            cpass.dispatch_workgroups(RT_SIZE.0.div_ceil(8), RT_SIZE.1.div_ceil(8), 1);
        }

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
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();

            rpass.set_pipeline(&self.sky.pipeline);
            rpass.set_bind_group(0, &self.sky.bind_group, &[]);
            rpass.draw(0..3, 0..1);

            if cube_count + sphere_count > 0 {
                rpass.set_pipeline(&self.props.pipeline);
                rpass.set_bind_group(0, &self.props.bind_group, &[]);
                rpass.set_vertex_buffer(1, self.props.instances.slice(..));
                if cube_count > 0 {
                    rpass.set_vertex_buffer(0, self.props.cube.vertices.slice(..));
                    rpass.draw(0..self.props.cube.count, 0..cube_count);
                }
                if sphere_count > 0 {
                    rpass.set_vertex_buffer(0, self.props.sphere.vertices.slice(..));
                    rpass.draw(
                        0..self.props.sphere.count,
                        cube_count..cube_count + sphere_count,
                    );
                }
            }

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
