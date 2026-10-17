// SMAA (Jimenez et al., Subpixel Morphological Anti-Aliasing) as URP runs it: three passes on
// the tonemapped, gamma-encoded image.
//   1. fs_edges: luma edges with local contrast adaptation (r = left edge, g = top edge).
//   2. fs_weights: for each edge, search along it to both ends, read the crossing edges there
//      to classify the shape (L, U, Z) and integrate the reconstructed silhouette over the
//      pixel. Oxide computes the coverage analytically instead of reading SMAA's precomputed
//      area texture, and has no diagonal patterns.
//      r = this pixel takes from the one above, g = the one above takes from this,
//      b = this takes from the left one, a = the left one takes from this.
//   3. fs_blend: mixes each pixel with its neighbours by those weights.

struct Smaa {
    texel: vec4f,       // xy = 1 / size, zw = size
    params: vec4f,      // x = edge threshold, y = max search steps, z = 1 if the target is sRGB
};

@group(0) @binding(0) var<uniform> smaa: Smaa;
@group(0) @binding(1) var color: texture_2d<f32>;
@group(0) @binding(2) var edges: texture_2d<f32>;
@group(0) @binding(3) var weights: texture_2d<f32>;

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

fn size() -> vec2i {
    return vec2i(smaa.texel.zw);
}

fn clamp_px(p: vec2i) -> vec2i {
    return clamp(p, vec2i(0), size() - 1);
}

fn rgb(p: vec2i) -> vec3f {
    return textureLoad(color, clamp_px(p), 0).rgb;
}

fn luma(p: vec2i) -> f32 {
    return dot(rgb(p), vec3f(0.2126, 0.7152, 0.0722));
}

// 1 where pixel `p` has a left (r) / top (g) edge; 0 outside the image.
fn edge(p: vec2i) -> vec2f {
    if (any(p < vec2i(0)) || any(p >= size())) {
        return vec2f(0.0);
    }
    return textureLoad(edges, p, 0).rg;
}

// ---- 1. Edge detection -------------------------------------------------------------------

@fragment
fn fs_edges(in: VsOut) -> @location(0) vec4f {
    let p = vec2i(in.clip.xy);
    let threshold = smaa.params.x;
    let l = luma(p);
    let left = luma(p + vec2i(-1, 0));
    let top = luma(p + vec2i(0, -1));
    let delta = abs(vec2f(l - left, l - top));
    var e = step(vec2f(threshold), delta);
    // No edge at the image border.
    e = select(e, vec2f(0.0, e.y), p.x == 0);
    e = select(e, vec2f(e.x, 0.0), p.y == 0);
    if (dot(e, vec2f(1.0)) == 0.0) {
        return vec4f(0.0);
    }
    // Local contrast adaptation: a much stronger edge next to this one hides it.
    let right = abs(l - luma(p + vec2i(1, 0)));
    let bottom = abs(l - luma(p + vec2i(0, 1)));
    let left2 = abs(left - luma(p + vec2i(-2, 0)));
    let top2 = abs(top - luma(p + vec2i(0, -2)));
    let m = max(max(delta, vec2f(right, bottom)), vec2f(left2, top2));
    let final_max = max(m.x, m.y);
    e = e * step(vec2f(final_max), 2.0 * delta);
    return vec4f(e, 0.0, 1.0);
}

// ---- 2. Blend weights --------------------------------------------------------------------

// Height of the reconstructed silhouette at `x` along a line of `len` pixels whose ends
// have crossings `cl` / `cr` (+1: towards the neighbour's side, -1: into this side, 0: none).
fn silhouette(x: f32, len: f32, cl: f32, cr: f32) -> f32 {
    if (cl != 0.0 && cr != 0.0) {
        if (cl == cr) {
            // U shape: from one end to the middle and back.
            let half_len = len * 0.5;
            if (x < half_len) {
                return cl * 0.5 * (1.0 - x / half_len);
            }
            return cr * 0.5 * ((x - half_len) / half_len);
        }
        // Z shape: one straight line from end to end.
        return mix(cl * 0.5, cr * 0.5, x / len);
    }
    if (cl != 0.0) {
        return cl * 0.5 * (1.0 - x / len);
    }
    if (cr != 0.0) {
        return cr * 0.5 * (x / len);
    }
    return 0.0;
}

