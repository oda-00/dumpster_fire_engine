#version 450

// Per-vertex inputs (matches ForgeVertex layout: 48-byte stride)
layout(location = 0) in vec3 inPosition;
layout(location = 1) in vec3 inNormal;
layout(location = 2) in vec4 inTangent;
layout(location = 3) in vec2 inUv;

// Push constant: model-view-projection matrix (column-major, 64 bytes).
// Set to identity if no camera is needed.
layout(push_constant) uniform Pc {
    mat4 mvp;
} pc;

layout(location = 0) out vec3 outNormal;
layout(location = 1) out vec2 outUv;

void main() {
    gl_Position = pc.mvp * vec4(inPosition, 1.0);
    outNormal   = inNormal;
    outUv       = inUv;
}
