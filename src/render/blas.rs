//! Bottom-Level Acceleration Structure (BLAS) builder.
//!
//! One BLAS per mesh primitive (matched glTF granularity). Built once at
//! asset upload time; never updated. Reads vertex + index buffers via their
//! device addresses (Buffer Device Address, requires VK_KHR_buffer_device_address).
//!
//! Phase 5 of the lighting / RT rollout. Constructed only when
//! `VulkanContext::has_ray_tracing` is true; otherwise the engine never
//! references these and the BLAS handles stay `vk::AccelerationStructureKHR::null()`.
//!
//! Usage:
//!   1. Caller ensures vertex / index buffers were created with
//!      `vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS |
//!       vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR`
//!      in addition to whatever raster usage flags they need.
//!   2. Call `build_blas(...)` per primitive (one-shot command buffer; blocks
//!      until the build completes; scratch buffer destroyed before return).
//!   3. Store the returned `(vk::AccelerationStructureKHR, ForgeBuffer)`
//!      alongside the primitive — `ForgeBuffer` is the backing storage; the
//!      acceleration structure handle indexes into that storage's device
//!      address space.
//!   4. On teardown: `accel_ext.destroy_acceleration_structure(handle, None)`
//!      THEN `buffer.destroy(device)`.

use ash::vk;

use crate::forge_master::ore::ForgeBuffer;
use crate::forge_master::master::{ForgeError, ForgeResult};

/// Inputs to `build_blas`. Names mirror the Vulkan calls one-to-one.
pub struct BlasBuildInputs<'a> {
    pub device:          &'a ash::Device,
    pub accel_ext:       &'a ash::khr::acceleration_structure::Device,
    pub memory_props:    &'a vk::PhysicalDeviceMemoryProperties,
    pub command_pool:    vk::CommandPool,
    pub queue:           vk::Queue,

    /// Vertex buffer handle; must have been created with
    /// `SHADER_DEVICE_ADDRESS | ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR`.
    pub vertex_buffer:   vk::Buffer,
    /// Byte offset into `vertex_buffer` where the position-bearing vertex
    /// array begins. Usually 0.
    pub vertex_offset:   vk::DeviceSize,
    /// Total vertex count in the array (used as `max_vertex` — Vulkan spec
    /// requires "the highest index of any vertex" which is `count - 1`).
    pub vertex_count:    u32,
    /// Stride in bytes between consecutive vertices.
    pub vertex_stride:   vk::DeviceSize,
    /// Format of the position component. Typically `R32G32B32_SFLOAT`.
    pub vertex_format:   vk::Format,

    pub index_buffer:    vk::Buffer,
    pub index_offset:    vk::DeviceSize,
    pub index_count:     u32,
    /// Usually `vk::IndexType::UINT32`. Triangle count = index_count / 3.
    pub index_type:      vk::IndexType,

    /// 3×4 row-major transform applied to vertices at build time. Pass
    /// `None` to use the identity (the common case — instance transforms
    /// live on the TLAS side via `VkAccelerationStructureInstanceKHR`).
    pub transform:       Option<vk::TransformMatrixKHR>,
}

