// The built-in Lit material's hooks for shadow.wgsl: no vertex changes, base map alpha.

fn vertex_mod(v: VertexInput) -> VertexMod {
    var m: VertexMod;
    m.position = v.position;
    m.normal = v.normal;
    m.tangent = v.tangent;
    return m;
}

fn surface(i: SurfaceInput) -> Surface {
    let uv = sample_uv(i.uv0 * material.tiling_offset.xy + material.tiling_offset.zw);
    var s: Surface;
    s.alpha = material.base_color.a * textureSample(base_map, mat_sampler, uv).a;
    s.alpha_clip = material.flags.x;
    return s;
}
