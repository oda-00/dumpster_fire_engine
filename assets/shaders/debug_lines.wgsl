// Debug line renderer for the wgpu fallback backend.

struct Camera { mvp: mat4x4<f32> }
@group(0) @binding(0) var<uniform> cam: Camera;

struct VertIn {
    @location(0) pos:   vec3<f32>,
    @location(1) color: vec4<f32>,
}

struct VertOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color:      vec4<f32>,
}

@vertex
fn vs_main(in: VertIn) -> VertOut {
    var out: VertOut;
    out.clip  = cam.mvp * vec4<f32>(in.pos, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    return in.color;
}
