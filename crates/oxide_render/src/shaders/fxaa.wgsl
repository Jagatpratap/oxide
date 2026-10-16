// FXAA (Lottes' Fast Approximate Anti-Aliasing, "quality" edge search), run on the
// tonemapped, gamma-encoded image as URP does.

struct Fxaa {
    texel: vec4f,       // xy = 1 / size, z = 1 if the target is sRGB (decode before writing)
};

@group(0) @binding(0) var<uniform> fxaa: Fxaa;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var lin: sampler;

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

const EDGE_MIN: f32 = 0.0312;
const EDGE_MAX: f32 = 0.125;
const SUBPIXEL: f32 = 0.75;
const STEPS: i32 = 10;

fn luma(c: vec3f) -> f32 {
    return dot(c, vec3f(0.299, 0.587, 0.114));
}

fn tap(uv: vec2f) -> vec3f {
    return textureSampleLevel(src, lin, uv, 0.0).rgb;
}

fn lum(uv: vec2f) -> f32 {
    return luma(tap(uv));
}

fn to_linear(c: vec3f) -> vec3f {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3f(2.4));
    return select(hi, lo, c <= vec3f(0.04045));
}

fn fxaa_color(uv: vec2f) -> vec3f {
    let t = fxaa.texel.xy;
    let center = tap(uv);
    let l_m = luma(center);
    let l_n = lum(uv + vec2f(0.0, -t.y));
    let l_s = lum(uv + vec2f(0.0, t.y));
    let l_e = lum(uv + vec2f(t.x, 0.0));
    let l_w = lum(uv + vec2f(-t.x, 0.0));
    let l_min = min(l_m, min(min(l_n, l_s), min(l_e, l_w)));
    let l_max = max(l_m, max(max(l_n, l_s), max(l_e, l_w)));
    let range = l_max - l_min;
    if (range < max(EDGE_MIN, l_max * EDGE_MAX)) {
        return center;
    }
    let l_nw = lum(uv + vec2f(-t.x, -t.y));
    let l_ne = lum(uv + vec2f(t.x, -t.y));
    let l_sw = lum(uv + vec2f(-t.x, t.y));
    let l_se = lum(uv + vec2f(t.x, t.y));
    let l_ns = l_n + l_s;
    let l_we = l_w + l_e;
    let edge_h = abs(-2.0 * l_w + l_nw + l_sw) + abs(-2.0 * l_m + l_ns) * 2.0 + abs(-2.0 * l_e + l_ne + l_se);
    let edge_v = abs(-2.0 * l_n + l_nw + l_ne) + abs(-2.0 * l_m + l_we) * 2.0 + abs(-2.0 * l_s + l_sw + l_se);
    let horizontal = edge_h >= edge_v;

    // The side of the edge with the larger gradient.
    let l1 = select(l_w, l_n, horizontal);
    let l2 = select(l_e, l_s, horizontal);
    let g1 = l1 - l_m;
    let g2 = l2 - l_m;
    let steep1 = abs(g1) >= abs(g2);
    let gradient = 0.25 * max(abs(g1), abs(g2));
    var step_len = select(t.x, t.y, horizontal);
    var l_local: f32;
    if (steep1) {
        step_len = -step_len;
        l_local = 0.5 * (l1 + l_m);
    } else {
        l_local = 0.5 * (l2 + l_m);
    }
    var cur = uv;
    if (horizontal) {
        cur.y += step_len * 0.5;
    } else {
        cur.x += step_len * 0.5;
    }

    // Walk along the edge in both directions until the luma changes.
    let off = select(vec2f(0.0, t.y), vec2f(t.x, 0.0), horizontal);
    var uv1 = cur - off;
    var uv2 = cur + off;
    var e1 = lum(uv1) - l_local;
    var e2 = lum(uv2) - l_local;
    var done1 = abs(e1) >= gradient;
    var done2 = abs(e2) >= gradient;
    for (var i = 0; i < STEPS; i++) {
        if (done1 && done2) {
            break;
        }
        let k = select(1.0, 1.5 + f32(i) * 0.5, i > 1);
        if (!done1) {
            uv1 -= off * k;
            e1 = lum(uv1) - l_local;
            done1 = abs(e1) >= gradient;
        }
        if (!done2) {
            uv2 += off * k;
            e2 = lum(uv2) - l_local;
            done2 = abs(e2) >= gradient;
        }
    }
    let d1 = select(uv.y - uv1.y, uv.x - uv1.x, horizontal);
    let d2 = select(uv2.y - uv.y, uv2.x - uv.x, horizontal);
    let closer1 = d1 < d2;
    let d_min = min(d1, d2);
    let edge_len = d1 + d2;
    let pixel_offset = 0.5 - d_min / edge_len;
    let center_smaller = l_m < l_local;
    let correct = (select(e2, e1, closer1) < 0.0) != center_smaller;
    var final_offset = select(0.0, pixel_offset, correct);

    // Sub-pixel aliasing (thin features).
    let l_avg = (2.0 * (l_ns + l_we) + l_nw + l_ne + l_sw + l_se) / 12.0;
    let sub1 = clamp(abs(l_avg - l_m) / range, 0.0, 1.0);
    let sub2 = (-2.0 * sub1 + 3.0) * sub1 * sub1;
    final_offset = max(final_offset, sub2 * sub2 * SUBPIXEL);

    var out_uv = uv;
    if (horizontal) {
        out_uv.y += final_offset * step_len;
    } else {
        out_uv.x += final_offset * step_len;
    }
    return tap(out_uv);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    var c = fxaa_color(in.uv);
    if (fxaa.texel.z > 0.5) {
        c = to_linear(c);
    }
    return vec4f(c, 1.0);
}
