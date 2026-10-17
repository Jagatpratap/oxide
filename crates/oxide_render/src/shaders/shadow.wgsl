// Shadow caster pass: depth only, with an alpha-clipped variant for cutout materials. The
// same hooks as lit.wgsl (`vertex_mod`, `surface`) make Shader Graph vertex animation and
// alpha clipping cast matching shadows (hooks go at the SURFACE marker).

struct ShadowView {
    view_proj: mat4x4f,
    params: vec4f,          // x = time
};

@group(0) @binding(0) var<uniform> shadow_view: ShadowView;
@group(0) @binding(1) var<storage, read> joints: array<mat4x4f>;

struct Material {
    base_color: vec4f,
    emission: vec4f,
    pbr: vec4f,
    tiling_offset: vec4f,
    flags: vec4f,
};
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_map: texture_2d<f32>;
@group(1) @binding(2) var metal_rough_map: texture_2d<f32>;
@group(1) @binding(3) var normal_map: texture_2d<f32>;
@group(1) @binding(4) var occlusion_map: texture_2d<f32>;
@group(1) @binding(5) var emission_map: texture_2d<f32>;
@group(1) @binding(6) var mat_sampler: sampler;
@group(1) @binding(7) var<uniform> graph_props: array<vec4f, 32>;

fn sg_time() -> f32 {
    return shadow_view.params.x;
}

fn sg_camera_pos() -> vec3f {
    return vec3f(0.0);
}

fn sg_view_matrix() -> mat4x4f {
    return mat4x4f(vec4f(1.0, 0.0, 0.0, 0.0), vec4f(0.0, 1.0, 0.0, 0.0), vec4f(0.0, 0.0, 1.0, 0.0), vec4f(0.0, 0.0, 0.0, 1.0));
}

fn sg_view_proj_matrix() -> mat4x4f {
    return shadow_view.view_proj;
}

struct SurfaceInput {
    uv0: vec2f,
    uv1: vec2f,
    world_pos: vec3f,
    object_pos: vec3f,
    world_normal: vec3f,
    object_normal: vec3f,
    world_tangent: vec3f,
    world_bitangent: vec3f,
    view_dir: vec3f,
    vertex_color: vec4f,
    screen_pos: vec4f,
    front: bool,
    in_vertex: bool,
    object_to_world: mat4x4f,
};

struct Surface {
    base_color: vec3f,
    alpha: f32,
    normal_ts: vec3f,
    metallic: f32,
    smoothness: f32,
    occlusion: f32,
    emission: vec3f,
    alpha_clip: f32,
};

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
    @location(13) extra: vec4u,
};

struct SkinIn {
    @location(14) joint_ids: vec4u,
    @location(15) weights: vec4f,
};

struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) uv1: vec2f,
    @location(2) world_pos: vec3f,
    @location(3) obj_pos: vec3f,
    @location(4) normal: vec3f,
    @location(5) color: vec4f,
    @location(6) @interpolate(flat) model_r0: vec4f,
    @location(7) @interpolate(flat) model_r1: vec4f,
    @location(8) @interpolate(flat) model_r2: vec4f,
};

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

fn finish(v: VertexIn, m: VertexMod, model: mat4x4f, world: vec4f, n: vec3f) -> VertexOut {
    var o: VertexOut;
    o.model_r0 = vec4f(model[0].x, model[1].x, model[2].x, model[3].x);
    o.model_r1 = vec4f(model[0].y, model[1].y, model[2].y, model[3].y);
    o.model_r2 = vec4f(model[0].z, model[1].z, model[2].z, model[3].z);
    o.clip = shadow_view.view_proj * world;
    o.uv = v.uv;
    o.uv1 = v.uv2;
    o.world_pos = world.xyz;
    o.obj_pos = m.position;
    o.normal = n;
    o.color = v.color;
    return o;
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let model = mat4x4f(v.m0, v.m1, v.m2, v.m3);
    let m = vertex_mod(vertex_input(v, model));
    return finish(v, m, model, model * vec4f(m.position, 1.0), (model * vec4f(m.normal, 0.0)).xyz);
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn) -> VertexOut {
    let base = v.extra.x;
    let skin = joints[base + s.joint_ids.x] * s.weights.x
             + joints[base + s.joint_ids.y] * s.weights.y
             + joints[base + s.joint_ids.z] * s.weights.z
             + joints[base + s.joint_ids.w] * s.weights.w;
    let m = vertex_mod(vertex_input(v, skin));
    return finish(v, m, skin, skin * vec4f(m.position, 1.0), (skin * vec4f(m.normal, 0.0)).xyz);
}

@fragment
fn fs_cutout(in: VertexOut, @builtin(front_facing) front: bool) {
    var i: SurfaceInput;
    i.uv0 = in.uv;
    i.uv1 = in.uv1;
    i.world_pos = in.world_pos;
    i.object_pos = in.obj_pos;
    i.world_normal = normalize(in.normal);
    i.object_normal = i.world_normal;
    i.world_tangent = vec3f(1.0, 0.0, 0.0);
    i.world_bitangent = vec3f(0.0, 0.0, 1.0);
    i.view_dir = vec3f(0.0, 1.0, 0.0);
    i.vertex_color = in.color;
    i.screen_pos = vec4f(0.0, 0.0, 0.0, 1.0);
    i.front = front;
    i.in_vertex = false;
    let r0 = in.model_r0;
    let r1 = in.model_r1;
    let r2 = in.model_r2;
    i.object_to_world = mat4x4f(
        vec4f(r0.x, r1.x, r2.x, 0.0),
        vec4f(r0.y, r1.y, r2.y, 0.0),
        vec4f(r0.z, r1.z, r2.z, 0.0),
        vec4f(r0.w, r1.w, r2.w, 1.0),
    );
    let s = surface(i);
    if (s.alpha < s.alpha_clip) {
        discard;
    }
}
