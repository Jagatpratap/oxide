// Sprites (SpriteRenderer, tiles): instanced quads, alpha blended.

@group(1) @binding(0) var stex: texture_2d<f32>;
@group(1) @binding(1) var ssamp: sampler;

struct SIn {
    @location(0) m0: vec4f,
    @location(1) m1: vec4f,
    @location(2) m2: vec4f,
    @location(3) m3: vec4f,
    @location(4) uv_rect: vec4f,
    @location(5) color: vec4f,
};

struct SOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) color: vec4f,
    @location(2) world_pos: vec3f,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, s: SIn) -> SOut {
    var corners = array<vec2f, 6>(vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0), vec2f(0.0, 0.0), vec2f(1.0, 1.0), vec2f(0.0, 1.0));
    let c = corners[vi];
    let model = mat4x4f(s.m0, s.m1, s.m2, s.m3);
    let world = model * vec4f(c, 0.0, 1.0);
    var o: SOut;
    o.clip = frame.view_proj * world;
    // Texture rows run top to bottom; sprite space runs bottom to top.
    o.uv = vec2f(s.uv_rect.x + c.x * s.uv_rect.z, s.uv_rect.y + (1.0 - c.y) * s.uv_rect.w);
    o.color = s.color;
    o.world_pos = world.xyz;
    return o;
}

@fragment
fn fs_main(in: SOut) -> @location(0) vec4f {
    let t = textureSample(stex, ssamp, in.uv);
    let c = t * in.color;
    if c.a < 0.002 {
        discard;
    }
    return vec4f(apply_fog(c.rgb, in.world_pos), c.a);
}
