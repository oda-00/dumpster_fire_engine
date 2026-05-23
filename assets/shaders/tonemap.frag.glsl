#version 460

layout(set = 0, binding = 0) uniform sampler2D u_hdr;
layout(push_constant) uniform Push { float exposure_scale; uint op; } pc;
layout(location = 0) in  vec2 v_uv;
layout(location = 0) out vec4 frag;

vec3 aces(vec3 x) {
    vec3 a = (x * (x + 0.0245786)) - 0.000090537;
    vec3 b = (x * (0.983729 * x + 0.4329510)) + 0.238081;
    return clamp(a / b, 0.0, 1.0);
}
vec3 reinhard(vec3 x) { return x / (x + 1.0); }

void main() {
    vec3 hdr_c = texture(u_hdr, v_uv).rgb * pc.exposure_scale;
    vec3 ldr;
    if      (pc.op == 1u) ldr = reinhard(hdr_c);
    else if (pc.op == 2u) ldr = aces(hdr_c);
    else                  ldr = clamp(hdr_c, 0.0, 1.0);
    // Linear -> sRGB
    ldr = mix(12.92 * ldr,
              1.055 * pow(max(ldr, vec3(0.0)), vec3(1.0/2.4)) - 0.055,
              step(vec3(0.0031308), ldr));
    frag = vec4(ldr, 1.0);
}
