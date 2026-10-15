// Post-processing: bloom (prefilter, downsample, upsample) and the final composite
// (exposure, ACES tonemapping, vignette, sRGB encode when the target is not sRGB).

struct Post {
    params: vec4f,   // x threshold, y knee, z bloom intensity, w exposure
    params2: vec4f,  // x vignette, y encode srgb, z bloom on, w unused
    texel: vec4f,    // xy = 1/source size
};

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.clip = vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2f(uv.x, 1.0 - uv.y);
    return o;
}

fn s(uv: vec2f) -> vec3f {
    return textureSampleLevel(src, src_sampler, uv, 0.0).rgb;
}

// 13-tap downsample (Jimenez, "Next Generation Post Processing in Call of Duty").
fn down13(uv: vec2f) -> vec3f {
    let t = post.texel.xy;
    let a = s(uv + t * vec2f(-2.0, -2.0));
    let b = s(uv + t * vec2f(0.0, -2.0));
    let c = s(uv + t * vec2f(2.0, -2.0));
    let d = s(uv + t * vec2f(-2.0, 0.0));
    let e = s(uv);
    let f = s(uv + t * vec2f(2.0, 0.0));
    let g = s(uv + t * vec2f(-2.0, 2.0));
    let h = s(uv + t * vec2f(0.0, 2.0));
    let i = s(uv + t * vec2f(2.0, 2.0));
    let j = s(uv + t * vec2f(-1.0, -1.0));
    let k = s(uv + t * vec2f(1.0, -1.0));
    let l = s(uv + t * vec2f(-1.0, 1.0));
    let m = s(uv + t * vec2f(1.0, 1.0));
    return e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

@fragment
fn fs_prefilter(in: VsOut) -> @location(0) vec4f {
    var c = down13(in.uv) * post.params.w;
    // Avoid fireflies: clamp extreme values before thresholding.
    c = min(c, vec3f(64.0));
    let br = max(c.r, max(c.g, c.b));
    let knee = post.params.x * post.params.y;
    var soft = clamp(br - post.params.x + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee + 1e-4);
    let contrib = max(soft, br - post.params.x) / max(br, 1e-4);
    return vec4f(c * contrib, 1.0);
}

@fragment
fn fs_down(in: VsOut) -> @location(0) vec4f {
    return vec4f(down13(in.uv), 1.0);
}

@fragment
fn fs_up(in: VsOut) -> @location(0) vec4f {
    // 3x3 tent upsample, added onto the larger mip by the blend state.
    let t = post.texel.xy;
    var c = s(in.uv) * 4.0;
    c += (s(in.uv + vec2f(-t.x, 0.0)) + s(in.uv + vec2f(t.x, 0.0)) + s(in.uv + vec2f(0.0, -t.y)) + s(in.uv + vec2f(0.0, t.y))) * 2.0;
    c += s(in.uv + vec2f(-t.x, -t.y)) + s(in.uv + vec2f(t.x, -t.y)) + s(in.uv + vec2f(-t.x, t.y)) + s(in.uv + vec2f(t.x, t.y));
    return vec4f(c / 16.0, 1.0);
}

fn aces(x: vec3f) -> vec3f {
    // Narkowicz ACES filmic fit.
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3f(0.0), vec3f(1.0));
}

fn to_srgb(c: vec3f) -> vec3f {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3f(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3f(0.0031308));
}

@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4f {
    var c = s(in.uv) * post.params.w;
    if (post.params2.z > 0.5) {
        c += textureSampleLevel(bloom_tex, src_sampler, in.uv, 0.0).rgb * post.params.z;
    }
    c = aces(c);
    let d = in.uv - 0.5;
    let vig = 1.0 - post.params2.x * smoothstep(0.2, 0.8, dot(d, d) * 2.0);
    c *= vig;
    if (post.params2.y > 0.5) {
        c = to_srgb(c);
    }
    return vec4f(c, 1.0);
}
