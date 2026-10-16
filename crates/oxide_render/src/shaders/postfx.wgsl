// Depth-based post effects (URP Depth Of Field and Motion Blur).
//
// Depth of field: circle of confusion (CoC) per pixel from depth → half-resolution color
// with CoC in alpha → disc gather (golden-angle spiral) where a sample counts if its own
// blur disc reaches the center → blend with the sharp image by CoC.
// Motion blur: per-pixel velocity from object motion vectors (TAA) or from depth and the
// camera's previous view-projection, then samples along it.

struct Fx {
    inv_vp: mat4x4f,    // inverse unjittered view-projection, this frame
    prev_vp: mat4x4f,   // unjittered view-projection, last frame
    dof: vec4f,         // x mode (1 Gaussian, 2 Bokeh), y start | focus distance, z end | CoC scale (px), w max radius (px)
    mb: vec4f,          // x intensity, y clamp (fraction of the screen), z object motion available, w samples
    proj: vec4f,        // near, far, orthographic
};

@group(0) @binding(0) var<uniform> fx: Fx;
@group(0) @binding(1) var tex_a: texture_2d<f32>;
@group(0) @binding(2) var tex_b: texture_2d<f32>;
@group(0) @binding(3) var tex_c: texture_2d<f32>;
@group(0) @binding(4) var lin: sampler;

const GOLDEN_ANGLE: f32 = 2.39996323;
const DOF_SAMPLES: i32 = 40;

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

fn linear_depth(d: f32) -> f32 {
    let n = fx.proj.x;
    let f = fx.proj.y;
    if (fx.proj.z > 0.5) {
        return f - d * (f - n);
    }
    return n * f / (d * (f - n) + n);
}

// Signed CoC radius in full-resolution pixels (negative = nearer than focus).
@fragment
fn fs_coc(in: VsOut) -> @location(0) vec4f {
    let z = linear_depth(textureLoad(tex_b, vec2i(in.clip.xy), 0).r);
    var coc: f32;
    if (fx.dof.x < 1.5) {
        // Gaussian: far field only.
        coc = clamp((z - fx.dof.y) / max(fx.dof.z - fx.dof.y, 1e-3), 0.0, 1.0) * fx.dof.w;
    } else {
        coc = clamp((1.0 - fx.dof.y / max(z, 1e-3)) * fx.dof.z, -fx.dof.w, fx.dof.w);
    }
    return vec4f(coc, 0.0, 0.0, 1.0);
}

// Half resolution: color (2×2 average through bilinear filtering) and CoC in half pixels.
@fragment
fn fs_dof_prefilter(in: VsOut) -> @location(0) vec4f {
    let c = textureSampleLevel(tex_a, lin, in.uv, 0.0).rgb;
    let p = vec2i(in.clip.xy) * 2;
    let size = vec2i(textureDimensions(tex_b)) - 1;
    var coc = 0.0;
    for (var i = 0; i < 4; i++) {
        let q = min(p + vec2i(i & 1, i >> 1u), size);
        let k = textureLoad(tex_b, q, 0).r;
        // Keep the strongest blur so in-focus edges stay inside their own pixels.
        coc = select(coc, k, abs(k) > abs(coc));
    }
    return vec4f(c, coc * 0.5);
}

@fragment
fn fs_dof_blur(in: VsOut) -> @location(0) vec4f {
    let texel = 1.0 / vec2f(textureDimensions(tex_a));
    let center = textureSampleLevel(tex_a, lin, in.uv, 0.0);
    let center_size = abs(center.a);
    let max_r = fx.dof.w * 0.5;
    var color = center.rgb;
    var total = 1.0;
    for (var i = 0; i < DOF_SAMPLES; i++) {
        let r = sqrt((f32(i) + 0.5) / f32(DOF_SAMPLES)) * max_r;
        let a = f32(i) * GOLDEN_ANGLE;
        let s = textureSampleLevel(tex_a, lin, in.uv + vec2f(cos(a), sin(a)) * r * texel, 0.0);
        var size = abs(s.a);
        // Farther samples can't spread far over a nearer center.
        if (s.a > center.a) {
            size = min(size, center_size * 2.0);
        }
        let m = smoothstep(r - 0.5, r + 0.5, size);
        color += mix(color / total, s.rgb, m);
        total += 1.0;
    }
    return vec4f(color / total, center.a);
}

@fragment
fn fs_dof_composite(in: VsOut) -> @location(0) vec4f {
    let px = vec2i(in.clip.xy);
    let sharp = textureLoad(tex_a, px, 0).rgb;
    let coc = abs(textureLoad(tex_b, px, 0).r);
    let blurred = textureSampleLevel(tex_c, lin, in.uv, 0.0);
    // Near-field blur spreads over sharper pixels behind it.
    let near = select(0.0, abs(blurred.a) * 2.0, blurred.a < 0.0);
    let t = smoothstep(0.5, 1.5, max(coc, near));
    return vec4f(mix(sharp, blurred.rgb, t), 1.0);
}

@fragment
fn fs_motion_blur(in: VsOut) -> @location(0) vec4f {
    let px = vec2i(in.clip.xy);
    let uv = in.uv;
    var v: vec2f;
    let m = textureLoad(tex_c, px, 0);
    if (fx.mb.z > 0.5 && m.a > 0.5) {
        v = m.xy;
    } else {
        let d = textureLoad(tex_b, px, 0).r;
        let w = fx.inv_vp * vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
        let p = fx.prev_vp * vec4f(w.xyz / w.w, 1.0);
        let prev_uv = vec2f(p.x / p.w * 0.5 + 0.5, 0.5 - p.y / p.w * 0.5);
        v = uv - prev_uv;
    }
    v *= fx.mb.x;
    let len = length(v);
    if (len > fx.mb.y) {
        v *= fx.mb.y / len;
    }
    let texel = 1.0 / vec2f(textureDimensions(tex_a));
    if (length(v / texel) < 0.5) {
        return vec4f(textureLoad(tex_a, px, 0).rgb, 1.0);
    }
    let n = i32(fx.mb.w);
    var c = vec3f(0.0);
    for (var i = 0; i < n; i++) {
        let t = f32(i) / f32(n - 1) - 0.5;
        c += textureSampleLevel(tex_a, lin, uv + v * t, 0.0).rgb;
    }
    return vec4f(c / f32(n), 1.0);
}
