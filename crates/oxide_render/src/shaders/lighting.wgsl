// ---- lighting -------------------------------------------------------------------------

// Clustered lights (Forward+): per cluster `[offset, count]` into the light index list.
@group(0) @binding(9) var<storage, read> cluster_cells: array<vec2u>;
@group(0) @binding(10) var<storage, read> cluster_lights: array<u32>;

const CLUSTER_X: i32 = 16;
const CLUSTER_Y: i32 = 9;
const CLUSTER_Z: i32 = 24;

fn cluster_of(frag_xy: vec2f, view_z: f32) -> vec2u {
    let vp = frame.cluster_viewport;
    let t = vec2i(floor((frag_xy - vp.xy) / vp.zw * vec2f(f32(CLUSTER_X), f32(CLUSTER_Y))));
    let tc = clamp(t, vec2i(0), vec2i(CLUSTER_X - 1, CLUSTER_Y - 1));
    let s = clamp(i32(floor(log(max(view_z, 1e-4)) * frame.cluster_params.x + frame.cluster_params.y)), 0, CLUSTER_Z - 1);
    return cluster_cells[(s * CLUSTER_Y + tc.y) * CLUSTER_X + tc.x];
}

// Point/spot light shadows: tiles of one atlas (8 × 8 tiles of 512 px).
@group(0) @binding(11) var<storage, read> shadow_tiles: array<mat4x4f>;
@group(0) @binding(12) var shadow_atlas: texture_depth_2d;

const ATLAS_PER_ROW: u32 = 8u;
const ATLAS_TILE_PX: f32 = 512.0;

fn punctual_shadow(L: PunctualLight, world_pos: vec3f, n: vec3f, dist: f32) -> f32 {
    var tile = u32(L.spot.z) - 1u;
    if (L.color.w < 0.5) {
        // Point light: the cube face (tile) facing the fragment.
        let d = world_pos - L.pos_range.xyz;
        let a = abs(d);
        if (a.x >= a.y && a.x >= a.z) {
            tile += select(1u, 0u, d.x > 0.0);
        } else if (a.y >= a.z) {
            tile += select(3u, 2u, d.y > 0.0);
        } else {
            tile += select(5u, 4u, d.z > 0.0);
        }
    }
    // Normal offset scaled by the shadow texel size at this distance.
    let texel = dist * L.shadow.w;
    let p = shadow_tiles[tile] * vec4f(world_pos + n * L.shadow.y * texel * 1.5, 1.0);
    if (p.w <= 0.0) {
        return 1.0;
    }
    let ndc = p.xyz / p.w;
    if (ndc.z > 1.0) {
        return 1.0;
    }
    let border = 2.0 / ATLAS_TILE_PX;
    let local = clamp(vec2f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5), vec2f(border), vec2f(1.0 - border));
    let tile_uv = 1.0 / f32(ATLAS_PER_ROW);
    let uv = (vec2f(f32(tile % ATLAS_PER_ROW), f32(tile / ATLAS_PER_ROW)) + local) * tile_uv;
    let depth = ndc.z - L.shadow.x * 1e-5;
    var lit: f32;
    if (L.shadow.z > 0.5) {
        let ts = tile_uv / ATLAS_TILE_PX;
        lit = 0.0;
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                lit += textureSampleCompareLevel(shadow_atlas, shadow_sampler, uv + vec2f(f32(x), f32(y)) * ts, depth);
            }
        }
        lit /= 9.0;
    } else {
        lit = textureSampleCompareLevel(shadow_atlas, shadow_sampler, uv, depth);
    }
    return mix(1.0, lit, L.spot.w);
}

// Baked lighting (lightmaps and light probes) per draw, indexed by the instance.
struct InstanceGi {
    lightmap_st: vec4f,     // Unity lightmapScaleOffset
    info: vec4f,            // x lightmap index (-1 none), y 1 when `sh` holds light probe data
    sh: array<vec4f, 9>,
};

@group(0) @binding(13) var lightmaps: texture_2d_array<f32>;
@group(0) @binding(14) var<storage, read> instance_gi: array<InstanceGi>;

const NO_GI: u32 = 0xffffffffu;

// Diffuse light from a lightmap (gamma-space RGBM, range 8) or light probe SH, else the sky.
fn diffuse_light(n: vec3f, gi: u32, lm_uv: vec2f) -> vec3f {
    if (gi != NO_GI) {
        let g = instance_gi[gi];
        if (g.info.x >= 0.0) {
            let p = textureSampleLevel(lightmaps, probe_sampler, vec2f(lm_uv.x, 1.0 - lm_uv.y), i32(g.info.x), 0.0);
            let s = p.rgb * p.a * 2.828427;
            return s * s;
        }
        if (g.info.y > 0.5) {
            let c = g.sh;
            let e = c[0].rgb * 0.282095
                + c[1].rgb * (0.488603 * n.y)
                + c[2].rgb * (0.488603 * n.z)
                + c[3].rgb * (0.488603 * n.x)
                + c[4].rgb * (1.092548 * n.x * n.y)
                + c[5].rgb * (1.092548 * n.y * n.z)
                + c[6].rgb * (0.315392 * (3.0 * n.z * n.z - 1.0))
                + c[7].rgb * (1.092548 * n.x * n.z)
                + c[8].rgb * (0.546274 * (n.x * n.x - n.y * n.y));
            return max(e, vec3f(0.0));
        }
    }
    return ambient_diffuse(n);
}

