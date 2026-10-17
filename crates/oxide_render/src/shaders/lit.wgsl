// Built-in Lit/Unlit shader (URP Lit equivalent): metallic-roughness PBR, directional sun
// with cascaded soft shadows, point/spot lights, gradient sky ambient, fog.
//
// The material's look comes from two hooks: `vertex_mod` (object-space vertex changes) and
// `surface` (base colour, normal, metallic, ...). The built-in Lit material's hooks are in
// lit_surface.wgsl; a Shader Graph replaces them with generated code (at the SURFACE marker).

struct Material {
    base_color: vec4f,
    emission: vec4f,
    pbr: vec4f,           // metallic, roughness, normal scale, occlusion strength
    tiling_offset: vec4f,
    flags: vec4f,         // alpha cutoff, unlit, cutout, transparent
};

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_map: texture_2d<f32>;
@group(1) @binding(2) var metal_rough_map: texture_2d<f32>;
@group(1) @binding(3) var normal_map: texture_2d<f32>;
@group(1) @binding(4) var occlusion_map: texture_2d<f32>;
@group(1) @binding(5) var emission_map: texture_2d<f32>;
@group(1) @binding(6) var mat_sampler: sampler;
// Shader Graph properties (its textures use bindings 1-5).
@group(1) @binding(7) var<uniform> graph_props: array<vec4f, 32>;

fn sg_time() -> f32 {
    return frame.camera_pos.w;
}

fn sg_camera_pos() -> vec3f {
    return frame.camera_pos.xyz;
}

fn sg_view_matrix() -> mat4x4f {
    return frame.view;
}

// What the surface hook sees for one pixel.
struct SurfaceInput {
    uv0: vec2f,             // mesh UV0, Unity convention (origin bottom-left), untiled
    uv1: vec2f,
    world_pos: vec3f,
    object_pos: vec3f,
    world_normal: vec3f,
    object_normal: vec3f,
    world_tangent: vec3f,
    world_bitangent: vec3f,
    view_dir: vec3f,        // world space, toward the camera
    vertex_color: vec4f,
    screen_pos: vec4f,      // xy 0..1 across the viewport
    front: bool,
    in_vertex: bool,
    object_to_world: mat4x4f,
};

// The surface: Unity's Fragment master stack.
struct Surface {
    base_color: vec3f,
    alpha: f32,
    normal_ts: vec3f,       // tangent space
    metallic: f32,
    smoothness: f32,
    occlusion: f32,
    emission: vec3f,
    alpha_clip: f32,        // discard below this (negative: no clipping)
};

// What the vertex hook sees and returns (object space): Unity's Vertex master stack.
struct VertexInput {
    position: vec3f,
    normal: vec3f,
    tangent: vec3f,
    uv0: vec2f,
    uv1: vec2f,
    color: vec4f,
    object_to_world: mat4x4f, // the model matrix (skinned meshes: the blended skin matrix)
};

struct VertexMod {
    position: vec3f,
    normal: vec3f,
    tangent: vec3f,
};

// Texture rows start at the top; Unity UVs at the bottom.
fn sample_uv(uv: vec2f) -> vec2f {
    return vec2f(uv.x, 1.0 - uv.y);
}

//#SURFACE

struct VertexIn {
    @location(0) position: vec3f,
    @location(1) normal: vec3f,
    @location(2) tangent: vec4f,
    @location(3) uv: vec2f,
    @location(4) uv2: vec2f,
    @location(5) color: vec4f,
    @location(6) m0: vec4f,
    @location(7) m1: vec4f,
    @location(8) m2: vec4f,
    @location(9) m3: vec4f,
    @location(10) n0: vec4f,
    @location(11) n1: vec4f,
    @location(12) n2: vec4f,
    @location(13) extra: vec4u,
};

struct SkinIn {
    @location(14) joint_ids: vec4u,
    @location(15) weights: vec4f,
};

struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) world_pos: vec3f,
    @location(1) normal: vec3f,
    @location(2) tangent: vec4f,
    @location(3) uv: vec2f,
    @location(4) color: vec4f,
    @location(5) @interpolate(flat) receive_shadows: u32,
    @location(6) @interpolate(flat) probes: vec2u,
    @location(7) lm_uv: vec2f,
    @location(8) @interpolate(flat) gi: u32,
    @location(9) obj_pos: vec3f,
    @location(10) uv1: vec2f,
    @location(11) obj_normal: vec3f,
    // Rows of the object-to-world matrix (Shader Graph Object space).
    @location(12) @interpolate(flat) model_r0: vec4f,
    @location(13) @interpolate(flat) model_r1: vec4f,
    @location(14) @interpolate(flat) model_r2: vec4f,
};

fn model_rows(o: ptr<function, VertexOut>, m: mat4x4f) {
    (*o).model_r0 = vec4f(m[0].x, m[1].x, m[2].x, m[3].x);
    (*o).model_r1 = vec4f(m[0].y, m[1].y, m[2].y, m[3].y);
    (*o).model_r2 = vec4f(m[0].z, m[1].z, m[2].z, m[3].z);
}

fn model_from_rows(r0: vec4f, r1: vec4f, r2: vec4f) -> mat4x4f {
    return mat4x4f(
        vec4f(r0.x, r1.x, r2.x, 0.0),
        vec4f(r0.y, r1.y, r2.y, 0.0),
        vec4f(r0.z, r1.z, r2.z, 0.0),
        vec4f(r0.w, r1.w, r2.w, 1.0),
    );
}

fn vertex_input(v: VertexIn, model: mat4x4f) -> VertexInput {
    var i: VertexInput;
    i.object_to_world = model;
    i.position = v.position;
    i.normal = v.normal;
    i.tangent = v.tangent.xyz;
    i.uv0 = v.uv;
    i.uv1 = v.uv2;
    i.color = v.color;
    return i;
}

