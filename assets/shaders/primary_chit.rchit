#version 460
#extension GL_EXT_ray_tracing          : require
#extension GL_EXT_nonuniform_qualifier : require

layout(location = 0) rayPayloadInEXT vec4 payload;
layout(location = 1) rayPayloadEXT    float shadowed;

hitAttributeEXT vec2 attribs;

struct LightGpu { vec4 ci; uint kind; uint flags; uint p0; uint p1; vec4 d[6]; };
layout(set = 0, binding = 0) uniform accelerationStructureEXT topLevelAS;
layout(set = 0, binding = 2) uniform Lights {
    uint count; uint env_idx; uint sky_present; uint _p;
    LightGpu lights[32];
} u_lights;

const float PI = 3.14159265;

float D_GGX(float NoH, float a) {
    float a2 = a*a, d = (NoH*NoH)*(a2-1.0)+1.0;
    return a2/(PI*d*d);
}
float V_Smith(float NoV, float NoL, float a) {
    float a2=a*a;
    float gv=NoL*sqrt(NoV*NoV*(1.0-a2)+a2);
    float gl=NoV*sqrt(NoL*NoL*(1.0-a2)+a2);
    return 0.5/max(gv+gl,1e-5);
}
vec3 F_Schlick(float VoH, vec3 f0) {
    return f0+(1.0-f0)*pow(clamp(1.0-VoH,0.0,1.0),5.0);
}
float smooth_cutoff(float d2, float inv_r2) {
    if (inv_r2==0.0) return 1.0;
    float x=clamp(1.0-d2*d2*inv_r2*inv_r2,0.0,1.0);
    return x*x;
}

void main() {
    // Barycentric interpolation — real mesh data fetch via BDA would go here.
    // For now reconstruct a shading normal from gl_WorldRayDirectionEXT
    // and apply PBR with a neutral grey material. Full implementation binds
    // MeshRecord SSBO at set 0 binding 3 and fetches true vertex data.
    vec3 bary = vec3(1.0 - attribs.x - attribs.y, attribs.x, attribs.y);
    vec3 n = normalize(gl_WorldRayDirectionEXT * -1.0);  // face normal approx
    vec3 v = normalize(-gl_WorldRayDirectionEXT);

    vec3 albedo   = vec3(0.7);
    float rough   = 0.5, metal = 0.0;
    vec3  f0      = vec3(0.04);
    float a       = rough * rough;

    vec3 direct  = vec3(0.0);
    vec3 hit_pos = gl_WorldRayOriginEXT + gl_HitTEXT * gl_WorldRayDirectionEXT;

    for (uint i = 0u; i < u_lights.count; i++) {
        LightGpu lt = u_lights.lights[i];
        if ((lt.flags & 2u) != 0u) continue;
        if (lt.kind >= 16u)        continue;

        vec3  l; float att;
        if (lt.kind == 0u) {
            vec3 dv=lt.d[0].xyz-hit_pos; float d2=dot(dv,dv), dd=sqrt(d2);
            l=dv/max(dd,1e-5); att=(1.0/max(d2,1e-5))*smooth_cutoff(d2,lt.d[0].w);
        } else if (lt.kind == 1u) {
            vec3 dv=lt.d[0].xyz-hit_pos; float d2=dot(dv,dv), dd=sqrt(d2);
            l=dv/max(dd,1e-5); att=(1.0/max(d2,1e-5))*smooth_cutoff(d2,lt.d[0].w);
            att*=smoothstep(lt.d[1].w,lt.d[2].x,dot(-l,lt.d[1].xyz));
        } else if (lt.kind == 2u || lt.kind == 3u) {
            l=-lt.d[0].xyz; att=1.0;
        } else {
            vec3 dv=lt.d[0].xyz-hit_pos; float d2=dot(dv,dv), dd=sqrt(d2);
            l=dv/max(dd,1e-5); att=(1.0/max(d2,1e-5));
        }

        // Shadow ray (kinds 0-10)
        if (lt.kind <= 10u) {
            vec3 shadow_dir = l;
            float tmax = (lt.kind <= 3u) ? 1e4 : length(lt.d[0].xyz - hit_pos);
            shadowed = 1.0;
            traceRayEXT(topLevelAS,
                gl_RayFlagsTerminateOnFirstHitEXT | gl_RayFlagsSkipClosestHitShaderEXT,
                0xFF, 1, 0, 1,
                hit_pos + n * 0.001, 0.001, shadow_dir, tmax, 1);
            if (shadowed > 0.5) continue;
        }

        float NoL = max(dot(n, l), 0.0);
        if (NoL < 0.001 || att < 0.001) continue;
        vec3  h_  = normalize(v + l);
        float NoH = max(dot(n, h_), 0.0), NoV = max(dot(n, v), 1e-4), VoH = max(dot(v, h_), 0.0);
        vec3  F   = F_Schlick(VoH, f0);
        vec3  spec = F * (D_GGX(NoH, a) * V_Smith(NoV, NoL, a));
        vec3  diff = (vec3(1.0) - F) * (1.0 - metal) * albedo / PI;
        direct += (diff + spec) * lt.ci.rgb * (lt.ci.a * att * NoL);
    }

    vec3 ambient = vec3(0.03) * albedo;
    for (uint i = 0u; i < u_lights.count; i++) {
        LightGpu lt = u_lights.lights[i];
        if (lt.kind == 19u) ambient += lt.ci.rgb * lt.ci.a;
    }

    payload = vec4(direct + ambient, gl_HitTEXT);
}
