// Simplified forward-lit shader for the wgpu fallback backend.
// No skinning, no ray-tracing. One directional light hardcoded.

struct Camera { mvp: mat4x4<f32> }
@group(0) @binding(0) var<uniform> cam: Camera;

struct VertIn {
    @location(0) pos:    vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv:     vec2<f32>,
}

struct VertOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal:     vec3<f32>,
    @location(1) uv:         vec2<f32>,
}

@vertex
fn vs_main(in: VertIn) -> VertOut {
    var out: VertOut;
    out.clip   = cam.mvp * vec4<f32>(in.pos, 1.0);
    out.normal = in.normal;
    out.uv     = in.uv;
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    let n   = normalize(in.normal);
    let sun = normalize(vec3<f32>(0.4, 1.0, 0.6));
    let ndl = max(dot(n, sun), 0.0);
    let col = vec3<f32>(0.8, 0.8, 0.85) * (ndl * 0.7 + 0.3);
    return vec4<f32>(col, 1.0);
}
