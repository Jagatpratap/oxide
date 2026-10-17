// Camera-facing particle billboards (instanced quads), alpha or additive.

@group(1) @binding(0) var ptex: texture_2d<f32>;
@group(1) @binding(1) var psamp: sampler;

struct PIn {
    @location(0) pos: vec3f,
    @location(1) size: f32,
    @location(2) color: vec4f,
    @location(3) rotation: f32,
    // 0 billboard, 1 stretched, 2 horizontal, 3 vertical, 4 ribbon segment.
    @location(4) mode: f32,
    // (5 is padding.)
    @location(6) uv_rect: vec4f,
    @location(7) axis: vec3f,
    @location(8) aspect: f32,
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
    let half = vec2f(p.size * 0.5, p.size * 0.5 * p.aspect);
    let r = vec2f(c.x * cs - c.y * sn, c.x * sn + c.y * cs) * half;
    let to_cam = normalize(frame.camera_pos.xyz - p.pos);
    let mode = u32(p.mode + 0.5);
    var world: vec3f;
    if mode == 1u || mode == 4u {
        // Along the axis (velocity or segment), turned to face the camera around it.
        let len = length(p.axis);
        var d = vec3f(0.0, 1.0, 0.0);
        if len > 1e-6 {
            d = p.axis / len;
        }
        var side = cross(d, to_cam);
        if length(side) < 1e-5 {
            side = vec3f(frame.view[0].x, frame.view[1].x, frame.view[2].x);
        }
        side = normalize(side);
        world = p.pos + side * c.x * p.size * 0.5 + p.axis * c.y;
    } else if mode == 2u {
        world = p.pos + vec3f(r.x, 0.0, r.y);
    } else if mode == 3u {
        let up = vec3f(0.0, 1.0, 0.0);
        var right = cross(up, to_cam);
        if length(right) < 1e-5 {
            right = vec3f(1.0, 0.0, 0.0);
        }
        world = p.pos + normalize(right) * r.x + up * r.y;
    } else {
        // Camera right/up in world space are the first two rows of the view matrix.
        let right = vec3f(frame.view[0].x, frame.view[1].x, frame.view[2].x);
        let up = vec3f(frame.view[0].y, frame.view[1].y, frame.view[2].y);
        world = p.pos + right * r.x + up * r.y;
    }
    var o: POut;
    o.clip = frame.view_proj * vec4f(world, 1.0);
    // The texture rectangle (top-left origin); quad y runs up.
    let t = c * 0.5 + 0.5;
    o.uv = p.uv_rect.xy + vec2f(t.x, 1.0 - t.y) * p.uv_rect.zw;
    o.color = p.color;
    o.world_pos = world;
    return o;
}

@fragment
fn fs_main(in: POut) -> @location(0) vec4f {
    let t = textureSample(ptex, psamp, in.uv);
    var c = t * in.color;
    // Fog fades particles like everything else.
    c = vec4f(apply_fog(c.rgb, in.world_pos), c.a);
    return c;
}
