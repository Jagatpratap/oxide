// Oxide Full Screen Pass header: put before a user shader that defines
//     fn fs_main(in: FullscreenIn) -> vec4f
// `source` is the camera image (HDR before post-processing, the final linear image after),
// `depth` the raw scene depth (reversed-Z: 1 = near, 0 = far) when available.

struct FullscreenParams {
    params0: vec4f,     // the feature's Params 0
    params1: vec4f,     // the feature's Params 1
    texel: vec4f,       // 1 / width, 1 / height, width, height
    time: vec4f,        // x = time (s)
    flags: vec4f,       // x = 1 when the output needs sRGB encoding (internal)
};

@group(0) @binding(0) var<uniform> fx: FullscreenParams;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;
@group(0) @binding(3) var depth: texture_2d<f32>;

struct FullscreenIn {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn oxide_fullscreen_vs(@builtin(vertex_index) i: u32) -> FullscreenIn {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var o: FullscreenIn;
    o.position = vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2f(uv.x, 1.0 - uv.y);
    return o;
}

fn sample_source(uv: vec2f) -> vec4f {
    return textureSampleLevel(source, source_sampler, uv, 0.0);
}

fn sample_depth(uv: vec2f) -> f32 {
    let size = vec2f(textureDimensions(depth));
    return textureLoad(depth, vec2i(clamp(uv * size, vec2f(0.0), size - 1.0)), 0).r;
}

fn oxide_to_srgb(c: vec3f) -> vec3f {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3f(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3f(0.0031308));
}

@fragment
fn oxide_fullscreen_fs(in: FullscreenIn) -> @location(0) vec4f {
    var c = fs_main(in);
    if (fx.flags.x > 0.5) {
        c = vec4f(oxide_to_srgb(max(c.rgb, vec3f(0.0))), c.a);
    }
    return c;
}
