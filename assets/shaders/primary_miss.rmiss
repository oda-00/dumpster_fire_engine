#version 460
#extension GL_EXT_ray_tracing : require
#extension GL_EXT_nonuniform_qualifier : require

layout(set = 0, binding = 6) uniform samplerCube u_env[];
struct LightGpu { vec4 ci; uint kind; uint flags; uint p0; uint p1; vec4 d[6]; };
layout(set = 0, binding = 2) uniform Lights {
    uint count; uint env_idx; uint sky_present; uint _p;
    LightGpu lights[32];
} u_lights;

layout(location = 0) rayPayloadInEXT vec4 payload;

void main() {
    vec3 dir = normalize(gl_WorldRayDirectionEXT);
    if (u_lights.env_idx != 0xFFFFFFFFu) {
        for (uint i = 0u; i < u_lights.count; i++) {
            LightGpu lt = u_lights.lights[i];
            if (lt.kind == 17u) {
                float rot = lt.d[0].y;
                float s = sin(rot), c = cos(rot);
                vec3  d = vec3(c*dir.x - s*dir.z, dir.y, s*dir.x + c*dir.z);
                vec3  col = texture(u_env[nonuniformEXT(u_lights.env_idx)], d).rgb;
                payload = vec4(col * lt.ci.a * lt.d[0].z, 0.0);
                return;
            }
        }
    }
    if (u_lights.sky_present == 1u) {
        for (uint i = 0u; i < u_lights.count; i++) {
            LightGpu lt = u_lights.lights[i];
            if (lt.kind == 18u) {
                // Preetham-style sky gradient (stub; full HW eval in sky_hw.rs)
                float t = 0.5 * (dir.y + 1.0);
                payload = vec4(mix(vec3(0.5, 0.3, 0.1), vec3(0.3, 0.6, 1.0), t), 0.0);
                return;
            }
        }
    }
    float h = dir.y;
    vec3 ground  = vec3(0.12, 0.12, 0.14);
    vec3 horizon = vec3(0.85, 0.88, 0.95);
    vec3 zenith  = vec3(0.35, 0.55, 0.95);
    vec3 col = h < 0.0
        ? mix(horizon, ground, clamp(-h * 3.0, 0.0, 1.0))
        : mix(horizon, zenith, clamp(h * 1.5, 0.0, 1.0));
    payload = vec4(col, 0.0);
}
