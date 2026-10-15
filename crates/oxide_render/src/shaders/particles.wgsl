// Camera-facing particle billboards (instanced quads), alpha or additive.

@group(1) @binding(0) var ptex: texture_2d<f32>;
@group(1) @binding(1) var psamp: sampler;

struct PIn {
    @location(0) pos: vec3f,
    @location(1) size: f32,
    @location(2) color: vec4f,
    @location(3) rotation: f32,
};

struct POut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) color: vec4f,
    @location(2) world_pos: vec3f,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, p: PIn) -> POut {
    var corners = array<vec2f, 6>(vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0));
    let c = corners[vi];
    let cs = cos(p.rotation);
    let sn = sin(p.rotation);
    let r = vec2f(c.x * cs - c.y * sn, c.x * sn + c.y * cs);
    // Camera right/up in world space are the first two rows of the view matrix.
    let right = vec3f(frame.view[0].x, frame.view[1].x, frame.view[2].x);
    let up = vec3f(frame.view[0].y, frame.view[1].y, frame.view[2].y);
    let world = p.pos + (right * r.x + up * r.y) * p.size * 0.5;
    var o: POut;
    o.clip = frame.view_proj * vec4f(world, 1.0);
    o.uv = c * 0.5 + 0.5;
    o.color = p.color;
    o.world_pos = world;
    return o;
}

@fragment
fn fs_main(in: POut) -> @location(0) vec4f {
    let t = textureSample(ptex, psamp, vec2f(in.uv.x, 1.0 - in.uv.y));
    var c = t * in.color;
    // Fog fades particles like everything else.
    c = vec4f(apply_fog(c.rgb, in.world_pos), c.a);
    return c;
}
