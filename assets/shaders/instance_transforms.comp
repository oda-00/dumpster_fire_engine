#version 450
//
// EXT_mesh_gpu_instancing transform expansion.
//
// One thread per instance: read (translation: vec3, rotation: vec4,
// scale: vec3) from the input SSBO, compose them into a column-major
// mat4 (translate * rotate * scale, the standard glTF TRS order), and
// write the result to the output SSBO at the same index. The output
// is what the ForwardLit + SkinnedForwardLit vertex shaders read at
// set 3 binding 0 via `instances.m[gl_InstanceIndex]`.
//
// Input SSBO layout per instance (48 bytes std430):
//   vec3  translation  (16 bytes — vec3 in std430 is vec4-aligned)
//   vec4  rotation     (16 bytes)
//   vec3  scale        (16 bytes — same vec3-pad-to-vec4 rule)

layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

struct InstanceTRS {
    vec4 translation_pad; // .xyz = translation; .w ignored
    vec4 rotation;        // (x, y, z, w) quaternion
    vec4 scale_pad;       // .xyz = scale; .w ignored
};

layout(set = 0, binding = 0, std430) readonly buffer Inputs {
    InstanceTRS trs[];
} in_buf;

layout(set = 0, binding = 1, std430) writeonly buffer Outputs {
    mat4 m[];
} out_buf;

layout(push_constant) uniform Pc {
    uint count;
} pc;

mat4 quat_to_mat4(vec4 q, vec3 t, vec3 s) {
    float x = q.x, y = q.y, z = q.z, w = q.w;
    float xx = x*x, yy = y*y, zz = z*z;
    float xy = x*y, xz = x*z, yz = y*z;
    float wx = w*x, wy = w*y, wz = w*z;
    mat4 r;
    r[0] = vec4((1.0 - 2.0*(yy + zz)) * s.x,    2.0*(xy + wz) * s.x,     2.0*(xz - wy) * s.x, 0.0);
    r[1] = vec4(    2.0*(xy - wz) * s.y,    (1.0 - 2.0*(xx + zz)) * s.y,  2.0*(yz + wx) * s.y, 0.0);
    r[2] = vec4(    2.0*(xz + wy) * s.z,        2.0*(yz - wx) * s.z, (1.0 - 2.0*(xx + yy)) * s.z, 0.0);
    r[3] = vec4(t.x, t.y, t.z, 1.0);
    return r;
}

void main() {
    uint id = gl_GlobalInvocationID.x;
    if (id >= pc.count) return;
    InstanceTRS in_ = in_buf.trs[id];
    out_buf.m[id] = quat_to_mat4(in_.rotation, in_.translation_pad.xyz, in_.scale_pad.xyz);
}