/// Build one BLAS. Blocks on a fence; transient scratch buffer is freed.
/// Returns `(handle, backing_buffer)` — caller owns both. Destroy via:
///   `accel_ext.destroy_acceleration_structure(handle, None);`
///   `backing_buffer.destroy(device);`
pub fn build_blas(input: &BlasBuildInputs<'_>) -> ForgeResult<(vk::AccelerationStructureKHR, ForgeBuffer)> {
    let device      = input.device;
    let accel       = input.accel_ext;
    let mem_props   = input.memory_props;

    // ── 1. Geometry description ───────────────────────────────────────────
    let vb_addr = unsafe {
        device.get_buffer_device_address(
            &vk::BufferDeviceAddressInfo::default().buffer(input.vertex_buffer),
        )
    };
    let ib_addr = unsafe {
        device.get_buffer_device_address(
            &vk::BufferDeviceAddressInfo::default().buffer(input.index_buffer),
        )
    };

    let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
        .vertex_format(input.vertex_format)
        .vertex_data(vk::DeviceOrHostAddressConstKHR {
            device_address: vb_addr + input.vertex_offset,
        })
        .vertex_stride(input.vertex_stride)
        .max_vertex(input.vertex_count.saturating_sub(1))
        .index_type(input.index_type)
        .index_data(vk::DeviceOrHostAddressConstKHR {
            device_address: ib_addr + input.index_offset,
        });

    // NOTE: `transform_data` accepts a device address pointing at a
    // `VkTransformMatrixKHR`. For the common case (no per-vertex transform —
    // instance-level placement is on the TLAS via
    // `VkAccelerationStructureInstanceKHR.transform`) we leave it null and
    // Vulkan interprets the geometry as untransformed. If a caller passes
    // a `transform`, they must have uploaded a `VkTransformMatrixKHR` and
    // would need to extend this API to carry that buffer's BDA — out of
    // scope for the static-mesh editor use we exercise today.
    let _ = input.transform;

    let geometry = vk::AccelerationStructureGeometryKHR::default()
        .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
        .geometry(vk::AccelerationStructureGeometryDataKHR { triangles })
        .flags(vk::GeometryFlagsKHR::OPAQUE);

    let geometries = [geometry];

    // ── 2. Build sizes query ──────────────────────────────────────────────
    let mut build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
        .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
        .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
        .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
        .geometries(&geometries);

    let triangle_count = input.index_count / 3;
    let max_primitive_counts = [triangle_count];

    let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
    unsafe {
        accel.get_acceleration_structure_build_sizes(
            vk::AccelerationStructureBuildTypeKHR::DEVICE,
            &build_info,
            &max_primitive_counts,
            &mut sizes,
        );
    }

    // ── 3. Allocate backing buffer + scratch ─────────────────────────────
    let result_buffer = ForgeBuffer::create(
        device,
        mem_props,
        sizes.acceleration_structure_size,
        vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;

    let mut scratch_buffer = ForgeBuffer::create(
        device,
        mem_props,
        sizes.build_scratch_size,
        vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;

    let scratch_addr = unsafe {
        device.get_buffer_device_address(
            &vk::BufferDeviceAddressInfo::default().buffer(scratch_buffer.handle),
        )
    };

    // ── 4. Create the acceleration structure handle on result_buffer ──────
    let create_info = vk::AccelerationStructureCreateInfoKHR::default()
        .buffer(result_buffer.handle)
        .offset(0)
        .size(sizes.acceleration_structure_size)
        .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);
    let as_handle = unsafe {
        accel.create_acceleration_structure(&create_info, None).map_err(ForgeError::Vk)?
    };

    build_info = build_info
        .dst_acceleration_structure(as_handle)
        .scratch_data(vk::DeviceOrHostAddressKHR { device_address: scratch_addr });

    // ── 5. Record + submit the build ─────────────────────────────────────
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(input.command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let cb = unsafe {
        device.allocate_command_buffers(&alloc_info).map_err(ForgeError::Vk)?[0]
    };

    let begin = vk::CommandBufferBeginInfo::default()
        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    unsafe { device.begin_command_buffer(cb, &begin).map_err(ForgeError::Vk)?; }

    let range = vk::AccelerationStructureBuildRangeInfoKHR::default()
        .primitive_count(triangle_count)
        .primitive_offset(0)
        .first_vertex(0)
        .transform_offset(0);
    let ranges_one: [vk::AccelerationStructureBuildRangeInfoKHR; 1] = [range];
    let ranges_outer: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [&ranges_one];

    unsafe {
        // ash takes parallel slices: one `BuildGeometryInfo` per BLAS, and
        // one outer slice of inner range-info slices (one entry per BLAS,
        // each containing one range per geometry — we have 1 geometry).
        accel.cmd_build_acceleration_structures(
            cb,
            std::slice::from_ref(&build_info),
            &ranges_outer,
        );
    }

    unsafe { device.end_command_buffer(cb).map_err(ForgeError::Vk)?; }

    let cbs = [cb];
    let submit = vk::SubmitInfo::default().command_buffers(&cbs);
    let fence = unsafe {
        device.create_fence(&vk::FenceCreateInfo::default(), None).map_err(ForgeError::Vk)?
    };

    unsafe {
        device.queue_submit(input.queue, &[submit], fence).map_err(ForgeError::Vk)?;
        device.wait_for_fences(&[fence], true, u64::MAX).map_err(ForgeError::Vk)?;
        device.destroy_fence(fence, None);
        device.free_command_buffers(input.command_pool, &cbs);
        scratch_buffer.destroy(device);
    }

    Ok((as_handle, result_buffer))
}

/// Convenience: device address of a finished BLAS, used by TLAS instance
/// records (`VkAccelerationStructureInstanceKHR.accelerationStructureReference`).
pub fn blas_device_address(
    accel_ext: &ash::khr::acceleration_structure::Device,
    handle:    vk::AccelerationStructureKHR,
) -> u64 {
    let info = vk::AccelerationStructureDeviceAddressInfoKHR::default()
        .acceleration_structure(handle);
    unsafe { accel_ext.get_acceleration_structure_device_address(&info) }
}
