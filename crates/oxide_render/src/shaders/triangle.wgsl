// P0 test shader: one vertex-colored triangle transformed by a model-view-projection matrix.

struct Uniforms {
    mvp: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    // Equilateral triangle centered on the origin, in the XY plane (Unity: facing -Z camera).
    var positions = array<vec3<f32>, 3>(
        vec3<f32>(0.0, 0.577, 0.0),
        vec3<f32>(0.5, -0.289, 0.0),
        vec3<f32>(-0.5, -0.289, 0.0),
    );
    // Linear-space colors; the sRGB render target encodes them.
    var colors = array<vec3<f32>, 3>(
        vec3<f32>(1.0, 0.08, 0.05),
        vec3<f32>(0.05, 0.8, 0.1),
        vec3<f32>(0.05, 0.2, 1.0),
    );
    var out: VsOut;
    out.position = u.mvp * vec4<f32>(positions[index], 1.0);
    out.color = colors[index];
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
