// The stage floor: a holographic grid disc that fades into the void — enough
// geometry to anchor depth perception without competing with the galaxy.

struct FloorUniforms {
    view_proj: mat4x4<f32>,
    time: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> u: FloorUniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_xz: vec2<f32>,
};

const EXTENT: f32 = 60.0;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Two triangles spanning [-EXTENT, EXTENT]^2 at y = 0.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let xz = corners[vi] * EXTENT;
    var out: VsOut;
    out.pos = u.view_proj * vec4<f32>(xz.x, 0.0, xz.y, 1.0);
    out.world_xz = xz;
    return out;
}

fn grid_line(coord: vec2<f32>, spacing: f32, width: f32) -> f32 {
    let g = abs(fract(coord / spacing - 0.5) - 0.5) * spacing;
    let m = min(g.x, g.y);
    return smoothstep(width, 0.0, m);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let r = length(in.world_xz);
    let fade = 1.0 - smoothstep(9.0, 26.0, r);
    if (fade <= 0.0) {
        discard;
    }
    let minor = grid_line(in.world_xz, 1.0, 0.02) * 0.10;
    let major = grid_line(in.world_xz, 5.0, 0.035) * 0.22;
    // Soft breathing glow at the origin.
    let pulse = 0.85 + 0.15 * sin(u.time * 0.8);
    let disc = exp(-r * r * 0.02) * 0.10 * pulse;
    let a = (minor + major + disc) * fade;
    let tint = mix(vec3<f32>(0.43, 0.91, 1.0), vec3<f32>(0.75, 0.52, 0.99), r / 30.0);
    return vec4<f32>(tint * a, a);
}
