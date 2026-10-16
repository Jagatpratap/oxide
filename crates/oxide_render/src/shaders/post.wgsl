// Post-processing: bloom (prefilter, downsample, upsample) and the final uber pass in URP's
// order: lens distortion, chromatic aberration, exposure, bloom, vignette, color grading
// (white balance, contrast, color filter, channel mixer, shadows/midtones/highlights,
// lift/gamma/gain, split toning, hue, saturation), tonemapping, film grain, sRGB encode.

struct Post {
    params: vec4f,          // x threshold (linear), y knee, z bloom intensity, w exposure
    params2: vec4f,         // y encode sRGB, z bloom on, w tonemap (0 none, 1 neutral, 2 ACES)
    texel: vec4f,           // xy = 1 / source size, z unused, w bloom clamp
    bloom_tint: vec4f,
    vignette_color: vec4f,  // rgb, w = intensity * 3
    vignette_params: vec4f, // center xy, z smoothness * 5, w roundness (aspect or 1)
    color: vec4f,           // x contrast factor, y saturation factor, z hue shift (turns), w chromatic aberration
    color_filter: vec4f,    // rgb, w = 1 when grading is on
    white_balance: vec4f,   // LMS coefficients, w = 1 when on
    grain: vec4f,           // x intensity, y response, z time
    lift: vec4f,
    gamma: vec4f,
    gain: vec4f,
    shadows: vec4f,
    midtones: vec4f,
    highlights: vec4f,
    smh_ranges: vec4f,      // shadows start/end, highlights start/end
    split_shadows: vec4f,   // rgb (sRGB), w balance (-1..1)
    split_highlights: vec4f,
    mixer_r: vec4f,
    mixer_g: vec4f,
    mixer_b: vec4f,
    distortion_amount: vec4f,
    distortion_center_scale: vec4f,
    // Screen Space Lens Flare.
    sslf0: vec4f,           // x intensity (0 = off), y first, z secondary, w warped flare intensity
    sslf1: vec4f,           // rgb tint, w samples
    sslf2: vec4f,           // xy warped scale, z sample dimmer, w vignette effect
    sslf3: vec4f,           // x starting position, y scale, z chromatic aberration, w streaks intensity
    sslf4: vec4f,           // x streaks length, y orientation (rad), z streaks threshold, w aspect
};

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;
@group(0) @binding(4) var flare_tex: texture_2d<f32>;

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
    c = min(c, vec3f(min(post.texel.w, 64.0)));
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

// ---- Screen Space Lens Flare (reads a bright bloom mip) ----------------------------------

fn sslf_tap(uv: vec2f) -> vec3f {
    if any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) {
        return vec3f(0.0);
    }
    return textureSampleLevel(src, src_sampler, uv, 0.0).rgb;
}

// Red, green and blue sampled at slightly different distances from the centre.
fn sslf_ca(uv: vec2f) -> vec3f {
    let d = (uv - 0.5) * post.sslf3.z * 0.04;
    return vec3f(sslf_tap(uv - d).r, sslf_tap(uv).g, sslf_tap(uv + d).b);
}

// Fades ghosts whose source is near the screen edge.
fn sslf_vignette(uv: vec2f) -> f32 {
    let r = length(uv - 0.5) * 1.41421356;
    return mix(1.0, 1.0 - smoothstep(0.15, 1.0, r), clamp(post.sslf2.w, 0.0, 1.0));
}

