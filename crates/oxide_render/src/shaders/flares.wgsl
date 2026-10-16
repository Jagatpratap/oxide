// Lens flares (Unity Lens Flare (SRP)): a copy of the HDR image, then element quads on top.
// Each quad's vertex shader samples the scene depth around its source to fade it when the
// source is hidden.

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var depth: texture_2d<f32>;   // reversed-Z: 1 = near, 0 = far

@group(1) @binding(0) var ftex: texture_2d<f32>;
@group(1) @binding(1) var fsamp: sampler;

struct BlitOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_blit(@builtin(vertex_index) i: u32) -> BlitOut {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var o: BlitOut;
    o.pos = vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2f(uv.x, 1.0 - uv.y);
    return o;
}

@fragment
fn fs_blit(in: BlitOut) -> @location(0) vec4f {
    return textureSampleLevel(scene, samp, in.uv, 0.0);
}

struct FIn {
    @location(0) center_size: vec4f,
    @location(1) color: vec4f,
    @location(2) shape: vec4f,      // gradient, edge offset, roundness, sides
    @location(3) misc: vec4f,       // kind, inverse, rotation, aspect
    @location(4) occlusion: vec4f,  // source ndc xy, source depth, radius
    @location(5) occlusion2: vec4f, // samples, blend
};

struct FOut {
    @builtin(position) pos: vec4f,
    @location(0) local: vec2f,
    @location(1) uv: vec2f,
    @location(2) color: vec4f,
    @location(3) @interpolate(flat) shape: vec4f,
    @location(4) @interpolate(flat) misc: vec4f,
};

// Fraction of samples around the source that aren't covered by nearer geometry.
fn visibility(o: vec4f, samples: f32, aspect: f32) -> f32 {
    let n = u32(samples);
    let dims = textureDimensions(depth);
    if n == 0u || dims.x <= 1u {
        return 1.0;
    }
    var visible = 0.0;
    for (var i = 0u; i < n; i++) {
        // Vogel disk: evenly spread points inside the radius.
        let r = o.w * sqrt((f32(i) + 0.5) / f32(n));
        let a = f32(i) * 2.39996323;
        let ndc = o.xy + vec2f(cos(a) * r / aspect, sin(a) * r);
        if abs(ndc.x) > 1.0 || abs(ndc.y) > 1.0 {
            visible += 1.0; // off screen: nothing known to hide it
            continue;
        }
        let px = vec2i(vec2f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * vec2f(dims));
        let d = textureLoad(depth, clamp(px, vec2i(0), vec2i(dims) - 1), 0).r;
        if d <= o.z * 1.0001 + 1e-7 {
            visible += 1.0;
        }
    }
    return visible / f32(n);
}

@vertex
fn vs_flare(@builtin(vertex_index) vi: u32, f: FIn) -> FOut {
    var corners = array<vec2f, 6>(vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0));
    let c = corners[vi];
    let aspect = f.misc.w;
    let vis = visibility(f.occlusion, f.occlusion2.x, aspect);
    // Rotate in screen-height units, then squash x back to NDC.
    let s = c * f.center_size.zw;
    let cr = cos(f.misc.z);
    let sr = sin(f.misc.z);
    let r = vec2f(s.x * cr - s.y * sr, s.x * sr + s.y * cr);
    var o: FOut;
    o.pos = vec4f(f.center_size.x + r.x / aspect, f.center_size.y + r.y, 0.0, 1.0);
    if vis <= 0.0 {
        o.pos = vec4f(2.0, 2.0, 2.0, 1.0); // fully hidden: degenerate, off screen
    }
    o.local = c;
    o.uv = vec2f(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    o.color = vec4f(f.color.rgb * vis, f.color.a);
    o.shape = f.shape;
    o.misc = vec4f(f.misc.x, f.misc.y, f.occlusion2.y, 0.0);
    return o;
}

// Fall Off 1 fades linearly to the edge; lower values give a softer glow.
fn falloff(m: f32, gradient: f32) -> f32 {
    return pow(clamp(m, 0.0, 1.0), 1.0 / max(gradient, 0.01));
}

@fragment
fn fs_flare(in: FOut) -> @location(0) vec4f {
    let kind = u32(in.misc.x + 0.5);
    let p = in.local;
    let r = length(p);
    var rgb = in.color.rgb;
    var m = 1.0;
    let edge = clamp(in.shape.y, 0.0, 0.999);
    if kind == 0u {
        let t = textureSample(ftex, fsamp, in.uv);
        rgb *= t.rgb;
        m = t.a;
    } else if kind == 1u {
        m = falloff((1.0 - r) / (1.0 - edge), in.shape.x);
    } else if kind == 2u {
        let n = max(round(in.shape.w), 3.0);
        let sector = 6.28318531 / n;
        let a = atan2(p.y, p.x) + 1.57079633; // a vertex points up
        let local_a = a - sector * floor(a / sector) - sector * 0.5;
        var rp = r * cos(local_a) / cos(3.14159265 / n);
        rp = mix(rp, r, clamp(in.shape.z, 0.0, 1.0));
        m = falloff((1.0 - rp) / (1.0 - edge), in.shape.x);
    } else {
        let half = max(edge, 0.02) * 0.5;
        m = falloff(1.0 - abs(r - (1.0 - half)) / half, in.shape.x);
    }
    if in.misc.y > 0.5 && kind != 0u {
        m = 1.0 - m;
    }
    let a = m * in.color.a;
    if a <= 0.0005 {
        discard;
    }
    let blend = u32(in.misc.z + 0.5);
    if blend == 3u {
        return vec4f(rgb, a);           // Lerp
    }
    if blend == 1u {
        return vec4f(min(rgb * a, vec3f(1.0)), a); // Screen works in 0..1
    }
    return vec4f(rgb * a, a);           // Additive / Premultiply
}
