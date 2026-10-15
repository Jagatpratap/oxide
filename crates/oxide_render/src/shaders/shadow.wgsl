// Shadow caster pass: depth only, with alpha-clipped variant for cutout materials.

struct ShadowView {
    view_proj: mat4x4f,
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
@group(1) @binding(6) var mat_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3f,
    @location(3) uv: vec2f,
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
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let model = mat4x4f(v.m0, v.m1, v.m2, v.m3);
    var o: VertexOut;
    o.clip = shadow_view.view_proj * (model * vec4f(v.position, 1.0));
    o.uv = v.uv * material.tiling_offset.xy + material.tiling_offset.zw;
    return o;
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn) -> VertexOut {
    let base = v.extra.x;
    let skin = joints[base + s.joint_ids.x] * s.weights.x
             + joints[base + s.joint_ids.y] * s.weights.y
             + joints[base + s.joint_ids.z] * s.weights.z
             + joints[base + s.joint_ids.w] * s.weights.w;
    var o: VertexOut;
    o.clip = shadow_view.view_proj * (skin * vec4f(v.position, 1.0));
    o.uv = v.uv * material.tiling_offset.xy + material.tiling_offset.zw;
    return o;
}

@fragment
fn fs_cutout(in: VertexOut) {
    let a = material.base_color.a * textureSample(base_map, mat_sampler, vec2f(in.uv.x, 1.0 - in.uv.y)).a;
    if (a < material.flags.x) {
        discard;
    }
}
