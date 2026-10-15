// Debug lines and gizmos (Debug.DrawLine, Gizmos, physics debug), depth-tested.

struct LineIn {
    @location(0) position: vec3f,
    @location(1) color: vec4f,
};

struct LineOut {
    @builtin(position) clip: vec4f,
    @location(0) color: vec4f,
};

@vertex
fn vs_main(v: LineIn) -> LineOut {
    var o: LineOut;
    o.clip = frame.view_proj * vec4f(v.position, 1.0);
    o.color = v.color;
    return o;
}

@fragment
fn fs_main(in: LineOut) -> @location(0) vec4f {
    return in.color;
}
