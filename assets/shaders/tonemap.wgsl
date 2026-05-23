// Fullscreen tonemap pass. Samples HDR texture and applies the selected
// operator: 0 = linear, 1 = Reinhard, 2 = ACES filmic.

@group(0) @binding(0) var hdr:     texture_2d<f32>;
@group(0) @binding(1) var s_hdr:   sampler;

struct TonemapUniforms { op: u32 }
@group(1) @binding(0) var<uniform> params: TonemapUniforms;

struct VertOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle — no vertex buffer needed.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertOut {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VertOut;
    out.uv   = uv;
    out.clip = vec4<f32>(uv * 2.0 - vec2<f32>(1.0), 0.0, 1.0);
    return out;
}

fn aces(c: vec3<f32>) -> vec3<f32> {
    let a = 2.51; let b = 0.03; let cc = 2.43; let d = 0.59; let e = 0.14;
    return clamp((c * (a * c + b)) / (c * (cc * c + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn reinhard(c: vec3<f32>) -> vec3<f32> {
    return c / (c + vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    let hdr_color = textureSample(hdr, s_hdr, in.uv).rgb;
    var ldr: vec3<f32>;
    switch params.op {
        case 1u: { ldr = reinhard(hdr_color); }
        case 2u: { ldr = aces(hdr_color); }
        default: { ldr = clamp(hdr_color, vec3<f32>(0.0), vec3<f32>(1.0)); }
    }
    return vec4<f32>(pow(ldr, vec3<f32>(1.0 / 2.2)), 1.0);
}
