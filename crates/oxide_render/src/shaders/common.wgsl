// Shared declarations for Oxide's built-in shaders. Space is Unity's: left-handed, +Y up.

struct Frame {
    view_proj: mat4x4f,
    view: mat4x4f,
    inv_view_proj: mat4x4f,
    camera_pos: vec4f,      // xyz, w = time
    sun_dir: vec4f,         // xyz = direction the light travels, w = 1 if there is a sun
    sun_color: vec4f,       // linear rgb * intensity, w = shadow strength (0 = no shadows)
    shadow_params: vec4f,   // x depth bias, y normal bias, z soft, w cascade count
    cascade_splits: vec4f,  // view distance where each cascade ends
    cascade_vp: array<mat4x4f, 4>,
    sky_zenith: vec4f,      // rgb, w = sky exposure
    sky_horizon: vec4f,     // rgb, w = sun disk size
    ground_color: vec4f,    // rgb, w = ambient intensity
    fog_color: vec4f,       // rgb, w = mode (0 off, 1 linear, 2 exp, 3 exp2)
    fog_params: vec4f,      // density, start, end, reflection intensity
    misc: vec4f,            // x = punctual light count, y unused, zw = target size
    clear_color: vec4f,     // solid clear color (linear)
    cascade_texel: vec4f,   // world-space texel size of each cascade
    ssao: vec4f,            // x = 1 when the AO texture is valid, y = direct lighting strength
    cluster_viewport: vec4f, // camera viewport in pixels: x, y (from the top), width, height
    cluster_params: vec4f,  // depth slice = floor(ln(view z) * x + y), z = sun rendering layers
    sun_cookie: vec4f,      // xy cookie tile size (m), zw offset (m)
    sun_right: vec4f,       // xyz the sun's right vector, w cookie layer (-1 = none)
};

struct PunctualLight {
    pos_range: vec4f,
    color: vec4f,           // linear rgb * intensity, w = 0 point / 1 spot
    dir: vec4f,             // spot direction
    spot: vec4f,            // x = cos(outer/2), y = 1 / (cos(inner/2) - cos(outer/2)), z = first shadow tile + 1, w = shadow strength
    shadow: vec4f,          // x depth bias, y normal bias, z soft, w texel size per metre
    extra: vec4f,           // x rendering layers, y cookie layer (-1 = none)
    right: vec4f,           // xyz the light's right vector (cookie orientation)
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> lights: array<PunctualLight>;
@group(0) @binding(2) var shadow_map: texture_depth_2d_array;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;
@group(0) @binding(4) var<storage, read> joints: array<mat4x4f>;

struct ReflectionProbe {
    position: vec4f,        // capture position, w = 1 for box projection
    box_min: vec4f,         // w = intensity
    box_max: vec4f,
};

// Slot 0 is the sky; others are reflection probes (see reflection.rs).
@group(0) @binding(5) var<storage, read> probes: array<ReflectionProbe>;
@group(0) @binding(6) var probe_cubes: texture_cube_array<f32>;
@group(0) @binding(7) var probe_sampler: sampler;
// Diffuse ambient from the sky: L2 spherical harmonics (irradiance / PI).
@group(0) @binding(8) var<storage, read> ambient_sh: array<vec4f, 9>;

const PI: f32 = 3.14159265;

// Octahedral unit-vector encoding (deferred G-buffer normals).
fn oct_encode(n: vec3f) -> vec2f {
    let p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    if (n.z >= 0.0) {
        return p;
    }
    return (1.0 - abs(p.yx)) * select(vec2f(-1.0), vec2f(1.0), p >= vec2f(0.0));
}

fn oct_decode(e: vec2f) -> vec3f {
    var n = vec3f(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        n = vec3f((1.0 - abs(n.yx)) * select(vec2f(-1.0), vec2f(1.0), n.xy >= vec2f(0.0)), n.z);
    }
    return normalize(n);
}

fn sun_to_light() -> vec3f {
    return -frame.sun_dir.xyz;
}

// Procedural sky (Unity "Skybox/Procedural"-like): gradient + sunset tint + sun disk.
fn sky_color(dir_in: vec3f, with_sun: bool) -> vec3f {
    let dir = normalize(dir_in);
    let l = sun_to_light();
    let has_sun = frame.sun_dir.w > 0.5;
    let sun_h = select(0.6, l.y, has_sun);
    // Day factor dims the sky as the sun sets.
    let day = clamp(sun_h * 3.0 + 0.35, 0.05, 1.0);
    let sunset = pow(clamp(1.0 - abs(sun_h) * 3.0, 0.0, 1.0), 2.0);
    let zenith = frame.sky_zenith.rgb * day;
    var horizon = mix(frame.sky_horizon.rgb, vec3f(1.0, 0.55, 0.25), sunset * 0.7) * day;
    let ground = frame.ground_color.rgb * day * 0.6;
    let y = dir.y;
    var c: vec3f;
    if (y >= 0.0) {
        c = mix(horizon, zenith, pow(y, 0.45));
    } else {
        c = mix(horizon, ground, clamp(pow(-y * 4.0, 0.6), 0.0, 1.0));
    }
    if (has_sun) {
        let cosang = dot(dir, l);
        let sun_rgb = frame.sun_color.rgb;
        // Mie-like glow around the sun.
        let glow = pow(max(cosang, 0.0), 12.0) * 0.35 + pow(max(cosang, 0.0), 200.0) * 0.8;
        c += sun_rgb * glow * mix(vec3f(1.0), vec3f(1.0, 0.6, 0.3), sunset) * 0.5;
        if (with_sun) {
            let size = max(frame.sky_horizon.w, 0.001);
            let disk = smoothstep(1.0 - size * size * 0.06, 1.0 - size * size * 0.04, cosang);
            c += sun_rgb * disk * 40.0 * step(-0.02, y);
        }
    }
    return c * frame.sky_zenith.w;
}

fn apply_fog(color: vec3f, world_pos: vec3f) -> vec3f {
    let mode = frame.fog_color.w;
    if (mode < 0.5) {
        return color;
    }
    let d = distance(world_pos, frame.camera_pos.xyz);
    var f: f32;
    if (mode < 1.5) {
        f = clamp((d - frame.fog_params.y) / max(frame.fog_params.z - frame.fog_params.y, 0.001), 0.0, 1.0);
    } else if (mode < 2.5) {
        f = 1.0 - exp(-frame.fog_params.x * d);
    } else {
        let k = frame.fog_params.x * d;
        f = 1.0 - exp(-k * k);
    }
    return mix(color, frame.fog_color.rgb, f);
}
