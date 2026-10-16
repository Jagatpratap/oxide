// Temporal anti-aliasing resolve (URP TAA): reprojects last frame's result with motion
// vectors (or depth + camera motion), clips it to the current pixel's neighbourhood
// (variance clipping in YCoCg) and blends it with the new jittered frame.

struct Taa {
    inv_vp: mat4x4f,     // inverse unjittered view-projection, this frame
    prev_vp: mat4x4f,    // unjittered view-projection, last frame
    params: vec4f,       // x current-frame weight, y variance clamp scale, z history valid
    texel: vec4f,        // xy = 1 / size, zw = size
};

@group(0) @binding(0) var<uniform> taa: Taa;
@group(0) @binding(1) var current: texture_2d<f32>;
@group(0) @binding(2) var history: texture_2d<f32>;
@group(0) @binding(3) var motion: texture_2d<f32>;
@group(0) @binding(4) var depth: texture_depth_2d;
@group(0) @binding(5) var lin: sampler;

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

fn to_ycocg(c: vec3f) -> vec3f {
    return vec3f(
        0.25 * c.r + 0.5 * c.g + 0.25 * c.b,
        0.5 * c.r - 0.5 * c.b,
        -0.25 * c.r + 0.5 * c.g - 0.25 * c.b,
    );
}

fn from_ycocg(c: vec3f) -> vec3f {
    return vec3f(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

fn luma(c: vec3f) -> f32 {
    return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

// Bicubic Catmull-Rom history sample in 5 bilinear taps (keeps the result sharp).
fn sample_history(uv: vec2f) -> vec3f {
    let size = taa.texel.zw;
    let pos = uv * size;
    let tc = floor(pos - 0.5) + 0.5;
    let f = pos - tc;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let tc12 = (tc + w2 / w12) * taa.texel.xy;
    let tc0 = (tc - 1.0) * taa.texel.xy;
    let tc3 = (tc + 2.0) * taa.texel.xy;
    var c = vec3f(0.0);
    var w = 0.0;
    let a = w12.x * w0.y;
    c += textureSampleLevel(history, lin, vec2f(tc12.x, tc0.y), 0.0).rgb * a;
    let b = w0.x * w12.y;
    c += textureSampleLevel(history, lin, vec2f(tc0.x, tc12.y), 0.0).rgb * b;
    let m = w12.x * w12.y;
    c += textureSampleLevel(history, lin, tc12, 0.0).rgb * m;
    let d = w3.x * w12.y;
    c += textureSampleLevel(history, lin, vec2f(tc3.x, tc12.y), 0.0).rgb * d;
    let e = w12.x * w3.y;
    c += textureSampleLevel(history, lin, vec2f(tc12.x, tc3.y), 0.0).rgb * e;
    w = a + b + m + d + e;
    return max(c / w, vec3f(0.0));
}

// Pulls `q` toward the box center until it lies inside (Playdead's clip, not a clamp).
fn clip_aabb(mn: vec3f, mx: vec3f, q: vec3f) -> vec3f {
    let center = 0.5 * (mx + mn);
    let ext = 0.5 * (mx - mn) + vec3f(1e-5);
    let v = q - center;
    let unit = abs(v / ext);
    let m = max(unit.x, max(unit.y, unit.z));
    if (m > 1.0) {
        return center + v / m;
    }
    return q;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let size = vec2i(taa.texel.zw);
    let px = vec2i(in.clip.xy);
    let c = textureLoad(current, px, 0).rgb;

    // Neighbourhood statistics, and the nearest depth (dilates motion onto edges).
    var m1 = vec3f(0.0);
    var m2 = vec3f(0.0);
    var nearest = -1.0;
    var nearest_px = px;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = clamp(px + vec2i(x, y), vec2i(0), size - 1);
            let s = to_ycocg(textureLoad(current, q, 0).rgb);
            m1 += s;
            m2 += s * s;
            let d = textureLoad(depth, q, 0);
            if (d > nearest) {
                nearest = d;
                nearest_px = q;
            }
        }
    }
    if (taa.params.z < 0.5) {
        return vec4f(c, 1.0);
    }
    let uv = (vec2f(px) + 0.5) * taa.texel.xy;
    let mv = textureLoad(motion, nearest_px, 0);
    var prev_uv: vec2f;
    if (mv.a > 0.5) {
        prev_uv = uv - mv.xy;
    } else {
        let ndc = vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, nearest, 1.0);
        let w = taa.inv_vp * ndc;
        let p = taa.prev_vp * vec4f(w.xyz / w.w, 1.0);
        prev_uv = vec2f(p.x / p.w * 0.5 + 0.5, 0.5 - p.y / p.w * 0.5);
    }
    if (any(prev_uv < vec2f(0.0)) || any(prev_uv > vec2f(1.0))) {
        return vec4f(c, 1.0);
    }
    let mean = m1 / 9.0;
    let sigma = sqrt(abs(m2 / 9.0 - mean * mean)) * taa.params.y;
    let h = from_ycocg(clip_aabb(mean - sigma, mean + sigma, to_ycocg(sample_history(prev_uv))));

    // Blend weighted by inverse luminance so bright pixels don't dominate (Karis).
    let wc = taa.params.x / (1.0 + luma(c));
    let wh = (1.0 - taa.params.x) / (1.0 + luma(h));
    let result = (c * wc + h * wh) / (wc + wh);
    if (any(result != result)) {
        return vec4f(c, 1.0);
    }
    return vec4f(max(result, vec3f(0.0)), 1.0);
}
