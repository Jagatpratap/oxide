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

// Lightmap layer of a draw (-1: not lightmapped).
fn lightmap_layer(gi: u32) -> i32 {
    if (gi == NO_GI) {
        return -1;
    }
    return i32(instance_gi[gi].info.x);
}

// The baked shadowmask of a lightmapped draw (1 = unshadowed; Shadowmask mode).
fn baked_shadowmask(gi: u32, lm_uv: vec2f) -> vec4f {
    let layer = lightmap_layer(gi);
    if (layer < 0 || frame.mixed.y < 0.0) {
        return vec4f(1.0);
    }
    return textureSampleLevel(lightmaps, probe_sampler, vec2f(lm_uv.x, 1.0 - lm_uv.y), i32(frame.mixed.y) + layer, 0.0);
}

// Subtractive: the main light's realtime shadow darkens baked light (Unity's
// SubtractMainLightWithRealtimeAttenuationFromLightmap), down to the shadow colour.
fn subtract_main_light(baked: vec3f, world_pos: vec3f, n: vec3f, geo_n: vec3f) -> vec3f {
    let view_depth = (frame.view * vec4f(world_pos, 1.0)).z;
    let atten = sun_shadow(world_pos, geo_n, view_depth);
    let lambert = frame.sun_color.rgb * clamp(dot(sun_to_light(), n), 0.0, 1.0);
    let subtracted = baked - lambert * (1.0 - atten);
    return min(baked, max(subtracted, frame.subtractive.rgb));
}