// (how much this pixel takes from the neighbour, how much the neighbour takes from this):
// the silhouette's area on each side over the pixel at `d1` from the first end.
fn coverage(d1: f32, len: f32, cl: f32, cr: f32) -> vec2f {
    var a = vec2f(0.0);
    for (var i = 0; i < 8; i++) {
        let h = silhouette(d1 + (f32(i) + 0.5) / 8.0, len, cl, cr);
        a += vec2f(max(-h, 0.0), max(h, 0.0));
    }
    return a / 8.0;
}

// Crossing at a line end from the two perpendicular edges there: +1 on the neighbour's side,
// -1 on this side, 0 for none or both.
fn crossing(this_side: f32, other_side: f32) -> f32 {
    return other_side - this_side;
}

@fragment
fn fs_weights(in: VsOut) -> @location(0) vec4f {
    let p = vec2i(in.clip.xy);
    let e = edge(p);
    let steps = i32(smaa.params.y);
    var w = vec4f(0.0);
    if (e.y > 0.0) {
        // Top edge: a horizontal line between this row and the one above.
        var dl = 0;
        for (var i = 1; i <= steps; i++) {
            if (edge(p + vec2i(-i, 0)).y == 0.0) { break; }
            dl = i;
        }
        var dr = 0;
        for (var i = 1; i <= steps; i++) {
            if (edge(p + vec2i(i, 0)).y == 0.0) { break; }
            dr = i;
        }
        let xl = p.x - dl;
        let xr = p.x + dr;
        let cl = crossing(edge(vec2i(xl, p.y)).x, edge(vec2i(xl, p.y - 1)).x);
        let cr = crossing(edge(vec2i(xr + 1, p.y)).x, edge(vec2i(xr + 1, p.y - 1)).x);
        w = vec4f(coverage(f32(dl), f32(dl + dr + 1), cl, cr), w.zw);
    }
    if (e.x > 0.0) {
        // Left edge: a vertical line between this column and the one on the left.
        var du = 0;
        for (var i = 1; i <= steps; i++) {
            if (edge(p + vec2i(0, -i)).x == 0.0) { break; }
            du = i;
        }
        var dd = 0;
        for (var i = 1; i <= steps; i++) {
            if (edge(p + vec2i(0, i)).x == 0.0) { break; }
            dd = i;
        }
        let yt = p.y - du;
        let yb = p.y + dd;
        let ct = crossing(edge(vec2i(p.x, yt)).y, edge(vec2i(p.x - 1, yt)).y);
        let cb = crossing(edge(vec2i(p.x, yb + 1)).y, edge(vec2i(p.x - 1, yb + 1)).y);
        w = vec4f(w.xy, coverage(f32(du), f32(du + dd + 1), ct, cb));
    }
    return w;
}

// ---- 3. Neighbourhood blending -----------------------------------------------------------

fn to_linear(c: vec3f) -> vec3f {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3f(2.4));
    return select(hi, lo, c <= vec3f(0.04045));
}

fn to_srgb(c: vec3f) -> vec3f {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3f(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3f(0.0031308));
}

fn weight_at(p: vec2i) -> vec4f {
    return textureLoad(weights, clamp_px(p), 0);
}

// Neighbours mix in linear light, as SMAA does reading through sRGB views.
fn lin(p: vec2i) -> vec3f {
    return to_linear(rgb(p));
}

@fragment
fn fs_blend(in: VsOut) -> @location(0) vec4f {
    let p = vec2i(in.clip.xy);
    let own = weight_at(p);
    // From above (own r), below (the lower pixel's g), left (own b), right (the right
    // pixel's a).
    let wt = own.r;
    let wb = select(0.0, weight_at(p + vec2i(0, 1)).g, p.y + 1 < size().y);
    let wl = own.b;
    let wr = select(0.0, weight_at(p + vec2i(1, 0)).a, p.x + 1 < size().x);
    let c = textureLoad(color, p, 0);
    let sum = wt + wb + wl + wr;
    var out = to_linear(c.rgb);
    if (sum > 0.0) {
        let k = min(sum, 1.0) / sum;
        let mixed = lin(p + vec2i(0, -1)) * wt + lin(p + vec2i(0, 1)) * wb
            + lin(p + vec2i(-1, 0)) * wl + lin(p + vec2i(1, 0)) * wr;
        out = out * (1.0 - sum * k) + mixed * k;
    }
    // An sRGB target encodes on write; otherwise write the encoded value.
    if (smaa.params.z < 0.5) {
        out = to_srgb(out);
    }
    return vec4f(out, c.a);
}
