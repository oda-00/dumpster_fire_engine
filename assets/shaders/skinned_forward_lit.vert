#version 450

// Per-vertex inputs from binding 0 (same ForgeVertex layout, 48-byte stride).
layout(location = 0) in vec3 inPosition;
layout(location = 1) in vec3 inNormal;
layout(location = 2) in vec4 inTangent;
layout(location = 3) in vec2 inUv;

// Per-vertex inputs from binding 1 — SkinVertex: joints[4] u16 packed into
// uvec2 + weights[4] f32 (32-byte stride).
layout(location = 4) in uvec2 inJointsPacked; // 4 × u16 in 2 × u32
layout(location = 5) in vec4  inWeights;

// MVP push constant.
layout(push_constant) uniform Pc {
    mat4 mvp;
} pc;

// Set 2 binding 0 — skin palette SSBO, written by the SkinPalette compute Ore.
// One mat4 per joint of the source skin.
layout(set = 2, binding = 0, std430) readonly buffer SkinPalette {
    mat4 joints[];
} palette;

// Set 3 binding 0 — per-instance mat4 offsets (EXT_mesh_gpu_instancing).
// Single identity for non-instanced draws; per-instance offsets for
// instanced draws. Composed with the skinning result before MVP.
layout(set = 3, binding = 0, std430) readonly buffer Instances {
    mat4 m[];
} instances;

layout(location = 0) out vec3 outNormal;
layout(location = 1) out vec2 outUv;

void main() {
    uvec4 j = uvec4(
        (inJointsPacked.x      ) & 0xffffu,
        (inJointsPacked.x >> 16) & 0xffffu,
        (inJointsPacked.y      ) & 0xffffu,
        (inJointsPacked.y >> 16) & 0xffffu
    );

    // Linear-blend skinning. Fall back to identity when weights sum to zero
    // (defensive — well-formed glTF normalises weights to sum to 1).
    float wsum = inWeights.x + inWeights.y + inWeights.z + inWeights.w;
    mat4 skin = (wsum > 0.0)
        ? (inWeights.x * palette.joints[j.x]
         + inWeights.y * palette.joints[j.y]
         + inWeights.z * palette.joints[j.z]
         + inWeights.w * palette.joints[j.w])
        : mat4(1.0);

    mat4 instance_offset = instances.m[gl_InstanceIndex];
    vec4 posed_pos = instance_offset * skin * vec4(inPosition, 1.0);
    gl_Position    = pc.mvp * posed_pos;

    // Transform normal by the upper-left 3x3 of (instance * skin). For uniform
    // scale this is exact; for non-uniform scale the engine would need to
    // pass an inverse-transpose palette — out of scope here.
    outNormal = mat3(instance_offset) * mat3(skin) * inNormal;
    outUv     = inUv;
}
