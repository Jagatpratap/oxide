// Deferred lighting (URP Deferred's lighting pass): one full-screen triangle per camera
// viewport reads the G-buffer and adds the sun and the clustered punctual lights to the
// indirect light the G-buffer pass stored, then fog. Sky pixels (depth 0) are left alone.

@group(1) @binding(0) var gb_normals: texture_2d<f32>;
@group(1) @binding(1) var gb_indirect: texture_2d<f32>;
@group(1) @binding(2) var gb_diffuse_code: texture_2d<f32>;
@group(1) @binding(3) var gb_f0_rough: texture_2d<f32>;
@group(1) @binding(4) var gb_depth: texture_depth_2d;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_light(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let xy = vec2i(p.xy);
    let depth = textureLoad(gb_depth, xy, 0);
    if (depth <= 0.0) {
        discard;
    }
    // World position from depth, within the camera's viewport.
    let vp = frame.cluster_viewport;
    let ndc = vec2f((p.x - vp.x) / vp.z * 2.0 - 1.0, 1.0 - (p.y - vp.y) / vp.w * 2.0);
    let h = frame.inv_view_proj * vec4f(ndc, depth, 1.0);
    let world_pos = h.xyz / h.w;
    let normals = textureLoad(gb_normals, xy, 0);
    let dc = textureLoad(gb_diffuse_code, xy, 0);
    let fr = textureLoad(gb_f0_rough, xy, 0);
    let code = u32(dc.a * 15.0 + 0.5);
    let model = code / 3u;
    let flags = code % 3u;
    let indirect = textureLoad(gb_indirect, xy, 0);
    var color = indirect.rgb;
    if (flags != 2u) {
        let bits = u32(indirect.a + 0.5);
        // Bit 8: Mixed lights were already added in the G-buffer pass.
        let ext = DirectExtra(bits & 0xffu, false, vec4f(1.0), select(0u, 2u, bits >= 256u));
        color += direct_light(world_pos, oct_decode(normals.xy), oct_decode(normals.zw), dc.rgb, fr.rgb, fr.a, model, flags == 1u, p.xy, ext);
    }
    return vec4f(apply_fog(color, world_pos), 1.0);
}
