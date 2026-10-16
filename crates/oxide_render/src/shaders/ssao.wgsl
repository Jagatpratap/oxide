// Screen Space Ambient Occlusion (URP's SSAO renderer feature, depth source with
// reconstructed normals): normal-oriented hemisphere samples in view space against the
// depth prepass, then a depth-aware separable blur.

struct Ssao {
    proj: mat4x4f,       // projection used for the depth prepass (jittered with TAA)
    inv_proj: mat4x4f,
    params: vec4f,       // x radius (m), y intensity, z falloff distance (m), w samples
    blur_dir: vec4f,     // xy = blur direction in pixels
};

@group(0) @binding(0) var<uniform> ssao: Ssao;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var ao_in: texture_2d<f32>;

struct VsOut {
    @builtin(position) clip: vec4f,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.clip = vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn size() -> vec2i {
    return vec2i(textureDimensions(depth));
}

// View-space position of pixel `p` (view space: +Z forward, Unity's left-handed camera).
fn view_pos(p: vec2i) -> vec3f {
    let q = clamp(p, vec2i(0), size() - 1);
    let d = textureLoad(depth, q, 0);
    let uv = (vec2f(q) + 0.5) / vec2f(size());
    let v = ssao.inv_proj * vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    return v.xyz / v.w;
}

// Normal from depth: per axis, the neighbour on the same surface (smaller depth step).
fn view_normal(p: vec2i, c: vec3f) -> vec3f {
    let l = view_pos(p + vec2i(-1, 0));
    let r = view_pos(p + vec2i(1, 0));
    let u = view_pos(p + vec2i(0, -1));
    let d = view_pos(p + vec2i(0, 1));
    let dx = select(c - l, r - c, abs(r.z - c.z) < abs(c.z - l.z));
    let dy = select(c - u, d - c, abs(d.z - c.z) < abs(c.z - u.z));
    var n = normalize(cross(dx, dy));
    if (dot(n, c) > 0.0) {
        n = -n;
    }
    return n;
}

fn hash3(i: u32) -> vec3f {
    // A fixed, well spread kernel: Fibonacci hemisphere points, denser near the center.
    let n = ssao.params.w;
    let k = f32(i) + 0.5;
    let z = 1.0 - k / n;
    let r = sqrt(max(1.0 - z * z, 0.0));
    let a = k * 2.39996323;
    let scale = mix(0.15, 1.0, (k / n) * (k / n));
    return vec3f(cos(a) * r, sin(a) * r, z) * scale;
}

@fragment
fn fs_ao(in: VsOut) -> @location(0) vec4f {
    let p = vec2i(in.clip.xy);
    let d = textureLoad(depth, p, 0);
    if (d <= 0.0) {
        return vec4f(1.0);
    }
    let c = view_pos(p);
    let n = view_normal(p, c);
    // Per-pixel rotation of the kernel (interleaved gradient noise).
    let noise = fract(52.9829189 * fract(dot(in.clip.xy, vec2f(0.06711056, 0.00583715))));
    let a = noise * 6.2831853;
    let rnd = vec3f(cos(a), sin(a), 0.0);
    let t = normalize(rnd - n * dot(rnd, n) + vec3f(1e-4, 0.0, 0.0));
    let b = cross(n, t);
    let radius = ssao.params.x;
    let count = u32(ssao.params.w);
    var occlusion = 0.0;
    for (var i = 0u; i < count; i++) {
        let k = hash3(i);
        let s = c + (t * k.x + b * k.y + n * k.z) * radius;
        let clip = ssao.proj * vec4f(s, 1.0);
        if (clip.w <= 0.0) {
            continue;
        }
        let uv = vec2f(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
        let sp = view_pos(vec2i(uv * vec2f(size())));
        // Occluded if the surface there is in front of the sample point.
        let in_front = select(0.0, 1.0, sp.z < s.z - 0.02 * radius);
        let range = smoothstep(0.0, 1.0, radius / max(abs(c.z - sp.z), 1e-4));
        occlusion += in_front * range;
    }
    var ao = clamp(1.0 - occlusion / f32(count) * ssao.params.y, 0.0, 1.0);
    // Fade out with distance (URP "Falloff Distance").
    ao = mix(ao, 1.0, smoothstep(ssao.params.z * 0.8, ssao.params.z, c.z));
    return vec4f(ao, 0.0, 0.0, 1.0);
}

// 7-tap Gaussian along `blur_dir`, skipping samples across depth edges.
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4f {
    let p = vec2i(in.clip.xy);
    let dir = vec2i(ssao.blur_dir.xy);
    let z = view_pos(p).z;
    let weights = array<f32, 4>(0.383, 0.242, 0.061, 0.006);
    var sum = textureLoad(ao_in, p, 0).r * weights[0];
    var total = weights[0];
    for (var i = 1; i < 4; i++) {
        for (var s = -1; s <= 1; s += 2) {
            let q = clamp(p + dir * i * s, vec2i(0), size() - 1);
            let dz = abs(view_pos(q).z - z) / max(z, 1e-3);
            let w = weights[i] * select(0.0, 1.0, dz < 0.05);
            sum += textureLoad(ao_in, q, 0).r * w;
            total += w;
        }
    }
    return vec4f(sum / total, 0.0, 0.0, 1.0);
}
