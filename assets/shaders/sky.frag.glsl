#version 460
//
// Procedural sky gradient. Matches the default gradient in
// primary_miss.rmiss so raster and ray-traced modes agree.
layout(push_constant) uniform Push { mat4 inv_vp; } pc;
layout(location = 0) in vec2 v_ndc;
layout(location = 0) out vec4 o_color;
void main() {
    vec4 n = pc.inv_vp * vec4(v_ndc, 0.0, 1.0);
    vec4 f = pc.inv_vp * vec4(v_ndc, 1.0, 1.0);
    vec3 dir = normalize(f.xyz / f.w - n.xyz / n.w);
    float h = dir.y;
    vec3 ground  = vec3(0.12, 0.12, 0.14);
    vec3 horizon = vec3(0.85, 0.88, 0.95);
    vec3 zenith  = vec3(0.35, 0.55, 0.95);
    vec3 col = h < 0.0
        ? mix(horizon, ground, clamp(-h * 3.0, 0.0, 1.0))
        : mix(horizon, zenith, clamp(h * 1.5, 0.0, 1.0));
    o_color = vec4(col, 1.0);
}
