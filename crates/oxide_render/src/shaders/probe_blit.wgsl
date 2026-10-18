// Copies a cubemap asset into the reflection-probe capture cube (one face per draw): a
// Custom probe's cubemap, or a Baked probe's saved capture (RGBM, range 8, like lightmaps).

struct Blit {
    face: u32,
    rgbm: u32,
    _pad: vec2u,
};

@group(0) @binding(0) var<uniform> blit: Blit;
@group(0) @binding(1) var source: texture_cube<f32>;
@group(0) @binding(2) var source_sampler: sampler;

struct VsOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.clip = vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2f(uv.x, 1.0 - uv.y);
    return o;
}

// World direction through texel `uv` (v down) of cube face `face` (D3D/Vulkan/Metal order).
fn face_dir(face: u32, uv: vec2f) -> vec3f {
    let a = uv * 2.0 - 1.0;
    // One return at the end: D3D's FXC rejects returns inside a switch.
    var d = vec3f(-a.x, -a.y, -1.0);
    switch face {
        case 0u: { d = vec3f(1.0, -a.y, -a.x); }
        case 1u: { d = vec3f(-1.0, -a.y, a.x); }
        case 2u: { d = vec3f(a.x, 1.0, a.y); }
        case 3u: { d = vec3f(a.x, -1.0, -a.y); }
        case 4u: { d = vec3f(a.x, -a.y, 1.0); }
        default: {}
    }
    return normalize(d);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let c = textureSampleLevel(source, source_sampler, face_dir(blit.face, in.uv), 0.0);
    if (blit.rgbm != 0u) {
        let s = c.rgb * c.a * 8.0;
        return vec4f(s * s, 1.0);
    }
    return vec4f(c.rgb, 1.0);
}
