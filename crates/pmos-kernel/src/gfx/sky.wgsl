// The distant galaxy (UI spec §1): procedural starfield + nebula rendered at
// infinite depth. The uniform carries a ROTATION-ONLY inverse view-projection,
// so camera translation/zoom can never change the parallax — the galaxy is
// unreachable by construction.

struct SkyUniforms {
    inv_rot_proj: mat4x4<f32>,
    time: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> u: SkyUniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Fullscreen triangle.
    var out: VsOut;
    let x = f32(i32(vi) - 1);
    let y = f32(i32(vi & 1u) * 2 - 1);
    out.pos = vec4<f32>(x * 3.0, y * 3.0, 0.0, 1.0);
    out.ndc = vec2<f32>(x * 3.0, y * 3.0);
    return out;
}

fn hash31(p: vec3<f32>) -> f32 {
    var q = fract(p * vec3<f32>(443.897, 441.423, 437.195));
    q += dot(q, q.yzx + 19.19);
    return fract((q.x + q.y) * q.z);
}

fn hash33(p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        hash31(p),
        hash31(p + vec3<f32>(1.7, 9.2, 3.1)),
        hash31(p + vec3<f32>(8.3, 2.8, 6.9)),
    );
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(mix(hash31(i), hash31(i + vec3<f32>(1., 0., 0.)), s.x),
            mix(hash31(i + vec3<f32>(0., 1., 0.)), hash31(i + vec3<f32>(1., 1., 0.)), s.x), s.y),
        mix(mix(hash31(i + vec3<f32>(0., 0., 1.)), hash31(i + vec3<f32>(1., 0., 1.)), s.x),
            mix(hash31(i + vec3<f32>(0., 1., 1.)), hash31(i + vec3<f32>(1., 1., 1.)), s.x), s.y),
        s.z,
    );
}

fn fbm(p: vec3<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += a * vnoise(q);
        q = q * 2.13;
        a *= 0.5;
    }
    return v;
}

// One star layer: grid the unit direction, one candidate star per cell.
fn stars(dir: vec3<f32>, freq: f32, t: f32) -> f32 {
    let p = dir * freq;
    let cell = floor(p);
    let center = cell + 0.5 + (hash33(cell) - 0.5) * 0.8;
    let d = length(p - center);
    let h = hash31(cell + 0.11);
    let presence = step(0.985, h);                      // sparse
    let twinkle = 0.72 + 0.28 * sin(t * (1.5 + h * 3.0) + h * 40.0);
    let core = smoothstep(0.10, 0.0, d);
    return presence * core * twinkle * (0.35 + 0.65 * fract(h * 13.7));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let world = u.inv_rot_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(world.xyz / world.w);

    // Deep-space base.
    var col = vec3<f32>(0.012, 0.014, 0.028);

    // Nebulae: two color fields, slowly drifting.
    let n1 = fbm(dir * 2.4 + vec3<f32>(u.time * 0.004, 0.0, 0.0));
    let n2 = fbm(dir * 3.1 + vec3<f32>(5.2, u.time * 0.003, 2.7));
    col += vec3<f32>(0.10, 0.16, 0.34) * pow(n1, 2.6) * 0.9;   // ion blue
    col += vec3<f32>(0.22, 0.10, 0.34) * pow(n2, 3.0) * 0.8;   // nebula violet

    // Milky band across the sphere (tilted).
    let band = exp(-pow(dot(dir, normalize(vec3<f32>(0.2, 1.0, 0.15))), 2.0) * 14.0);
    col += vec3<f32>(0.05, 0.06, 0.09) * band * (0.5 + 0.5 * n1);

    // Star layers, brighter inside the band.
    let s = stars(dir, 34.0, u.time) + stars(dir, 61.0, u.time) * 0.6
        + stars(dir, 90.0, u.time) * 0.35;
    col += vec3<f32>(0.85, 0.92, 1.0) * s * (0.55 + 0.7 * band);

    return vec4<f32>(col, 1.0);
}
