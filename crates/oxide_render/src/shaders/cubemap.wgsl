// Image-based lighting: cube capture mip chain, GGX prefiltering into reflection-probe
// slots, and L2 spherical harmonics of the sky for diffuse ambient.
// Cube faces follow the D3D/Vulkan/Metal convention (+X, -X, +Y, -Y, +Z, -Z), which matches
// Unity's left-handed world space, so world directions sample the cube directly.

const PI: f32 = 3.14159265;

// World direction through texel `uv` (0..1, v down) of cube face `face`.
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