@fragment
fn fs_sslf(in: VsOut) -> @location(0) vec4f {
    let uv = in.uv;
    let aspect = post.sslf4.w;
    var col = vec3f(0.0);
    var dim = 1.0;
    var k = post.sslf3.x;
    let n = u32(clamp(post.sslf1.w, 1.0, 3.0));
    for (var i = 0u; i < n; i++) {
        // First flare: bright spots mirrored through the screen centre.
        let p1 = 0.5 - (uv - 0.5) * k;
        col += sslf_ca(p1) * sslf_vignette(p1) * post.sslf0.y * dim;
        // Secondary flare: smaller ghosts between the source and the centre.
        let p2 = 0.5 + (uv - 0.5) * k;
        col += sslf_ca(p2) * sslf_vignette(p2) * post.sslf0.z * dim;
        // Warped flare: a halo around the centre (polar warp of the image).
        var dir = uv - 0.5;
        dir.x *= aspect;
        let len = max(length(dir), 1e-4);
        var hv = dir / len * (0.5 - len) * post.sslf2.xy / k;
        hv.x /= aspect;
        let p3 = 0.5 - hv;
        col += sslf_ca(p3) * sslf_vignette(p3) * post.sslf0.w * dim * 0.5;
        dim *= post.sslf2.z;
        k *= post.sslf3.y;
    }
    // Streaks: the brightest parts smeared along one direction.
    if post.sslf3.w > 0.0 {
        let o = post.sslf4.y;
        let d = vec2f(cos(o) / aspect, sin(o)) * post.sslf4.x * 0.5;
        var st = vec3f(0.0);
        var ws = 0.0;
        for (var j = 0; j < 24; j++) {
            let t = f32(j) / 23.0 * 2.0 - 1.0;
            let w = 1.0 - abs(t);
            st += max(sslf_tap(uv + d * t) - vec3f(post.sslf4.z), vec3f(0.0)) * w;
            ws += w;
        }
        col += st / ws * post.sslf3.w * 4.0;
    }
    return vec4f(col * post.sslf1.rgb, 1.0);
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

fn neutral_curve(x: vec3f, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> vec3f {
    return ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f;
}

// Unity's Neutral tonemapper.
fn neutral(x: vec3f) -> vec3f {
    let a = 0.2;
    let b = 0.29;
    let c = 0.24;
    let d = 0.272;
    let e = 0.02;
    let f = 0.3;
    let white_scale = vec3f(1.0) / neutral_curve(vec3f(5.3), a, b, c, d, e, f);
    return clamp(neutral_curve(x * white_scale, a, b, c, d, e, f) * white_scale, vec3f(0.0), vec3f(1.0));
}

fn to_srgb(c: vec3f) -> vec3f {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3f(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3f(0.0031308));
}

fn to_linear(c: vec3f) -> vec3f {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3f(2.4));
    return select(hi, lo, c <= vec3f(0.04045));
}

fn luminance(c: vec3f) -> f32 {
    return dot(c, vec3f(0.2126729, 0.7151522, 0.0721750));
}

fn rgb_to_hsv(c: vec3f) -> vec3f {
    let k = vec4f(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = select(vec4f(c.gb, k.xy), vec4f(c.bg, k.wz), c.g < c.b);
    let q = select(vec4f(p.xyw, c.r), vec4f(c.r, p.yzx), c.r < p.x);
    let d = q.x - min(q.w, q.y);
    let e = 1e-4;
    return vec3f(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

fn hsv_to_rgb(c: vec3f) -> vec3f {
    let k = vec4f(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + k.xyz) * 6.0 - k.www);
    return c.z * mix(k.xxx, clamp(p - k.xxx, vec3f(0.0), vec3f(1.0)), c.y);
}

fn soft_light(base: vec3f, blend: vec3f) -> vec3f {
    let r1 = 2.0 * base * blend + base * base * (1.0 - 2.0 * blend);
    let r2 = sqrt(base) * (2.0 * blend - 1.0) + 2.0 * base * (1.0 - blend);
    return select(r2, r1, blend <= vec3f(0.5));
}

// Unity's white balance in LMS space.
fn white_balance(c: vec3f) -> vec3f {
    let lms = vec3f(
        dot(vec3f(3.90405e-1, 5.49941e-1, 8.92632e-3), c),
        dot(vec3f(7.08416e-2, 9.63172e-1, 1.35775e-3), c),
        dot(vec3f(2.31082e-2, 1.28021e-1, 9.36245e-1), c),
    ) * post.white_balance.rgb;
    return vec3f(
        dot(vec3f(2.85847e+0, -1.62879e+0, -2.48910e-2), lms),
        dot(vec3f(-2.10182e-1, 1.15820e+0, 3.24281e-4), lms),
        dot(vec3f(-4.18120e-2, -1.18169e-1, 1.06867e+0), lms),
    );
}

fn grade(c_in: vec3f) -> vec3f {
    var c = c_in;
    if (post.white_balance.w > 0.5) {
        c = white_balance(c);
    }
    // Contrast in log space around middle grey.
    let mid = 0.18;
    c = mid * pow(max(c, vec3f(1e-6)) / mid, vec3f(post.color.x));
    c = max(c * post.color_filter.rgb, vec3f(0.0));
    c = vec3f(dot(c, post.mixer_r.rgb), dot(c, post.mixer_g.rgb), dot(c, post.mixer_b.rgb));
    // Shadows / midtones / highlights.
    let l = luminance(c);
    let sf = 1.0 - smoothstep(post.smh_ranges.x, post.smh_ranges.y, l);
    let hf = smoothstep(post.smh_ranges.z, post.smh_ranges.w, l);
    let mf = 1.0 - sf - hf;
    c = c * post.shadows.rgb * sf + c * post.midtones.rgb * mf + c * post.highlights.rgb * hf;
    // Lift / gamma / gain.
    c = c * post.gain.rgb + post.lift.rgb;
    c = sign(c) * pow(abs(c), post.gamma.rgb);
    // Split toning (gamma space).
    var g = to_srgb(clamp(c, vec3f(0.0), vec3f(1.0)));
    let t = clamp(luminance(g) + post.split_shadows.w, 0.0, 1.0);
    let sh = mix(vec3f(0.5), post.split_shadows.rgb, 1.0 - t);
    let hi = mix(vec3f(0.5), post.split_highlights.rgb, t);
    if (any(post.split_shadows.rgb != vec3f(0.5)) || any(post.split_highlights.rgb != vec3f(0.5))) {
        g = soft_light(soft_light(g, sh), hi);
        c = select(c, to_linear(g), c <= vec3f(1.0));
    }
    // Hue and saturation.
    if (post.color.z != 0.0) {
        var hsv = rgb_to_hsv(c);
        hsv.x = fract(hsv.x + post.color.z);
        c = hsv_to_rgb(hsv);
    }
    let lum = luminance(c);
    c = max(vec3f(lum) + (c - vec3f(lum)) * post.color.y, vec3f(0.0));
    return c;
}

// URP's lens distortion of the sampling position.
fn distort(uv_in: vec2f) -> vec2f {
    let amount = post.distortion_amount;
    if (amount.w == 0.0) {
        return uv_in;
    }
    let cs = post.distortion_center_scale;
    var uv = (uv_in - 0.5) * amount.z + 0.5;
    let ruv = cs.zw * (uv - 0.5 - cs.xy);
    var ru = length(ruv);
    if (amount.w > 0.0) {
        let wu = ru * amount.x;
        ru = tan(wu) * (1.0 / (ru * amount.y));
    } else {
        ru = (1.0 / ru) * amount.x * atan(ru * amount.y);
    }
    return uv + ruv * (ru - 1.0);
}

fn hash(p: vec2f) -> f32 {
    return fract(sin(dot(p, vec2f(12.9898, 78.233))) * 43758.5453);
}

@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4f {
    let uv = distort(in.uv);
    var c: vec3f;
    let ca = post.color.w;
    if (ca > 0.0) {
        // Three taps spread toward the edges (URP's fast path).
        let coords = 2.0 * uv - 1.0;
        let end = uv - coords * dot(coords, coords) * ca * 0.05;
        let delta = (end - uv) / 3.0;
        c = vec3f(s(uv).r, s(uv + delta).g, s(uv + delta * 2.0).b);
    } else {
        c = s(uv);
    }
    c *= post.params.w;
    if (post.params2.z > 0.5) {
        c += textureSampleLevel(bloom_tex, src_sampler, uv, 0.0).rgb * post.params.z * post.bloom_tint.rgb;
    }
    if (post.sslf0.x > 0.0) {
        c += textureSampleLevel(flare_tex, src_sampler, uv, 0.0).rgb * post.sslf0.x;
    }
    // Vignette.
    if (post.vignette_color.w > 0.0) {
        var d = abs(in.uv - post.vignette_params.xy) * post.vignette_color.w;
        d.x *= post.vignette_params.w;
        let f = pow(clamp(1.0 - dot(d, d), 0.0, 1.0), post.vignette_params.z);
        c *= mix(post.vignette_color.rgb, vec3f(1.0), f);
    }
    if (post.color_filter.w > 0.5) {
        c = grade(c);
    }
    let mode = post.params2.w;
    if (mode > 1.5) {
        c = aces(c);
    } else if (mode > 0.5) {
        c = neutral(c);
    } else {
        c = clamp(c, vec3f(0.0), vec3f(1.0));
    }
    // Film grain, less in bright areas.
    if (post.grain.x > 0.0) {
        let n = (hash(in.clip.xy + fract(post.grain.z) * 113.0) - 0.5) * 2.0;
        let lum = mix(1.0, 1.0 - sqrt(luminance(c)), post.grain.y);
        c = max(c + c * n * post.grain.x * lum, vec3f(0.0));
    }
    if (post.params2.y > 0.5) {
        c = to_srgb(c);
    }
    return vec4f(c, 1.0);
}