// Screen-space ambient occlusion of this camera (white when off).
@group(2) @binding(0) var ssao_tex: texture_2d<f32>;

fn screen_ao(frag_xy: vec2f) -> f32 {
    if (frame.ssao.x < 0.5) {
        return 1.0;
    }
    return textureLoad(ssao_tex, vec2i(frag_xy), 0).r;
}

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

fn v_smith(nv: f32, nl: f32, a: f32) -> f32 {
    let a2 = a * a;
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-5);
}

fn f_schlick(f0: vec3f, vh: f32) -> vec3f {
    return f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
}

// Radiance reflected towards `v` from a light arriving along `l` (URP scale: Lambert
// without 1/PI, so a white surface facing an intensity-1 light is white).
fn brdf(n: vec3f, v: vec3f, l: vec3f, albedo: vec3f, f0: vec3f, rough: f32, metallic: f32) -> vec3f {
    let nl = max(dot(n, l), 0.0);
    if (nl <= 0.0) {
        return vec3f(0.0);
    }
    let h = normalize(v + l);
    let nv = max(dot(n, v), 1e-4);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let a = max(rough * rough, 0.002);
    let f = f_schlick(f0, vh);
    let spec = d_ggx(nh, a) * v_smith(nv, nl, a) * f;
    let kd = (1.0 - f) * (1.0 - metallic);
    return (kd * albedo / PI + spec) * nl * PI;
}

fn shadow_tap(layer: i32, uv: vec2f, depth: f32) -> f32 {
    return textureSampleCompareLevel(shadow_map, shadow_sampler, uv, layer, depth);
}

fn sun_shadow(world_pos: vec3f, n: vec3f, view_depth: f32) -> f32 {
    let count = i32(frame.shadow_params.w);
    if (frame.sun_color.w <= 0.0 || count == 0) {
        return 1.0;
    }
    var cascade = -1;
    for (var i = 0; i < count; i++) {
        if (view_depth < frame.cascade_splits[i]) {
            cascade = i;
            break;
        }
    }
    if (cascade < 0) {
        return 1.0;
    }
    // Normal offset scaled by the cascade's texel size.
    let texel = frame.cascade_texel[cascade];
    let l = sun_to_light();
    let nl = clamp(dot(n, l), 0.0, 1.0);
    let offset = n * frame.shadow_params.y * texel * (1.0 - nl * 0.5);
    let p = frame.cascade_vp[cascade] * vec4f(world_pos + offset, 1.0);
    let ndc = p.xyz / p.w;
    let uv = vec2f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) || ndc.z > 1.0) {
        return 1.0;
    }
    let depth = ndc.z - frame.shadow_params.x * 0.001;
    var lit: f32;
    if (frame.shadow_params.z > 0.5) {
        // 3x3 PCF on hardware-filtered taps (~5x5 kernel).
        let ts = 1.0 / f32(textureDimensions(shadow_map).x);
        lit = 0.0;
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                lit += shadow_tap(cascade, uv + vec2f(f32(x), f32(y)) * ts * 1.5, depth);
            }
        }
        lit /= 9.0;
    } else {
        lit = shadow_tap(cascade, uv, depth);
    }
    // Fade out at the end of the last cascade.
    let last = frame.cascade_splits[count - 1];
    let fade = clamp((last - view_depth) / (last * 0.1), 0.0, 1.0);
    lit = mix(1.0, lit, fade);
    return mix(1.0, lit, frame.sun_color.w);
}

