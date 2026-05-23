// UI overlay shader — mirrors ui.vert.glsl / ui.frag.glsl logic.
// Solid fills use degenerate UV (0,0) → vertex-color bypass.
// Glyph quads use real atlas UV → R8 texture sample for alpha.

@group(0) @binding(0) var t_atlas: texture_2d<f32>;
@group(0) @binding(1) var s_atlas: sampler;

struct Screen { size: vec2<f32> }
@group(1) @binding(0) var<uniform> screen: Screen;

struct VertIn {
    @location(0) pos:   vec2<f32>,
    @location(1) uv:    vec2<f32>,
    @location(2) color: vec4<f32>,   // already linear f32 (unpacked from u8 by CPU)
}

struct VertOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv:         vec2<f32>,
    @location(1) color:      vec4<f32>,
}

@vertex
fn vs_main(in: VertIn) -> VertOut {
    // Pixel-space [0,screen] → NDC [-1,1], Y up.
    let ndc = (in.pos / screen.size) * 2.0 - vec2<f32>(1.0, 1.0);
    var out: VertOut;
    out.clip  = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    out.uv    = in.uv;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    // Degenerate-UV bypass: solid-fill quads have all vertices at (0,0).
    if in.uv.x <= 0.001 && in.uv.y <= 0.001 {
        return in.color;
    }
    let a = textureSample(t_atlas, s_atlas, in.uv).r;
    return vec4<f32>(in.color.rgb, in.color.a * a);
}
