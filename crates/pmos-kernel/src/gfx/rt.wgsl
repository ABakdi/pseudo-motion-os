// The Whitted ray tracer (Architecture spec §4.3): a WebGPU compute pass —
// spheres (diffuse, mirror, glass), a checkered plane, a point light with
// hard shadows, iterative reflection/refraction, sky gradient background.

struct RtUniforms {
    time: f32,
    bounces: f32,
    animate: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> u: RtUniforms;
@group(0) @binding(1) var out_tex: texture_storage_2d<rgba8unorm, write>;

struct Sphere {
    center: vec3<f32>,
    radius: f32,
    color: vec3<f32>,
    // 0 = diffuse, 1 = mirror, 2 = glass
    kind: f32,
};

const NSPHERES: u32 = 5u;

fn scene_sphere(i: u32, t: f32) -> Sphere {
    let a = select(0.0, t * 0.4, u.animate > 0.5);
    switch i {
        case 0u: { // mirror centerpiece
            return Sphere(vec3<f32>(0.0, 1.0, 0.0), 1.0, vec3<f32>(0.9, 0.9, 0.95), 1.0);
        }
        case 1u: { // glass orbiter
            return Sphere(
                vec3<f32>(2.2 * cos(a), 0.6, 2.2 * sin(a)),
                0.6,
                vec3<f32>(0.9, 0.95, 1.0),
                2.0,
            );
        }
        case 2u: {
            return Sphere(
                vec3<f32>(2.0 * cos(a + 2.1), 0.5, 2.0 * sin(a + 2.1)),
                0.5,
                vec3<f32>(0.43, 0.91, 1.0),
                0.0,
            );
        }
        case 3u: {
            return Sphere(
                vec3<f32>(2.4 * cos(a + 4.2), 0.45, 2.4 * sin(a + 4.2)),
                0.45,
                vec3<f32>(0.75, 0.52, 0.99),
                0.0,
            );
        }
        default: {
            return Sphere(vec3<f32>(-1.6, 0.35, -1.4), 0.35, vec3<f32>(1.0, 0.62, 0.42), 0.0);
        }
    }
}

fn light_pos() -> vec3<f32> {
    return vec3<f32>(4.0, 6.0, -3.0);
}

struct Hit {
    t: f32,
    pos: vec3<f32>,
    normal: vec3<f32>,
    color: vec3<f32>,
    kind: f32,
};

fn hit_sphere(ro: vec3<f32>, rd: vec3<f32>, s: Sphere) -> f32 {
    let oc = ro - s.center;
    let b = dot(oc, rd);
    let c = dot(oc, oc) - s.radius * s.radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return -1.0;
    }
    let sq = sqrt(disc);
    let t0 = -b - sq;
    if t0 > 0.001 {
        return t0;
    }
    let t1 = -b + sq;
    if t1 > 0.001 {
        return t1;
    }
    return -1.0;
}

fn intersect(ro: vec3<f32>, rd: vec3<f32>, t: f32) -> Hit {
    var best: Hit;
    best.t = 1e9;
    best.kind = -1.0;
    for (var i = 0u; i < NSPHERES; i++) {
        let s = scene_sphere(i, t);
        let d = hit_sphere(ro, rd, s);
        if d > 0.0 && d < best.t {
            best.t = d;
            best.pos = ro + rd * d;
            best.normal = normalize(best.pos - s.center);
            best.color = s.color;
            best.kind = s.kind;
        }
    }
    // Ground plane y = 0 with a checker.
    if abs(rd.y) > 1e-4 {
        let d = -ro.y / rd.y;
        if d > 0.001 && d < best.t {
            let p = ro + rd * d;
            if abs(p.x) < 12.0 && abs(p.z) < 12.0 {
                best.t = d;
                best.pos = p;
                best.normal = vec3<f32>(0.0, 1.0, 0.0);
                let checker = f32((i32(floor(p.x)) + i32(floor(p.z))) & 1);
                best.color = mix(vec3<f32>(0.10, 0.12, 0.20), vec3<f32>(0.24, 0.27, 0.38), checker);
                best.kind = 0.0;
            }
        }
    }
    return best;
}

