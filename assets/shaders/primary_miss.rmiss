#version 460
#extension GL_EXT_ray_tracing : require
#extension GL_EXT_nonuniform_qualifier : require

layout(set = 0, binding = 6) uniform samplerCube u_env[];
struct LightGpu { vec4 ci; uint kind; uint flags; uint p0; uint p1; vec4 d[6]; };
layout(set = 0, binding = 2) uniform Lights {
    uint count; uint env_idx; uint sky_present; uint flags;
    LightGpu lights[32];
} u_lights;

layout(location = 0) rayPayloadInEXT vec4 payload;

void main() {
    vec3 dir = normalize(gl_WorldRayDirectionEXT);
    vec3 col;
    bool resolved = false;

    if (u_lights.env_idx != 0xFFFFFFFFu) {
        for (uint i = 0u; i < u_lights.count; i++) {
            LightGpu lt = u_lights.lights[i];
            if (lt.kind == 17u) {
                float rot = lt.d[0].y;
                float s = sin(rot), c = cos(rot);
                vec3  d = vec3(c*dir.x - s*dir.z, dir.y, s*dir.x + c*dir.z);
                col = texture(u_env[nonuniformEXT(u_lights.env_idx)], d).rgb
                    * lt.ci.a * lt.d[0].z;
                resolved = true;
                break;
            }
        }
    }
    if (!resolved && u_lights.sky_present == 1u) {
        for (uint i = 0u; i < u_lights.count; i++) {
            LightGpu lt = u_lights.lights[i];
            if (lt.kind == 18u) {
                // Preetham-style sky gradient (stub; full HW eval in sky_hw.rs)
                float t = 0.5 * (dir.y + 1.0);
                col = mix(vec3(0.5, 0.3, 0.1), vec3(0.3, 0.6, 1.0), t);
                resolved = true;
                break;
            }
        }
    }
    if (!resolved) {
        // Default gradient — matches assets/shaders/sky.frag.glsl so raster
        // and ray-traced modes agree.
        float h = dir.y;
        vec3 ground  = vec3(0.12, 0.12, 0.14);
        vec3 horizon = vec3(0.85, 0.88, 0.95);
        vec3 zenith  = vec3(0.35, 0.55, 0.95);
        col = h < 0.0
            ? mix(horizon, ground, clamp(-h * 3.0, 0.0, 1.0))
            : mix(horizon, zenith, clamp(h * 1.5, 0.0, 1.0));
    }

    // Editor floor grid (flags bit 0) — analytic ray/plane composite so the
    // RT path shows the same world grid as the raster DebugLines pass.
    // Misses only, so scene geometry occludes the grid for free.
    if ((u_lights.flags & 1u) != 0u && dir.y < -1e-4) {
        vec3  org = gl_WorldRayOriginEXT;
        float t   = -org.y / dir.y;
        vec3  hit = org + t * dir;
        if (t > 0.0 && abs(hit.x) <= 24.0 && abs(hit.z) <= 24.0) {
            float fade = clamp((32.0 - t) / (32.0 - 14.0), 0.0, 1.0);
            float w  = 0.006 * t + 0.004; // widen with distance to limit shimmer
            vec2  g1 = abs(fract(hit.xz + 0.5) - 0.5);
            float minor = 1.0 - smoothstep(0.0, w, min(g1.x, g1.y));
            vec2  g10 = abs(fract(hit.xz / 10.0 + 0.5) - 0.5) * 10.0;
            float major = 1.0 - smoothstep(0.0, w * 1.5, min(g10.x, g10.y));
            float ax = 1.0 - smoothstep(0.0, w * 1.5, abs(hit.z)); // X axis (z = 0)
            float az = 1.0 - smoothstep(0.0, w * 1.5, abs(hit.x)); // Z axis (x = 0)
            vec3  line_col = vec3(0.55, 0.55, 0.58);
            float a = minor * 0.5;
            line_col = mix(line_col, vec3(0.80, 0.80, 0.85), major);
            a = max(a, major * 0.7);
            if (ax > az && ax > 0.0) {
                line_col = vec3(0.95, 0.30, 0.30);
                a = max(a, ax * 0.95);
            } else if (az > 0.0) {
                line_col = vec3(0.30, 0.55, 0.95);
                a = max(a, az * 0.95);
            }
            col = mix(col, line_col, a * fade);
        }
    }

    payload = vec4(col, 0.0);
}
