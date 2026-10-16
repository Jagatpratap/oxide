// Copies the scene depth (MSAA sample 0, or the single sample) into an R32Float texture
// that post effects can read. `DEPTH_TYPE` is replaced with texture_depth_2d or
// texture_depth_multisampled_2d when the module is built.

@group(0) @binding(0) var depth: DEPTH_TYPE;

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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    return vec4f(textureLoad(depth, vec2i(in.clip.xy), 0), 0.0, 0.0, 1.0);
}