fn sky(rd: vec3<f32>) -> vec3<f32> {
    let g = max(rd.y, 0.0);
    return mix(vec3<f32>(0.05, 0.06, 0.12), vec3<f32>(0.02, 0.03, 0.08), g)
        + vec3<f32>(0.4, 0.3, 0.6) * pow(max(dot(rd, normalize(vec3<f32>(0.5, 0.3, -0.6))), 0.0), 8.0) * 0.3;
}

fn in_shadow(p: vec3<f32>, t: f32) -> f32 {
    let l = light_pos() - p;
    let dist = length(l);
    let dir = l / dist;
    let h = intersect(p + dir * 0.01, dir, t);
    return select(1.0, 0.25, h.kind >= 0.0 && h.t < dist);
}

fn shade_local(h: Hit, rd: vec3<f32>, t: f32) -> vec3<f32> {
    let l = normalize(light_pos() - h.pos);
    let diff = max(dot(h.normal, l), 0.0);
    let v = -rd;
    let spec = pow(max(dot(normalize(l + v), h.normal), 0.0), 64.0);
    let sh = in_shadow(h.pos, t);
    return h.color * (0.15 + diff * 0.9 * sh) + vec3<f32>(spec * 0.6 * sh);
}

@compute @workgroup_size(8, 8)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(out_tex);
    if gid.x >= size.x || gid.y >= size.y {
        return;
    }
    let uv = (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) / vec2<f32>(f32(size.x), f32(size.y));
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);

    // A gently orbiting camera looking at the centerpiece.
    let ca = select(0.6, u.time * 0.15, u.animate > 0.5);
    let eye = vec3<f32>(6.5 * cos(ca), 2.6, 6.5 * sin(ca));
    let look = vec3<f32>(0.0, 0.8, 0.0);
    let fwd = normalize(look - eye);
    let right = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(right, fwd);
    let aspect = f32(size.x) / f32(size.y);
    var rd = normalize(fwd * 1.6 + right * ndc.x * aspect + up * ndc.y);
    var ro = eye;

    var color = vec3<f32>(0.0);
    var throughput = vec3<f32>(1.0);
    let max_bounce = u32(clamp(u.bounces, 1.0, 5.0));

    for (var bounce = 0u; bounce < max_bounce; bounce++) {
        let h = intersect(ro, rd, u.time);
        if h.kind < 0.0 {
            color += throughput * sky(rd);
            break;
        }
        if h.kind < 0.5 {
            // Diffuse: shade and stop this path.
            color += throughput * shade_local(h, rd, u.time);
            break;
        }
        if h.kind < 1.5 {
            // Mirror: a touch of local highlight, then bounce on.
            color += throughput * shade_local(h, rd, u.time) * 0.12;
            throughput *= 0.82 * h.color;
            rd = reflect(rd, h.normal);
            ro = h.pos + rd * 0.01;
            continue;
        }
        // Glass: refract (or total-internal-reflect), tinted slightly.
        let entering = dot(rd, h.normal) < 0.0;
        let n = select(-h.normal, h.normal, entering);
        let eta = select(1.5, 1.0 / 1.5, entering);
        let refr = refract(rd, n, eta);
        color += throughput * shade_local(h, rd, u.time) * 0.06;
        throughput *= 0.92 * h.color;
        if length(refr) < 0.5 {
            rd = reflect(rd, n);
        } else {
            rd = normalize(refr);
        }
        ro = h.pos + rd * 0.01;
    }

    textureStore(out_tex, vec2<i32>(i32(gid.x), i32(gid.y)), vec4<f32>(color, 1.0));
}
