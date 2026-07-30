// Stage props: instanced cubes/spheres driven by the physics engine.

struct SceneUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: SceneUniforms;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // per-instance
    @location(2) ipos_scale: vec4<f32>,   // xyz = position, w = scale
    @location(3) irot: vec4<f32>,         // quaternion (x,y,z,w)
    @location(4) icolor: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let world = rotate(in.irot, in.pos * in.ipos_scale.w) + in.ipos_scale.xyz;
    out.clip = u.view_proj * vec4<f32>(world, 1.0);
    out.world_pos = world;
    out.normal = rotate(in.irot, in.normal);
    out.color = in.icolor.rgb;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let l = normalize(-u.light_dir.xyz);
    let v = normalize(u.camera_pos.xyz - in.world_pos);
    let diff = max(dot(n, l), 0.0);
    let h = normalize(l + v);
    let spec = pow(max(dot(n, h), 0.0), 48.0) * 0.5;
    let rim = pow(1.0 - max(dot(n, v), 0.0), 3.0) * 0.25;
    let ambient = 0.22;
    let col = in.color * (ambient + diff * 0.85) + vec3<f32>(spec) + in.color * rim;
    return vec4<f32>(col, 1.0);
}