// Diffuse light from a lightmap (gamma-space RGBM, range 8; with its dominant direction in
// Directional mode) or light probe SH, else the sky.
fn diffuse_light(world_pos: vec3f, n: vec3f, geo_n: vec3f, gi: u32, lm_uv: vec2f) -> vec3f {
    if (gi != NO_GI) {
        let g = instance_gi[gi];
        if (g.info.x >= 0.0) {
            let uv = vec2f(lm_uv.x, 1.0 - lm_uv.y);
            let p = textureSampleLevel(lightmaps, probe_sampler, uv, i32(g.info.x), 0.0);
            var s = p.rgb * p.a * 2.828427;
            s = s * s;
            if (frame.mixed.z >= 0.0) {
                // Unity's DecodeDirectionalLightmap.
                let d = textureSampleLevel(lightmaps, probe_sampler, uv, i32(frame.mixed.z) + i32(g.info.x), 0.0);
                let half_lambert = dot(n, d.xyz - 0.5) + 0.5;
                s = s * half_lambert / max(1e-4, d.w);
            }
            if (u32(frame.mixed.x + 0.5) == 2u && frame.sun_mixed.x > 0.5 && frame.sun_dir.w > 0.5) {
                s = subtract_main_light(s, world_pos, n, geo_n);
            }
            return s;
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
// without 1/PI, so a white surface facing an intensity-1 light is white). `diffuse` is the
// Lambert colour, `f0` the specular reflectance at normal incidence.
fn brdf_df(n: vec3f, v: vec3f, l: vec3f, diffuse: vec3f, f0: vec3f, rough: f32) -> vec3f {
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
    return ((1.0 - f) * diffuse / PI + spec) * nl * PI;
}

// Metallic workflow.
fn brdf(n: vec3f, v: vec3f, l: vec3f, albedo: vec3f, f0: vec3f, rough: f32, metallic: f32) -> vec3f {
    return brdf_df(n, v, l, albedo * (1.0 - metallic), f0, rough);
}

// URP Simple Lit: Lambert plus a Blinn-Phong highlight in the specular colour.
fn blinn_phong(n: vec3f, v: vec3f, l: vec3f, diffuse: vec3f, spec: vec3f, smoothness: f32) -> vec3f {
    let nl = max(dot(n, l), 0.0);
    if (nl <= 0.0) {
        return vec3f(0.0);
    }
    let nh = max(dot(n, normalize(v + l)), 0.0);
    let shininess = exp2(10.0 * smoothness + 1.0);
    return (diffuse + spec * pow(nh, shininess)) * nl;
}

// Lighting models: 0 physically based (Lit), 1 Blinn-Phong (Simple Lit), 2 baked only.
fn light_brdf(model: u32, n: vec3f, v: vec3f, l: vec3f, diffuse: vec3f, f0: vec3f, rough: f32) -> vec3f {
    if (model == 1u) {
        return blinn_phong(n, v, l, diffuse, f0, 1.0 - rough);
    }
    return brdf_df(n, v, l, diffuse, f0, rough);
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

// Direct light: the sun (with cascaded shadows) and the clustered point/spot lights.
// Baked Lit (model 2) gets none.
@group(0) @binding(15) var light_cookies: texture_2d_array<f32>;
@group(0) @binding(16) var cookie_sampler: sampler;

// A cookie's tint: RGB × alpha of layer `layer` at `uv`.
fn cookie(layer: f32, uv: vec2f) -> vec3f {
    let c = textureSampleLevel(light_cookies, cookie_sampler, uv, i32(layer), 0.0);
    return c.rgb * c.a;
}

// Rendering layers for lighting (8, which the deferred G-buffer keeps).
const LIGHT_LAYERS: u32 = 0xffu;

// What a surface brings to direct lighting besides its material.
struct DirectExtra {
    // Rendering layers: lights reach it when their layers overlap.
    layers: u32,
    // Lightmapped (baked data may stand in for Mixed lights).
    lightmapped: bool,
    // Baked shadowmask (Shadowmask mode).
    shadowmask: vec4f,
    // Lights to add: 0 all, 1 only Mixed ones, 2 only the others.
    select: u32,
};

fn no_extra() -> DirectExtra {
    return DirectExtra(0xffffffffu, false, vec4f(1.0), 0u);
}

// Whether `ext.select` takes a light.
fn selected(mixed: bool, ext: DirectExtra) -> bool {
    return ext.select == 0u || (ext.select == 1u) == mixed;
}

// How a Mixed light reaches a surface: 0 when it is baked in (Subtractive lightmapped),
// else its baked static shadow (1 = none).
fn mixed_factor(mixed: bool, channel: f32, ext: DirectExtra) -> f32 {
    if (!mixed || !ext.lightmapped) {
        return 1.0;
    }
    let mode = u32(frame.mixed.x + 0.5);
    if (mode == 2u) {
        return 0.0;
    }
    if (mode == 1u && channel >= 0.0) {
        return ext.shadowmask[u32(channel)];
    }
    return 1.0;
}

fn direct_light(world_pos: vec3f, n: vec3f, geo_n: vec3f, diffuse: vec3f, f0: vec3f, rough: f32, model: u32, receive_shadows: bool, frag_xy: vec2f, ext: DirectExtra) -> vec3f {
    let layers = ext.layers;
    if (model == 2u) {
        return vec3f(0.0);
    }
    // URP: AO also darkens direct light by "Direct Lighting Strength".
    let direct_ao = mix(1.0, screen_ao(frag_xy), frame.ssao.y);
    let v = normalize(frame.camera_pos.xyz - world_pos);
    var color = vec3f(0.0);
    if (frame.sun_dir.w > 0.5 && (u32(frame.cluster_params.z) & layers & LIGHT_LAYERS) != 0u && selected(frame.sun_mixed.x > 0.5, ext)) {
        let view_depth = (frame.view * vec4f(world_pos, 1.0)).z;
        var shadow = 1.0;
        if (receive_shadows) {
            shadow = sun_shadow(world_pos, geo_n, view_depth);
        }
        shadow = min(shadow, mixed_factor(frame.sun_mixed.x > 0.5, frame.sun_mixed.y, ext));
        var tint = vec3f(1.0);
        if (frame.sun_right.w >= 0.0) {
            // Tiled across the world along the light's right and up vectors.
            let right = frame.sun_right.xyz;
            let up = cross(frame.sun_dir.xyz, right);
            let uv = (vec2f(dot(world_pos, right), -dot(world_pos, up)) + frame.sun_cookie.zw) / frame.sun_cookie.xy;
            tint = cookie(frame.sun_right.w, fract(uv));
        }
        color += light_brdf(model, n, v, sun_to_light(), diffuse, f0, rough) * frame.sun_color.rgb * tint * shadow * direct_ao;
    }
    let view_z = (frame.view * vec4f(world_pos, 1.0)).z;
    let cluster = cluster_of(frag_xy, view_z);
    for (var j = 0u; j < cluster.y; j++) {
        let L = lights[cluster_lights[cluster.x + j]];
        if ((u32(L.extra.x) & layers & LIGHT_LAYERS) == 0u || !selected(L.extra.w > 0.5, ext)) {
            continue;
        }
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
        var tint = vec3f(1.0);
        if (L.extra.y >= 0.0 && L.color.w > 0.5 && atten > 0.0) {
            // Projected through the cone: the cookie fills the outer angle.
            let fwd = L.dir.xyz;
            let right = L.right.xyz;
            let up = cross(fwd, right);
            let p = -to_l;
            let z = max(dot(p, fwd), 1e-4);
            let tan_half = sqrt(max(1.0 - L.spot.x * L.spot.x, 0.0)) / max(L.spot.x, 1e-4);
            let uv = vec2f(dot(p, right), -dot(p, up)) / (z * tan_half) * 0.5 + 0.5;
            tint = cookie(L.extra.y, clamp(uv, vec2f(0.0), vec2f(1.0)));
        }
        if (L.spot.z > 0.5 && receive_shadows && atten > 0.0) {
            atten *= punctual_shadow(L, world_pos, geo_n, sqrt(d2));
        }
        atten *= mixed_factor(L.extra.w > 0.5, L.extra.z, ext);
        color += light_brdf(model, n, v, l, diffuse, f0, rough) * L.color.rgb * tint * atten * direct_ao;
    }
    return color;
}

// Indirect light: baked or ambient diffuse (lightmaps, light probes, sky SH) and, for the
// physically based model, reflections (probes / sky), with material and screen-space AO.
fn indirect_light(world_pos: vec3f, n: vec3f, geo_n: vec3f, diffuse: vec3f, f0: vec3f, rough: f32, ao_in: f32, model: u32, probe_sel: vec2u, frag_xy: vec2f, gi: u32, lm_uv: vec2f) -> vec3f {
    let ao = min(ao_in, screen_ao(frag_xy));
    let diffuse_amb = diffuse_light(world_pos, n, geo_n, gi, lm_uv) * diffuse;
    if (model != 0u) {
        return diffuse_amb * ao;
    }
    let v = normalize(frame.camera_pos.xyz - world_pos);
    let nv = max(dot(n, v), 1e-4);
    let r = reflect(-v, n);
    let spec_env = env_specular(r, world_pos, rough, probe_sel);
    let horizon_occ = clamp(1.0 + dot(r, geo_n), 0.0, 1.0);
    return (diffuse_amb + spec_env * env_brdf_approx(f0, rough, nv) * frame.fog_params.w * horizon_occ * horizon_occ) * ao;
}

// Full surface shading (forward, metallic workflow): direct + indirect.
fn shade(world_pos: vec3f, n: vec3f, geo_n: vec3f, albedo: vec3f, metallic: f32, rough: f32, ao_in: f32, receive_shadows: bool, probe_sel: vec2u, frag_xy: vec2f, gi: u32, lm_uv: vec2f) -> vec3f {
    let diffuse = albedo * (1.0 - metallic);
    let f0 = mix(vec3f(0.04), albedo, metallic);
    return direct_light(world_pos, n, geo_n, diffuse, f0, rough, 0u, receive_shadows, frag_xy, no_extra())
        + indirect_light(world_pos, n, geo_n, diffuse, f0, rough, ao_in, 0u, probe_sel, frag_xy, gi, lm_uv);
}

// Scene view debug views (frame.misc.y): 1 albedo, 2 normals, 3 lighting only,
// 4 shadow cascades, 5 baked lightmap / probes.
fn debug_mode() -> u32 {
    return u32(frame.misc.y + 0.5);
}

fn debug_view(mode: u32, world_pos: vec3f, n: vec3f, geo_n: vec3f, albedo: vec3f, receive_shadows: bool, probe_sel: vec2u, frag_xy: vec2f, gi: u32, lm_uv: vec2f) -> vec3f {
    switch mode {
        case 1u: {
            return albedo;
        }
        case 2u: {
            return n * 0.5 + 0.5;
        }
        case 3u: {
            return shade(world_pos, n, geo_n, vec3f(0.7), 0.0, 0.6, 1.0, receive_shadows, probe_sel, frag_xy, gi, lm_uv);
        }
        case 4u: {
            let lit = shade(world_pos, n, geo_n, vec3f(0.7), 0.0, 0.6, 1.0, receive_shadows, probe_sel, frag_xy, gi, lm_uv);
            let view_depth = (frame.view * vec4f(world_pos, 1.0)).z;
            let colors = array<vec3f, 4>(vec3f(1.0, 0.25, 0.25), vec3f(0.25, 1.0, 0.25), vec3f(0.3, 0.45, 1.0), vec3f(1.0, 1.0, 0.25));
            let count = i32(frame.shadow_params.w);
            for (var i = 0; i < count; i++) {
                if (view_depth < frame.cascade_splits[i]) {
                    return lit * colors[i];
                }
            }
            return lit;
        }
        default: {
            return diffuse_light(world_pos, n, geo_n, gi, lm_uv);
        }
    }
}
