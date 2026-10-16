// ---- lighting -------------------------------------------------------------------------

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
fn shade(world_pos: vec3f, n: vec3f, geo_n: vec3f, albedo: vec3f, metallic: f32, rough: f32, ao: f32, receive_shadows: bool, probe_sel: vec2u) -> vec3f {
    let f0 = mix(vec3f(0.04), albedo, metallic);
    let v = normalize(frame.camera_pos.xyz - world_pos);
    let nv = max(dot(n, v), 1e-4);
    var color = vec3f(0.0);
    if (frame.sun_dir.w > 0.5) {
        let view_depth = (frame.view * vec4f(world_pos, 1.0)).z;
        var shadow = 1.0;
        if (receive_shadows) {
            shadow = sun_shadow(world_pos, geo_n, view_depth);
        }
        color += brdf(n, v, sun_to_light(), albedo, f0, rough, metallic) * frame.sun_color.rgb * shadow;
    }
    let count = u32(frame.misc.x);
    for (var i = 0u; i < count; i++) {
        let L = lights[i];
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
        color += brdf(n, v, l, albedo, f0, rough, metallic) * L.color.rgb * atten;
    }
    // Environment: SH diffuse ambient + prefiltered reflection probes / sky.
    let diffuse_amb = ambient_diffuse(n) * albedo * (1.0 - metallic);
    let r = reflect(-v, n);
    let spec_env = env_specular(r, world_pos, rough, probe_sel);
    let horizon_occ = clamp(1.0 + dot(r, geo_n), 0.0, 1.0);
    color += (diffuse_amb + spec_env * env_brdf_approx(f0, rough, nv) * frame.fog_params.w * horizon_occ * horizon_occ) * ao;
    return color;
}
