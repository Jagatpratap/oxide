// Built-in Lit/Unlit shader (URP Lit equivalent): metallic-roughness PBR, directional sun
// with cascaded soft shadows, point/spot lights, gradient sky ambient, fog.

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
};

fn finish_vertex(v: VertexIn, world: vec4f, n: vec3f, t: vec3f) -> VertexOut {
    var o: VertexOut;
    o.clip = frame.view_proj * world;
    o.world_pos = world.xyz;
    o.normal = n;
    o.tangent = vec4f(t, v.tangent.w);
    o.uv = v.uv * material.tiling_offset.xy + material.tiling_offset.zw;
    o.color = v.color;
    o.receive_shadows = v.extra.y;
    o.probes = v.extra.zw;
    return o;
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let model = mat4x4f(v.m0, v.m1, v.m2, v.m3);
    let nmat = mat3x3f(v.n0.xyz, v.n1.xyz, v.n2.xyz);
    let world = model * vec4f(v.position, 1.0);
    return finish_vertex(v, world, nmat * v.normal, (model * vec4f(v.tangent.xyz, 0.0)).xyz);
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn) -> VertexOut {
    let base = v.extra.x;
    let skin = joints[base + s.joint_ids.x] * s.weights.x
             + joints[base + s.joint_ids.y] * s.weights.y
             + joints[base + s.joint_ids.z] * s.weights.z
             + joints[base + s.joint_ids.w] * s.weights.w;
    let world = skin * vec4f(v.position, 1.0);
    let n = (skin * vec4f(v.normal, 0.0)).xyz;
    let t = (skin * vec4f(v.tangent.xyz, 0.0)).xyz;
    return finish_vertex(v, world, n, t);
}

struct FragOut {
    @location(0) color: vec4f,
};

fn sample_uv(uv: vec2f) -> vec2f {
    // Unity UVs have a bottom-left origin; texture rows start at the top.
    return vec2f(uv.x, 1.0 - uv.y);
}

// Depth prepass of alpha-cutout materials: only the alpha test.
@fragment
fn fs_depth_cutout(in: VertexOut) {
    let a = material.base_color.a * textureSample(base_map, mat_sampler, sample_uv(in.uv)).a * in.color.a;
    if (a < material.flags.x) {
        discard;
    }
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> FragOut {
    let uv = sample_uv(in.uv);
    var base = material.base_color * textureSample(base_map, mat_sampler, uv) * in.color;
    if (material.flags.z > 0.5 && base.a < material.flags.x) {
        discard;
    }
    let emission = material.emission.rgb * textureSample(emission_map, mat_sampler, uv).rgb;
    var out: FragOut;
    if (material.flags.y > 0.5) {
        let c = apply_fog(base.rgb + emission, in.world_pos);
        out.color = vec4f(c, select(1.0, base.a, material.flags.w > 0.5));
        return out;
    }

    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let t_len = length(in.tangent.xyz);
    if (t_len > 1e-4) {
        var t = in.tangent.xyz / t_len;
        t = normalize(t - n * dot(n, t));
        let b = cross(n, t) * in.tangent.w;
        // Tangent-space normal from X/Y (works for RGB and two-channel BC5 maps), as
        // Unity's UnpackNormalScale: scale X/Y, then rebuild Z.
        let nxy = (textureSample(normal_map, mat_sampler, uv).xy * 2.0 - 1.0) * material.pbr.z;
        let tn = vec3f(nxy, sqrt(clamp(1.0 - dot(nxy, nxy), 0.0, 1.0)));
        n = normalize(t * tn.x + b * tn.y + n * max(tn.z, 1e-3));
    }

    let mr = textureSample(metal_rough_map, mat_sampler, uv);
    let metallic = clamp(material.pbr.x * mr.b, 0.0, 1.0);
    let rough = clamp(material.pbr.y * mr.g, 0.045, 1.0);
    let ao = mix(1.0, textureSample(occlusion_map, mat_sampler, uv).r, material.pbr.w);

    let albedo = base.rgb;
    var color = shade(in.world_pos, n, normalize(in.normal), albedo, metallic, rough, ao, in.receive_shadows != 0u, in.probes, in.clip.xy);
    color += emission;

    color = apply_fog(color, in.world_pos);
    out.color = vec4f(color, select(1.0, base.a, material.flags.w > 0.5));
    return out;
}
