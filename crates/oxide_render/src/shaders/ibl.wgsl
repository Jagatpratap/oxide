// GGX prefiltering of a captured cube into a reflection-probe slot (needs cubemap.wgsl).

struct Params {
    face: u32,
    _pad0: u32,
    // GGX alpha (perceptual roughness squared) for prefiltering.
    alpha: f32,
    // Source cube face resolution (mip 0).
    src_size: f32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src_faces: texture_2d_array<f32>;
@group(0) @binding(2) var src_cube: texture_cube<f32>;
@group(0) @binding(3) var src_sampler: sampler;

struct VsOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.clip = vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
    // Texture rows go down; clip space y goes up.
    o.uv = vec2f(uv.x, 1.0 - uv.y);
    return o;
}

// 2x2 box filter of the previous mip (one face).
@fragment
fn fs_downsample(in: VsOut) -> @location(0) vec4f {
    return textureSampleLevel(src_faces, src_sampler, in.uv, i32(params.face), 0.0);
}

fn radical_inverse(bits_in: u32) -> f32 {
    var bits = bits_in;
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}

// GGX prefiltered radiance for one roughness level (Karis' split sum, n = v = r), with
// filtered importance sampling: each sample reads a source mip matching its solid angle.
@fragment
fn fs_prefilter(in: VsOut) -> @location(0) vec4f {
    let n = face_dir(params.face, in.uv);
    let a = params.alpha;
    if (a < 1e-4) {
        return vec4f(textureSampleLevel(src_cube, src_sampler, n, 0.0).rgb, 1.0);
    }
    let up = select(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), abs(n.z) < 0.999);
    let tx = normalize(cross(up, n));
    let ty = cross(n, tx);
    let count = 96u;
    let texel_sa = 4.0 * PI / (6.0 * params.src_size * params.src_size);
    var sum = vec3f(0.0);
    var weight = 0.0;
    for (var i = 0u; i < count; i++) {
        let xi = vec2f(f32(i) / f32(count), radical_inverse(i));
        let phi = 2.0 * PI * xi.x;
        let cos_t = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
        let sin_t = sqrt(1.0 - cos_t * cos_t);
        let h = tx * (sin_t * cos(phi)) + ty * (sin_t * sin(phi)) + n * cos_t;
        let l = 2.0 * dot(n, h) * h - n;
        let nl = dot(n, l);
        if (nl > 0.0) {
            let nh = max(dot(n, h), 0.0);
            let a2 = a * a;
            let dd = nh * nh * (a2 - 1.0) + 1.0;
            let d = a2 / (PI * dd * dd);
            let pdf = d * 0.25;
            let sample_sa = 1.0 / (f32(count) * pdf + 1e-4);
            let lod = max(0.5 * log2(sample_sa / texel_sa) + 1.0, 0.0);
            // Clamp very bright texels (the sun disk) so they don't sparkle in rough mips.
            let c = min(textureSampleLevel(src_cube, src_sampler, l, lod).rgb, vec3f(200.0));
            sum += c * nl;
            weight += nl;
        }
    }
    return vec4f(sum / max(weight, 1e-4), 1.0);
}
