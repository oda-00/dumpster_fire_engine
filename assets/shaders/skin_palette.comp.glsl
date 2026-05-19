// Skin-palette compute shader.
//
// Builds the per-joint world-space transform palette used by a skinned mesh:
//   palette[i] = worldMatrix[joints[i]] * inverseBindMatrix[i]
//
// Bindings (mirror the forge's compute descriptor layout):
//   set 0, binding 0  primary   — joint world-matrices (joint_count × mat4, 16 floats each)
//   set 0, binding 1  secondary — inverse bind matrices (joint_count × mat4, 16 floats each)
//   set 0, binding 2  result    — output palette        (joint_count × mat4, 16 floats each)
//
// Each invocation handles one joint.  Dispatch ceil(joint_count / 64) workgroups.
//
// Compile:
//   glslangValidator -V -S comp skin_palette.comp.glsl -o skin_palette.comp.glsl.spv

#version 450
layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

layout(set = 0, binding = 0) readonly  buffer Primary   { float data[]; } world_mats;
layout(set = 0, binding = 1) readonly  buffer Secondary { float data[]; } ibms;
layout(set = 0, binding = 2) writeonly buffer Result    { float data[]; } palette;

mat4 read_mat4_world(uint i) {
    uint b = i * 16u;
    return mat4(
        world_mats.data[b+ 0], world_mats.data[b+ 1], world_mats.data[b+ 2], world_mats.data[b+ 3],
        world_mats.data[b+ 4], world_mats.data[b+ 5], world_mats.data[b+ 6], world_mats.data[b+ 7],
        world_mats.data[b+ 8], world_mats.data[b+ 9], world_mats.data[b+10], world_mats.data[b+11],
        world_mats.data[b+12], world_mats.data[b+13], world_mats.data[b+14], world_mats.data[b+15]
    );
}

mat4 read_mat4_ibm(uint i) {
    uint b = i * 16u;
    return mat4(
        ibms.data[b+ 0], ibms.data[b+ 1], ibms.data[b+ 2], ibms.data[b+ 3],
        ibms.data[b+ 4], ibms.data[b+ 5], ibms.data[b+ 6], ibms.data[b+ 7],
        ibms.data[b+ 8], ibms.data[b+ 9], ibms.data[b+10], ibms.data[b+11],
        ibms.data[b+12], ibms.data[b+13], ibms.data[b+14], ibms.data[b+15]
    );
}

void write_mat4(uint i, mat4 m) {
    uint b = i * 16u;
    palette.data[b+ 0] = m[0][0]; palette.data[b+ 1] = m[0][1];
    palette.data[b+ 2] = m[0][2]; palette.data[b+ 3] = m[0][3];
    palette.data[b+ 4] = m[1][0]; palette.data[b+ 5] = m[1][1];
    palette.data[b+ 6] = m[1][2]; palette.data[b+ 7] = m[1][3];
    palette.data[b+ 8] = m[2][0]; palette.data[b+ 9] = m[2][1];
    palette.data[b+10] = m[2][2]; palette.data[b+11] = m[2][3];
    palette.data[b+12] = m[3][0]; palette.data[b+13] = m[3][1];
    palette.data[b+14] = m[3][2]; palette.data[b+15] = m[3][3];
}

void main() {
    uint i = gl_GlobalInvocationID.x;
    // joint_count derived from primary buffer length (each joint = 16 floats)
    uint joint_count = uint(world_mats.data.length()) / 16u;
    if (i >= joint_count) return;

    mat4 world  = read_mat4_world(i);
    mat4 ibm    = read_mat4_ibm(i);
    write_mat4(i, world * ibm);
}
