#version 450

// Smoke-test compute shader for the forge/factory/renderer pipeline.
// Reads `primary[i]`, writes `primary[i] * 2` to `result[i]`.
// `secondary` is unused but must be declared because the forge's descriptor
// layout (see ORE_SECONDARY_BINDING in src/forge_master/forge.rs) always
// binds it.
//
// Regenerate the .spv next to this file with:
//   glslc -fshader-stage=compute -O demo_doubler.comp.glsl -o demo_doubler.spv

layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

layout(set = 0, binding = 0) readonly buffer Primary {
    uint data[];
} primary;

layout(set = 0, binding = 1) readonly buffer Secondary {
    uint data[];
} secondary;

layout(set = 0, binding = 2) writeonly buffer Result {
    uint data[];
} result;

void main() {
    uint i = gl_GlobalInvocationID.x;
    if (i >= primary.data.length()) {
        return;
    }
    result.data[i] = primary.data[i] * 2u;
}
