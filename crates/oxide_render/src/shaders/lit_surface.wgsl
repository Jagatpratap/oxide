// The built-in Lit material's hooks for lit.wgsl (a Shader Graph generates its own).

// Keywords (Unity's _NORMALMAP, ...): pipeline constants set from the material, so each
// combination is its own shader variant and unused texture fetches compile away.
override kw__NORMALMAP: u32 = 1u;
override kw__METALLICSPECGLOSSMAP: u32 = 1u;
override kw__OCCLUSIONMAP: u32 = 1u;
override kw__EMISSION: u32 = 1u;
override kw__ALPHATEST_ON: u32 = 1u;

fn vertex_mod(v: VertexInput) -> VertexMod {
    var m: VertexMod;
    m.position = v.position;
    m.normal = v.normal;
    m.tangent = v.tangent;
    return m;
}

fn surface(i: SurfaceInput) -> Surface {
    let uv = sample_uv(i.uv0 * material.tiling_offset.xy + material.tiling_offset.zw);
    let base = material.base_color * textureSample(base_map, mat_sampler, uv) * i.vertex_color;
    var s: Surface;
    s.base_color = base.rgb;
    s.alpha = base.a;
    // Tangent-space normal from X/Y (RGB and two-channel BC5 maps), as Unity's
    // UnpackNormalScale: scale X/Y, then rebuild Z.
    if (kw__NORMALMAP != 0u) {
        let nxy = (textureSample(normal_map, mat_sampler, uv).xy * 2.0 - 1.0) * material.pbr.z;
        s.normal_ts = vec3f(nxy, sqrt(clamp(1.0 - dot(nxy, nxy), 0.0, 1.0)));
    } else {
        s.normal_ts = vec3f(0.0, 0.0, 1.0);
    }
    var mr = vec4f(1.0);
    if (kw__METALLICSPECGLOSSMAP != 0u) {
        mr = textureSample(metal_rough_map, mat_sampler, uv);
    }
    if (material.spec.w > 0.5 && material.spec.w < 2.5) {
        // Specular workflow / Simple Lit: the map's RGB is the specular colour, A smoothness.
        s.specular = material.spec.rgb * mr.rgb;
        s.smoothness = (1.0 - material.pbr.y) * mr.a;
    } else {
        s.metallic = material.pbr.x * mr.b;
        s.smoothness = 1.0 - material.pbr.y * mr.g;
    }
    s.occlusion = 1.0;
    if (kw__OCCLUSIONMAP != 0u) {
        s.occlusion = mix(1.0, textureSample(occlusion_map, mat_sampler, uv).r, material.pbr.w);
    }
    s.emission = vec3f(0.0);
    if (kw__EMISSION != 0u) {
        s.emission = material.emission.rgb * textureSample(emission_map, mat_sampler, uv).rgb;
    }
    s.alpha_clip = select(-1.0, material.flags.x, kw__ALPHATEST_ON != 0u && material.flags.z > 0.5);
    return s;
}
