// Terrain: up to 16 texture layers blended by RGBA control (splat) maps — four layers per
// map — tiled in world space with optional normal maps, holes cut by a hole map, lit with
// the shared PBR lighting (sun + shadows, punctual lights, sky ambient, fog).

struct Terrain {
    size: vec4f,              // world size xyz, w = layer count
    flags: vec4f,             // x = has holes, y = control map count
    tiles: array<vec4f, 16>,  // 1/tile x, 1/tile z, smoothness, metallic
    offsets: array<vec4f, 16>,// tile offset xy, normal scale, has normal map
    tints: array<vec4f, 16>,
};

@group(1) @binding(0) var<uniform> terrain: Terrain;
@group(1) @binding(1) var control_maps: texture_2d_array<f32>;
@group(1) @binding(2) var layer_maps: texture_2d_array<f32>;
@group(1) @binding(3) var layer_sampler: sampler;
@group(1) @binding(4) var control_sampler: sampler;
@group(1) @binding(5) var normal_maps: texture_2d_array<f32>;
@group(1) @binding(6) var hole_map: texture_2d<f32>;
@group(1) @binding(7) var hole_sampler: sampler;

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
    // Holes: one texel per heightmap cell, nearest sampling.
    if (terrain.flags.x > 0.5 && textureSample(hole_map, hole_sampler, in.uv).r < 0.5) {
        discard;
    }
    let count = i32(terrain.size.w);
    let maps = max(i32(terrain.flags.y), 1);
    // Control map rows go along +Z (bottom-left origin like the heightmap).
    var weights: array<f32, 16>;
    var total = 0.0;
    for (var m = 0; m < 4; m++) {
        if (m >= maps) {
            break;
        }
        let w = textureSample(control_maps, control_sampler, in.uv, m);
        weights[m * 4] = w.r;
        weights[m * 4 + 1] = w.g;
        weights[m * 4 + 2] = w.b;
        weights[m * 4 + 3] = w.a;
        total += w.r + w.g + w.b + w.a;
    }
    // Nothing painted: the first layer.
    if (total < 1e-3) {
        weights[0] = 1.0;
        total = 1.0;
    }
    let inv = 1.0 / total;
    let geo_n = normalize(in.normal);
    // Tangent frame for world-XZ tiling: +X and +Z projected onto the surface.
    let t = normalize(vec3f(1.0, 0.0, 0.0) - geo_n * geo_n.x);
    let b = normalize(cross(t, geo_n));
    var albedo = vec3f(0.0);
    var nrm = vec3f(0.0);
    var smooth_ = 0.0;
    var metal = 0.0;
    for (var i = 0; i < 16; i++) {
        if (i >= count) {
            break;
        }
        let wi = weights[i] * inv;
        if (wi <= 0.001) {
            continue;
        }
        let tl = terrain.tiles[i];
        let off = terrain.offsets[i];
        // World-space tiling, with a second, larger scale mixed in to hide repetition.
        let uv = in.world_pos.xz * tl.xy + off.xy * tl.xy;
        let c1 = textureSample(layer_maps, layer_sampler, uv, i).rgb;
        let c2 = textureSample(layer_maps, layer_sampler, uv * 0.13 + vec2f(0.37, 0.71), i).rgb;
        let c = mix(c1, c1 * c2 * 2.0, 0.35) * terrain.tints[i].rgb;
        albedo += c * wi;
        smooth_ += tl.z * wi;
        metal += tl.w * wi;
        if (off.w > 0.5) {
            var tn = textureSample(normal_maps, layer_sampler, uv, i).xyz * 2.0 - 1.0;
            tn = vec3f(tn.xy * off.z, tn.z);
            nrm += (t * tn.x - b * tn.y + geo_n * tn.z) * wi;
        } else {
            nrm += geo_n * wi;
        }
    }
    let n = normalize(select(geo_n, nrm, dot(nrm, nrm) > 1e-6));
    let mode = debug_mode();
    if (mode != 0u) {
        return vec4f(debug_view(mode, in.world_pos, n, geo_n, albedo, in.receive_shadows != 0u, in.probes, in.clip.xy, NO_GI, vec2f(0.0)), 1.0);
    }
    var color = shade(in.world_pos, n, geo_n, albedo, metal, clamp(1.0 - smooth_, 0.045, 1.0), 1.0, in.receive_shadows != 0u, in.probes, in.clip.xy, NO_GI, vec2f(0.0));
    color = apply_fog(color, in.world_pos);
    return vec4f(color, 1.0);
}
