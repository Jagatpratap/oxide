// Terrain: four texture layers blended by a splat (control) map, tiled in world space,
// lit with the shared PBR lighting (sun + shadows, punctual lights, sky ambient, fog).

struct Terrain {
    size: vec4f,            // world size xyz, w = layer count
    tiles: array<vec4f, 4>, // 1/tile x, 1/tile z, smoothness, metallic
    tints: array<vec4f, 4>,
};

@group(1) @binding(0) var<uniform> terrain: Terrain;
@group(1) @binding(1) var control_map: texture_2d<f32>;
@group(1) @binding(2) var layer_maps: texture_2d_array<f32>;
@group(1) @binding(3) var layer_sampler: sampler;
@group(1) @binding(4) var control_sampler: sampler;

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

struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) world_pos: vec3f,
    @location(1) normal: vec3f,
    @location(2) uv: vec2f,
    @location(3) @interpolate(flat) receive_shadows: u32,
    @location(4) @interpolate(flat) probes: vec2u,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let model = mat4x4f(v.m0, v.m1, v.m2, v.m3);
    let nmat = mat3x3f(v.n0.xyz, v.n1.xyz, v.n2.xyz);
    let world = model * vec4f(v.position, 1.0);
    var o: VertexOut;
    o.clip = frame.view_proj * world;
    o.world_pos = world.xyz;
    o.normal = nmat * v.normal;
    o.uv = v.uv;
    o.receive_shadows = v.extra.y;
    o.probes = v.extra.zw;
    return o;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    // Control map rows go along +Z (bottom-left origin like the heightmap).
    var w = textureSample(control_map, control_sampler, vec2f(in.uv.x, in.uv.y));
    w = w / max(w.r + w.g + w.b + w.a, 1e-3);
    let count = i32(terrain.size.w);
    var albedo = vec3f(0.0);
    var smooth_ = 0.0;
    var metal = 0.0;
    let weights = array<f32, 4>(w.r, w.g, w.b, w.a);
    for (var i = 0; i < 4; i++) {
        if (i >= count || weights[i] <= 0.001) {
            continue;
        }
        let t = terrain.tiles[i];
        // World-space tiling, with a second, larger scale mixed in to hide repetition.
        let uv = in.world_pos.xz * t.xy;
        let c1 = textureSample(layer_maps, layer_sampler, uv, i).rgb;
        let c2 = textureSample(layer_maps, layer_sampler, uv * 0.13 + vec2f(0.37, 0.71), i).rgb;
        let c = mix(c1, c1 * c2 * 2.0, 0.35) * terrain.tints[i].rgb;
        albedo += c * weights[i];
        smooth_ += t.z * weights[i];
        metal += t.w * weights[i];
    }
    let n = normalize(in.normal);
    var color = shade(in.world_pos, n, n, albedo, metal, clamp(1.0 - smooth_, 0.045, 1.0), 1.0, in.receive_shadows != 0u, in.probes);
    color = apply_fog(color, in.world_pos);
    return vec4f(color, 1.0);
}
