// TextMeshPro "Distance Field" shading, shared by 3D text and Canvas text.
// The atlas alpha holds 0.5 + distance / (2 × spread): 0.5 on the outline.

struct SdfParams {
    face: vec4f,           // linear face colour × vertex colour (premultiplied later)
    outline: vec4f,        // linear outline colour
    underlay: vec4f,       // linear underlay colour
    shape: vec4f,          // x dilate (-1..1), y softness (0..1), z outline width (0..1), w spread (atlas px)
    under: vec4f,          // xy underlay offset (-1..1), z underlay dilate, w underlay softness
    atlas: vec4f,          // xy atlas size, z underlay on, w 0 = all, 1 = underlay only, 2 = face only
};

// Coverage of a signed distance (atlas px) with `blur` atlas px of softness, anti-aliased
// over one screen pixel.
fn sdf_cover(d: f32, px: f32, blur: f32) -> f32 {
    let w = max(px * 0.5 + blur * 0.5, 1e-4);
    return clamp((d + w) / (2.0 * w), 0.0, 1.0);
}

// Premultiplied colour of a glyph sample at `uv`, given `alpha` = atlas alpha at uv and
// `under_alpha` = atlas alpha at the underlay's offset uv.
fn sdf_shade(p: SdfParams, alpha: f32, under_alpha: f32, px: f32) -> vec4f {
    let spread = p.shape.w;
    let d = (alpha - 0.5) * 2.0 * spread + p.shape.x * spread * 0.5;
    let half_outline = p.shape.z * spread * 0.5;
    let blur = p.shape.y * spread;
    let outer = sdf_cover(d + half_outline, px, blur);
    let inner = sdf_cover(d - half_outline, px, blur);
    // Face inside, outline in the band around the edge (TMP centres the outline).
    var face = p.face;
    if p.shape.z <= 0.0 {
        face = vec4f(p.face.rgb, p.face.a * outer);
    } else {
        let col = mix(p.outline, p.face, inner);
        face = vec4f(col.rgb, col.a * outer);
    }
    var c = vec4f(face.rgb * face.a, face.a);
    if p.atlas.w > 0.5 && p.atlas.w < 1.5 {
        c = vec4f(0.0);
    }
    if p.atlas.z > 0.5 && p.atlas.w < 1.5 {
        let du = (under_alpha - 0.5) * 2.0 * spread + p.under.z * spread * 0.5;
        let ua = sdf_cover(du, px, p.under.w * spread) * p.underlay.a;
        let u = vec4f(p.underlay.rgb * ua, ua);
        c = c + u * (1.0 - c.a);
    }
    return c;
}
