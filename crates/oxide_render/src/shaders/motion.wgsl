// Per-object motion vectors for TAA (URP's motion vector pass): objects that moved since
// the last frame write their screen-space velocity where they are the visible surface.
// Pixels nobody writes (a = 0) reproject from depth and the camera motion instead.

struct Motion {
    jittered_vp: mat4x4f,   // the projection used for the scene (matches its depth)
    curr_vp: mat4x4f,       // unjittered, this frame
    prev_vp: mat4x4f,       // unjittered, last frame
};

@group(0) @binding(0) var<uniform> motion: Motion;
@group(0) @binding(1) var<storage, read> joints: array<mat4x4f>;
@group(0) @binding(2) var<storage, read> prev_joints: array<mat4x4f>;
@group(0) @binding(3) var scene_depth: texture_depth_2d;

struct VertexIn {
    @location(0) position: vec3f,
    @location(1) m0: vec4f,
    @location(2) m1: vec4f,
    @location(3) m2: vec4f,
    @location(4) m3: vec4f,
    @location(5) p0: vec4f,
    @location(6) p1: vec4f,
    @location(7) p2: vec4f,
    @location(8) p3: vec4f,
    // x = joint offset, y = previous joint offset
    @location(9) extra: vec4u,
};

struct SkinIn {
    @location(10) joint_ids: vec4u,
    @location(11) weights: vec4f,
};

struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) curr: vec4f,
    @location(1) prev: vec4f,
};

fn finish(world: vec4f, prev_world: vec4f) -> VertexOut {
    var o: VertexOut;
    o.clip = motion.jittered_vp * world;
    o.curr = motion.curr_vp * world;
    o.prev = motion.prev_vp * prev_world;
    return o;
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let p = vec4f(v.position, 1.0);
    let model = mat4x4f(v.m0, v.m1, v.m2, v.m3);
    let prev = mat4x4f(v.p0, v.p1, v.p2, v.p3);
    return finish(model * p, prev * p);
}

fn skin(base: u32, s: SkinIn, prev: bool) -> mat4x4f {
    if (prev) {
        return prev_joints[base + s.joint_ids.x] * s.weights.x
             + prev_joints[base + s.joint_ids.y] * s.weights.y
             + prev_joints[base + s.joint_ids.z] * s.weights.z
             + prev_joints[base + s.joint_ids.w] * s.weights.w;
    }
    return joints[base + s.joint_ids.x] * s.weights.x
         + joints[base + s.joint_ids.y] * s.weights.y
         + joints[base + s.joint_ids.z] * s.weights.z
         + joints[base + s.joint_ids.w] * s.weights.w;
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn) -> VertexOut {
    let p = vec4f(v.position, 1.0);
    return finish(skin(v.extra.x, s, false) * p, skin(v.extra.y, s, true) * p);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    // Only where this surface is what the camera sees (reversed-Z: nearer = larger).
    let d = textureLoad(scene_depth, vec2i(in.clip.xy), 0);
    if (in.clip.z < d - max(d * 0.002, 1e-7)) {
        discard;
    }
    let c = in.curr.xy / in.curr.w;
    let p = in.prev.xy / in.prev.w;
    // NDC delta → UV delta (v points down).
    return vec4f((c - p) * vec2f(0.5, -0.5), 0.0, 1.0);
}
