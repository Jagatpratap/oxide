// Terrain grass and detail billboards (Unity's Grass / Grass Billboard details): instanced,
// bottom-anchored quads — two crossed quads per instance, or one camera-facing quad —
// swaying with the terrain wind, alpha-tested, lit like the terrain.

@group(1) @binding(0) var gtex: texture_2d<f32>;
@group(1) @binding(1) var gsamp: sampler;

struct GIn {
    @location(0) pos: vec3f,
    @location(1) width: f32,
    @location(2) height: f32,
    @location(3) rotation: f32,
    @location(4) wind: f32,
    @location(5) mode: f32,
    @location(6) color: vec4f,   // rgb tint, a = wind speed
};

struct GOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) color: vec3f,
    @location(2) world_pos: vec3f,
    @location(3) normal: vec3f,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, g: GIn) -> GOut {
    var corners = array<vec2f, 6>(vec2f(-0.5, 0.0), vec2f(0.5, 0.0), vec2f(0.5, 1.0), vec2f(-0.5, 0.0), vec2f(0.5, 1.0), vec2f(-0.5, 1.0));
    let quad = vi / 6u;
    let c = corners[vi % 6u];
    var right: vec3f;
    if (g.mode > 0.5) {
        // Billboard: faces the camera around the vertical axis; the second quad collapses.
        let cr = vec3f(frame.view[0].x, frame.view[1].x, frame.view[2].x);
        right = normalize(vec3f(cr.x, 0.0, cr.z) + vec3f(1e-4, 0.0, 0.0));
        if (quad == 1u) {
            right = vec3f(0.0);
        }
    } else {
        let a = g.rotation + f32(quad) * 1.5707963;
        right = vec3f(cos(a), 0.0, sin(a));
    }
    // Wind: blades bend more toward the tip.
    let t = frame.camera_pos.w * (0.5 + g.color.a * 2.5);
    let phase = dot(g.pos.xz, vec2f(0.11, 0.17));
    let sway = vec3f(sin(t + phase), 0.0, cos(t * 0.7 + phase * 1.3)) * g.wind * c.y * c.y * g.height;
    let world = g.pos + right * c.x * g.width + vec3f(0.0, c.y * g.height, 0.0) + sway;
    var o: GOut;
    o.clip = frame.view_proj * vec4f(world, 1.0);
    o.uv = vec2f(c.x + 0.5, c.y);
    o.color = g.color.rgb;
    o.world_pos = world;
    // Mostly up, so grass is lit like the ground under it.
    o.normal = normalize(vec3f(0.0, 1.0, 0.0) + cross(right, vec3f(0.0, 1.0, 0.0)) * 0.3);
    return o;
}

@fragment
fn fs_main(in: GOut, @builtin(front_facing) front: bool) -> @location(0) vec4f {
    let t = textureSample(gtex, gsamp, vec2f(in.uv.x, 1.0 - in.uv.y));
    if (t.a < 0.5) {
        discard;
    }
    let albedo = t.rgb * in.color;
    let n = normalize(in.normal);
    var color = shade(in.world_pos, n, n, albedo, 0.0, 0.9, 1.0, true, vec2u(0u, 0u), in.clip.xy, NO_GI, vec2f(0.0));
    // Light shining through the blades.
    color += albedo * frame.sun_color.rgb * 0.08;
    color = apply_fog(color, in.world_pos);
    return vec4f(color, 1.0);
}
