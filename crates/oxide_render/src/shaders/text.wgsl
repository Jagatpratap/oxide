// 3D text (TextMeshPro): SDF glyph quads in world space. `text_sdf.wgsl` is prepended.

@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_samp: sampler;

struct TIn {
    @location(0) c0: vec4f,
    @location(1) c1: vec4f,
    @location(2) c2: vec4f,
    @location(3) uv_rect: vec4f,
    @location(4) face: vec4f,
    @location(5) outline: vec4f,
    @location(6) underlay: vec4f,
    @location(7) shape: vec4f,
    @location(8) under: vec4f,
    @location(9) atlas: vec4f,
    // Corner colours bl, br, tr, tl (sRGB RGBA8; Color Gradients).
    @location(10) tint: vec4<u32>,
};

struct TOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) world_pos: vec3f,
    @location(2) @interpolate(flat) uv_rect: vec4f,
    @location(3) @interpolate(flat) face: vec4f,
    @location(4) @interpolate(flat) outline: vec4f,
    @location(5) @interpolate(flat) underlay: vec4f,
    @location(6) @interpolate(flat) shape: vec4f,
    @location(7) @interpolate(flat) under: vec4f,
    @location(8) @interpolate(flat) atlas: vec4f,
    @location(9) tint: vec4f,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, t: TIn) -> TOut {
    // Corners bl, br, tr, tl.
    var pos = array<vec3f, 4>(
        t.c0.xyz,
        vec3f(t.c0.w, t.c1.x, t.c1.y),
        vec3f(t.c1.z, t.c1.w, t.c2.x),
        t.c2.yzw,
    );
    var corner = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let k = corner[vi];
    // Atlas rows run top to bottom: bl/br use the rect's max y.
    var uvs = array<vec2f, 4>(
        vec2f(t.uv_rect.x, t.uv_rect.w),
        vec2f(t.uv_rect.z, t.uv_rect.w),
        vec2f(t.uv_rect.z, t.uv_rect.y),
        vec2f(t.uv_rect.x, t.uv_rect.y),
    );
    var o: TOut;
    o.clip = frame.view_proj * vec4f(pos[k], 1.0);
    o.uv = uvs[k];
    o.world_pos = pos[k];
    o.uv_rect = t.uv_rect;
    o.face = t.face;
    o.outline = t.outline;
    o.underlay = t.underlay;
    o.shape = t.shape;
    o.under = t.under;
    o.atlas = t.atlas;
    let tc = unpack4x8unorm(t.tint[k]);
    o.tint = vec4f(pow(tc.rgb, vec3f(2.2)), tc.a);
    return o;
}

@fragment
fn fs_main(in: TOut) -> @location(0) vec4f {
    // (Every sample and derivative first: they need uniform control flow.)
    let tex = textureSample(atlas_tex, atlas_samp, in.uv);
    // Underlay: offset by up to the spread, kept inside this glyph's rectangle.
    let off = vec2f(in.under.x, -in.under.y) * in.shape.w / in.atlas.xy;
    let uv_u = clamp(in.uv - off, in.uv_rect.xy, in.uv_rect.zw);
    let ua = textureSample(atlas_tex, atlas_samp, uv_u).a;
    // Atlas pixels per screen pixel.
    let g = fwidth(in.uv * in.atlas.xy);
    let px = max(0.5 * (g.x + g.y), 1e-4);
    let face = in.face * in.tint;
    var c: vec4f;
    if in.atlas.w > 2.5 {
        // Sprites (`<sprite>`): the texture's colours.
        let sp = tex * face;
        c = vec4f(sp.rgb * sp.a, sp.a);
    } else {
        let outline = vec4f(in.outline.rgb, in.outline.a * in.tint.a);
        let underlay = vec4f(in.underlay.rgb, in.underlay.a * in.tint.a);
        let p = SdfParams(face, outline, underlay, in.shape, in.under, in.atlas);
        c = sdf_shade(p, tex.a, ua, px);
    }
    if c.a <= 0.002 {
        discard;
    }
    c = vec4f(apply_fog(c.rgb / c.a, in.world_pos) * c.a, c.a);
    return c;
}
