// Skybox / clear pass: a full-screen triangle drawn behind everything (reversed-Z depth 0).

struct VsOut {
    @builtin(position) clip: vec4f,
    @location(0) ndc: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.ndc = uv * 2.0 - 1.0;
    o.clip = vec4f(o.ndc, 0.0, 1.0);
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    // Reconstruct the view ray from the inverse view-projection (far plane = depth 0).
    let p = frame.inv_view_proj * vec4f(in.ndc, 0.0001, 1.0);
    let dir = normalize(p.xyz / p.w - frame.camera_pos.xyz);
    var c = sky_color(dir, true);
    if (frame.fog_color.w > 0.5) {
        // Blend the horizon into the fog color, like Unity's fog over the skybox.
        let h = clamp(1.0 - abs(dir.y) * 6.0, 0.0, 1.0);
        c = mix(c, frame.fog_color.rgb, h * h);
    }
    return vec4f(c, 1.0);
}

@fragment
fn fs_solid(in: VsOut) -> @location(0) vec4f {
    return frame.clear_color;
}