fn finish_vertex(v: VertexIn, m: VertexMod, model: mat4x4f, world: vec4f, n: vec3f, t: vec3f) -> VertexOut {
    var o: VertexOut;
    model_rows(&o, model);
    o.clip = frame.view_proj * world;
    o.world_pos = world.xyz;
    o.normal = n;
    o.tangent = vec4f(t, v.tangent.w);
    o.uv = v.uv;
    o.uv1 = v.uv2;
    o.obj_pos = m.position;
    o.obj_normal = m.normal;
    o.color = v.color;
    o.receive_shadows = v.extra.y;
    o.probes = v.extra.zw;
    // Baked lighting entry of this draw (stored in the normal matrix's spare float).
    let gi = i32(v.n1.w);
    o.gi = select(NO_GI, u32(gi), gi >= 0);
    if (gi >= 0) {
        let st = instance_gi[gi].lightmap_st;
        o.lm_uv = v.uv2 * st.xy + st.zw;
    }
    return o;
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let model = mat4x4f(v.m0, v.m1, v.m2, v.m3);
    let nmat = mat3x3f(v.n0.xyz, v.n1.xyz, v.n2.xyz);
    let m = vertex_mod(vertex_input(v, model));
    let world = model * vec4f(m.position, 1.0);
    return finish_vertex(v, m, model, world, nmat * m.normal, (model * vec4f(m.tangent, 0.0)).xyz);
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn) -> VertexOut {
    let base = v.extra.x;
    let skin = joints[base + s.joint_ids.x] * s.weights.x
             + joints[base + s.joint_ids.y] * s.weights.y
             + joints[base + s.joint_ids.z] * s.weights.z
             + joints[base + s.joint_ids.w] * s.weights.w;
    let m = vertex_mod(vertex_input(v, skin));
    let world = skin * vec4f(m.position, 1.0);
    let n = (skin * vec4f(m.normal, 0.0)).xyz;
    let t = (skin * vec4f(m.tangent, 0.0)).xyz;
    return finish_vertex(v, m, skin, world, n, t);
}

struct FragOut {
    @location(0) color: vec4f,
};

fn surface_input(in: VertexOut, front: bool) -> SurfaceInput {
    var i: SurfaceInput;
    i.uv0 = in.uv;
    i.uv1 = in.uv1;
    i.world_pos = in.world_pos;
    i.object_pos = in.obj_pos;
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    i.world_normal = n;
    i.object_normal = normalize(in.obj_normal);
    let t_len = length(in.tangent.xyz);
    // Meshes without tangents get an arbitrary frame around the normal.
    var t = normalize(cross(n, select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(n.y) > 0.9)));
    if (t_len > 1e-4) {
        let tt = in.tangent.xyz / t_len;
        let o = tt - n * dot(n, tt);
        if (dot(o, o) > 1e-8) {
            t = normalize(o);
        }
    }
    i.world_tangent = t;
    i.world_bitangent = cross(n, t) * select(1.0, in.tangent.w, t_len > 1e-4);
    i.view_dir = normalize(frame.camera_pos.xyz - in.world_pos);
    i.vertex_color = in.color;
    let vp = frame.cluster_viewport;
    i.screen_pos = vec4f((in.clip.xy - vp.xy) / max(vp.zw, vec2f(1.0)), in.clip.z, 1.0);
    i.front = front;
    i.in_vertex = false;
    i.object_to_world = model_from_rows(in.model_r0, in.model_r1, in.model_r2);
    return i;
}

// Depth prepass of alpha-cutout materials: only the alpha test.
@fragment
fn fs_depth_cutout(in: VertexOut, @builtin(front_facing) front: bool) {
    let s = surface(surface_input(in, front));
    if (s.alpha < s.alpha_clip) {
        discard;
    }
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> FragOut {
    let si = surface_input(in, front);
    let s = surface(si);
    if (s.alpha < s.alpha_clip) {
        discard;
    }
    var out: FragOut;
    let alpha = select(1.0, s.alpha, material.flags.w > 0.5);
    if (material.flags.y > 0.5) {
        out.color = vec4f(apply_fog(s.base_color + s.emission, in.world_pos), alpha);
        return out;
    }
    let geo = normalize(in.normal) * select(-1.0, 1.0, front);
    let tn = s.normal_ts;
    let n = normalize(si.world_tangent * tn.x + si.world_bitangent * tn.y + si.world_normal * max(tn.z, 1e-3));
    let metallic = clamp(s.metallic, 0.0, 1.0);
    let rough = clamp(1.0 - s.smoothness, 0.045, 1.0);
    let ao = clamp(s.occlusion, 0.0, 1.0);
    let albedo = max(s.base_color, vec3f(0.0));
    let mode = debug_mode();
    if (mode != 0u) {
        let c = debug_view(mode, in.world_pos, n, geo, albedo, in.receive_shadows != 0u, in.probes, in.clip.xy, in.gi, in.lm_uv);
        out.color = vec4f(c, 1.0);
        return out;
    }
    var color = shade(in.world_pos, n, geo, albedo, metallic, rough, ao, in.receive_shadows != 0u, in.probes, in.clip.xy, in.gi, in.lm_uv);
    color += s.emission;
    out.color = vec4f(apply_fog(color, in.world_pos), alpha);
    return out;
}

// Wireframe draw modes: mesh edges.
@fragment
fn fs_wire(in: VertexOut) -> FragOut {
    var out: FragOut;
    out.color = vec4f(0.0, 0.0, 0.0, 0.55);
    return out;
}
