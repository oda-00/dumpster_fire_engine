#version 450

// Per-vertex inputs (matches ForgeVertex layout: 48-byte stride)
layout(location = 0) in vec3 inPosition;
layout(location = 1) in vec3 inNormal;
layout(location = 2) in vec4 inTangent;
layout(location = 3) in vec2 inUv;

// Push constant: model-view-projection matrix (column-major, 64 bytes).
layout(push_constant) uniform Pc {
    mat4 mvp;
} pc;

// Set 3 binding 0: per-instance mat4 offsets (EXT_mesh_gpu_instancing).
// For non-instanced draws the buffer contains a single identity matrix at
// index 0; gl_InstanceIndex is 0, so the multiplication is a no-op. For
// instanced draws each instance reads its own mat4 here and applies it
// before the MVP transform.
layout(set = 3, binding = 0) readonly buffer Instances {
    mat4 m[];
} instances;

layout(location = 0) out vec3 outNormal;
layout(location = 1) out vec2 outUv;

void main() {
    mat4 instance_offset = instances.m[gl_InstanceIndex];
    vec4 world_pos = instance_offset * vec4(inPosition, 1.0);
    gl_Position = pc.mvp * world_pos;
    outNormal   = mat3(instance_offset) * inNormal;
    outUv       = inUv;
}
