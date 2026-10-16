// L2 spherical harmonics of the sky cube for diffuse ambient (needs cubemap.wgsl).


@group(0) @binding(0) var sh_faces: texture_2d_array<f32>;
@group(0) @binding(1) var<storage, read_write> sh_out: array<vec4f, 9>;

var<workgroup> partial: array<array<vec3f, 9>, 64>;
var<workgroup> partial_w: array<f32, 64>;

fn sh_basis(d: vec3f) -> array<f32, 9> {
    return array<f32, 9>(
        0.282095,
        0.488603 * d.y,
        0.488603 * d.z,
        0.488603 * d.x,
        1.092548 * d.x * d.y,
        1.092548 * d.y * d.z,
        0.315392 * (3.0 * d.z * d.z - 1.0),
        1.092548 * d.x * d.z,
        0.546274 * (d.x * d.x - d.y * d.y),
    );
}

// Projects the cube (layers 0..5 of `sh_faces`) onto L2 SH and stores irradiance / PI
// coefficients (cosine lobe folded in), so `sum(c_i * Y_i(n))` is the diffuse ambient.
@compute @workgroup_size(64)
fn cs_sh(@builtin(local_invocation_index) t: u32) {
    let size = textureDimensions(sh_faces).x;
    let total = size * size * 6u;
    var acc: array<vec3f, 9>;
    for (var k = 0u; k < 9u; k++) {
        acc[k] = vec3f(0.0);
    }
    var wsum = 0.0;
    for (var idx = t; idx < total; idx += 64u) {
        let face = idx / (size * size);
        let rem = idx % (size * size);
        let px = vec2u(rem % size, rem / size);
        let uv = (vec2f(px) + 0.5) / f32(size);
        let a = uv * 2.0 - 1.0;
        // Solid angle of the texel.
        let w = 4.0 / (pow(1.0 + dot(a, a), 1.5) * f32(size * size));
        let c = min(textureLoad(sh_faces, px, face, 0).rgb, vec3f(200.0));
        let y = sh_basis(face_dir(face, uv));
        for (var k = 0u; k < 9u; k++) {
            acc[k] += c * (y[k] * w);
        }
        wsum += w;
    }
    partial[t] = acc;
    partial_w[t] = wsum;
    workgroupBarrier();
    if (t == 0u) {
        var total_acc: array<vec3f, 9>;
        var total_w = 0.0;
        for (var k = 0u; k < 9u; k++) {
            total_acc[k] = vec3f(0.0);
        }
        for (var i = 0u; i < 64u; i++) {
            for (var k = 0u; k < 9u; k++) {
                total_acc[k] += partial[i][k];
            }
            total_w += partial_w[i];
        }
        let norm = 4.0 * PI / max(total_w, 1e-6);
        // Cosine-lobe convolution per band (A_l / PI): 1, 2/3, 1/4.
        let band = array<f32, 9>(1.0, 0.6666667, 0.6666667, 0.6666667, 0.25, 0.25, 0.25, 0.25, 0.25);
        for (var k = 0u; k < 9u; k++) {
            sh_out[k] = vec4f(total_acc[k] * norm * band[k], 0.0);
        }
    }
}