fn env_brdf_approx(f0: vec3f, rough: f32, nv: f32) -> vec3f {
    // Karis' analytic fit of the split-sum environment BRDF.
    let c0 = vec4f(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4f(1.0, 0.0425, 1.04, -0.04);
    let r = rough * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2f(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// Diffuse ambient from the sky's spherical harmonics (Unity "Environment Lighting: Skybox").
fn ambient_diffuse(n: vec3f) -> vec3f {
    let c = ambient_sh;
    let e = c[0].rgb * 0.282095
        + c[1].rgb * (0.488603 * n.y)
        + c[2].rgb * (0.488603 * n.z)
        + c[3].rgb * (0.488603 * n.x)
        + c[4].rgb * (1.092548 * n.x * n.y)
        + c[5].rgb * (1.092548 * n.y * n.z)
        + c[6].rgb * (0.315392 * (3.0 * n.z * n.z - 1.0))
        + c[7].rgb * (1.092548 * n.x * n.z)
        + c[8].rgb * (0.546274 * (n.x * n.x - n.y * n.y));
    return max(e, vec3f(0.0)) * frame.ground_color.w;
}

// Unity's perceptualRoughnessToMipmapLevel (6 LOD steps).
fn roughness_mip(rough: f32) -> f32 {
    return rough * (1.7 - 0.7 * rough) * 6.0;
}

// Prefiltered radiance from one probe slot, box-projected when the probe asks for it.
fn sample_probe(slot: u32, r: vec3f, pos: vec3f, mip: f32) -> vec3f {
    let p = probes[slot];
    var dir = r;
    if (p.position.w > 0.5) {
        let rd = select(r, vec3f(1e-5), abs(r) < vec3f(1e-5));
        let t1 = (p.box_max.xyz - pos) / rd;
        let t2 = (p.box_min.xyz - pos) / rd;
        let tf = max(t1, t2);
        let t = max(min(min(tf.x, tf.y), tf.z), 0.0);
        dir = pos + r * t - p.position.xyz;
    }
    return textureSampleLevel(probe_cubes, probe_sampler, dir, i32(slot), mip).rgb * p.box_min.w;
}

// Specular environment: probe A blended over probe B (or the sky) by A's weight.
// `sel` = (slot_a | slot_b << 16, bitcast weight_a), from the instance data.
fn env_specular(r: vec3f, pos: vec3f, rough: f32, sel: vec2u) -> vec3f {
    let mip = roughness_mip(rough);
    let a = sel.x & 0xffffu;
    let b = sel.x >> 16u;
    let w = bitcast<f32>(sel.y);
    let cb = sample_probe(b, r, pos, mip);
    if (w <= 0.0) {
        return cb;
    }
    return mix(cb, sample_probe(a, r, pos, mip), w);
}

// Full surface shading: sun (with shadows), punctual lights, sky ambient and reflections.
fn shade(world_pos: vec3f, n: vec3f, geo_n: vec3f, albedo: vec3f, metallic: f32, rough: f32, ao_in: f32, receive_shadows: bool, probe_sel: vec2u, frag_xy: vec2f, gi: u32, lm_uv: vec2f) -> vec3f {
    let f0 = mix(vec3f(0.04), albedo, metallic);
    let ssao = screen_ao(frag_xy);
    let ao = min(ao_in, ssao);
    // URP: AO also darkens direct light by "Direct Lighting Strength".
    let direct_ao = mix(1.0, ssao, frame.ssao.y);
    let v = normalize(frame.camera_pos.xyz - world_pos);
    let nv = max(dot(n, v), 1e-4);
    var color = vec3f(0.0);
    if (frame.sun_dir.w > 0.5) {
        let view_depth = (frame.view * vec4f(world_pos, 1.0)).z;
        var shadow = 1.0;
        if (receive_shadows) {
            shadow = sun_shadow(world_pos, geo_n, view_depth);
        }
        color += brdf(n, v, sun_to_light(), albedo, f0, rough, metallic) * frame.sun_color.rgb * shadow * direct_ao;
    }
    let view_z = (frame.view * vec4f(world_pos, 1.0)).z;
    let cluster = cluster_of(frag_xy, view_z);
    for (var j = 0u; j < cluster.y; j++) {
        let L = lights[cluster_lights[cluster.x + j]];
        let to_l = L.pos_range.xyz - world_pos;
        let d2 = dot(to_l, to_l);
        let r = L.pos_range.w;
        if (d2 > r * r) {
            continue;
        }
        let l = to_l * inverseSqrt(max(d2, 1e-8));
        // URP attenuation: inverse square with a smooth window at the range.
        let k = d2 / (r * r);
        let window = clamp(1.0 - k * k, 0.0, 1.0);
        var atten = window * window / max(d2, 0.01);
        if (L.color.w > 0.5) {
            let cd = dot(L.dir.xyz, -l);
            let s = clamp((cd - L.spot.x) * L.spot.y, 0.0, 1.0);
            atten *= s * s;
        }
        if (L.spot.z > 0.5 && receive_shadows && atten > 0.0) {
            atten *= punctual_shadow(L, world_pos, geo_n, sqrt(d2));
        }
        color += brdf(n, v, l, albedo, f0, rough, metallic) * L.color.rgb * atten * direct_ao;
    }
    // Environment: SH diffuse ambient + prefiltered reflection probes / sky.
    let diffuse_amb = diffuse_light(n, gi, lm_uv) * albedo * (1.0 - metallic);
    let r = reflect(-v, n);
    let spec_env = env_specular(r, world_pos, rough, probe_sel);
    let horizon_occ = clamp(1.0 + dot(r, geo_n), 0.0, 1.0);
    color += (diffuse_amb + spec_env * env_brdf_approx(f0, rough, nv) * frame.fog_params.w * horizon_occ * horizon_occ) * ao;
    return color;
}
