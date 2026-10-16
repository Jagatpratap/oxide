// Canvas text (TextMeshProUGUI) inside egui's pass: positions in points relative to the
// paint callback's rectangle, colours in gamma space like the rest of egui.
// `text_sdf.wgsl` is prepended.

@group(0) @binding(0) var atlas_tex: texture_2d<f32>;
@group(0) @binding(1) var atlas_samp: sampler;

struct GIn {
    @location(0) c01: vec4f,
    @location(1) c23: vec4f,
    @location(2) uv_rect: vec4f,
    @location(3) face: vec4f,
    @location(4) outline: vec4f,
    @location(5) underlay: vec4f,
    @location(6) shape: vec4f,
    @location(7) under: vec4f,
    @location(8) atlas: vec4f,
    @location(9) rect: vec4f,
};

struct GOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) @interpolate(flat) uv_rect: vec4f,
    @location(2) @interpolate(flat) face: vec4f,
    @location(3) @interpolate(flat) outline: vec4f,
    @location(4) @interpolate(flat) underlay: vec4f,
    @location(5) @interpolate(flat) shape: vec4f,
    @location(6) @interpolate(flat) under: vec4f,
    @location(7) @interpolate(flat) atlas: vec4f,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, g: GIn) -> GOut {
    var pos = array<vec2f, 4>(g.c01.xy, g.c01.zw, g.c23.xy, g.c23.zw);
    var corner = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let k = corner[vi];
    var uvs = array<vec2f, 4>(
        vec2f(g.uv_rect.x, g.uv_rect.w),
        vec2f(g.uv_rect.z, g.uv_rect.w),
        vec2f(g.uv_rect.z, g.uv_rect.y),
        vec2f(g.uv_rect.x, g.uv_rect.y),
    );
    let ndc = pos[k] / max(g.rect.xy, vec2f(1.0)) * 2.0 - 1.0;
    var o: GOut;
    o.clip = vec4f(ndc.x, -ndc.y, 0.0, 1.0);
    o.uv = uvs[k];
    o.uv_rect = g.uv_rect;
    o.face = g.face;
    o.outline = g.outline;
    o.underlay = g.underlay;
    o.shape = g.shape;
    o.under = g.under;
    o.atlas = g.atlas;
    return o;
}

@fragment
fn fs_main(in: GOut) -> @location(0) vec4f {
    let p = SdfParams(in.face, in.outline, in.underlay, in.shape, in.under, in.atlas);
    let a = textureSample(atlas_tex, atlas_samp, in.uv).a;
    let off = vec2f(in.under.x, -in.under.y) * in.shape.w / in.atlas.xy;
    let ua = textureSample(atlas_tex, atlas_samp, clamp(in.uv - off, in.uv_rect.xy, in.uv_rect.zw)).a;
    let gr = fwidth(in.uv * in.atlas.xy);
    let px = max(0.5 * (gr.x + gr.y), 1e-4);
    return sdf_shade(p, a, ua, px);
}
