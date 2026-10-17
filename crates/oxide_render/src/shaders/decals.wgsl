// URP Decal Projector, DBuffer technique: each decal's box is drawn over the screen; pixels
// whose scene depth (from the depth prepass) falls inside the box take the decal's colour.
// The result (premultiplied albedo, coverage) is blended into Lit surfaces before lighting.

struct Decal {
    to_local: mat4x4f,   // world -> unit box (-0.5..0.5), projecting along +Z
    to_world: mat4x4f,
    color: vec4f,        // linear colour × opacity in a
    uv: vec4f,           // scale xy, offset zw
    fade: vec4f,         // x start angle (deg), y end angle (deg), z unused, w unused
    axis: vec4f,         // world projection direction
};

@group(1) @binding(0) var<uniform> decal: Decal;
@group(1) @binding(1) var dtex: texture_2d<f32>;
@group(1) @binding(2) var dsamp: sampler;
@group(1) @binding(3) var depth_tex: texture_depth_2d;

struct DOut {
    @builtin(position) clip: vec4f,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> DOut {
    // A unit cube, 12 triangles.
    var idx = array<u32, 36>(
        0u, 2u, 1u, 0u, 3u, 2u, 4u, 5u, 6u, 4u, 6u, 7u,
        0u, 1u, 5u, 0u, 5u, 4u, 2u, 3u, 7u, 2u, 7u, 6u,
        1u, 2u, 6u, 1u, 6u, 5u, 0u, 4u, 7u, 0u, 7u, 3u,
    );
    let c = idx[vi];
    let local = vec3f(
        select(-0.5, 0.5, (c & 1u) != (c >> 1u & 1u)),
        select(-0.5, 0.5, (c & 2u) != 0u),
        select(-0.5, 0.5, (c & 4u) != 0u),
    );
    var o: DOut;
    o.clip = frame.view_proj * (decal.to_world * vec4f(local, 1.0));
    return o;
}

@fragment
fn fs_main(in: DOut) -> @location(0) vec4f {
    let px = vec2i(in.clip.xy);
    let d = textureLoad(depth_tex, px, 0);
    let size = max(frame.misc.zw, vec2f(1.0));
    let suv = in.clip.xy / size;
    let ndc = vec4f(suv.x * 2.0 - 1.0, 1.0 - suv.y * 2.0, d, 1.0);
    let wp4 = frame.inv_view_proj * ndc;
    let wp = wp4.xyz / wp4.w;
    // Surface normal from the depth (derivatives before any discard).
    var n = normalize(cross(dpdy(wp), dpdx(wp)));
    // Visible surfaces face the camera.
    if dot(n, frame.camera_pos.xyz - wp) < 0.0 {
        n = -n;
    }
    // Reverse-Z: 0 is the far plane (nothing drawn there).
    if d <= 0.0 {
        discard;
    }
    let local = (decal.to_local * vec4f(wp, 1.0)).xyz;
    if any(abs(local) > vec3f(0.5)) {
        discard;
    }
    // Fade on surfaces turned away from the projector (0° facing it, 180° facing away).
    let facing = clamp(dot(n, -decal.axis.xyz), -1.0, 1.0);
    let angle = degrees(acos(facing));
    var fade = 1.0;
    if decal.fade.y > decal.fade.x {
        fade = 1.0 - clamp((angle - decal.fade.x) / (decal.fade.y - decal.fade.x), 0.0, 1.0);
    }
    let uv = vec2f(local.x + 0.5, 0.5 - local.y) * decal.uv.xy + decal.uv.zw;
    let c = textureSampleLevel(dtex, dsamp, uv, 0.0) * decal.color;
    let a = clamp(c.a * fade, 0.0, 1.0);
    return vec4f(c.rgb * a, a);
}
