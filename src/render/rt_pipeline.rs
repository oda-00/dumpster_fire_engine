//! Phase 9 — Ray-tracing pipeline + Shader Binding Table + TLAS rebuild.
//!
//! Single RT pipeline with 4 shader groups:
//!   group 0: raygen        — assets/shaders/raygen.rgen.spv
//!   group 1: primary miss  — assets/shaders/primary_miss.rmiss.spv
//!   group 2: shadow miss   — assets/shaders/shadow_miss.rmiss.spv
//!   group 3: primary chit  — assets/shaders/primary_chit.rchit.spv
//!
//! SBT layout: one raygen entry, two miss entries (primary + shadow),
//! one hit entry. Aligned per `rt_props.shader_group_handle_alignment` and
//! `shader_group_base_alignment`.
//!
//! TLAS is rebuilt per frame from a caller-supplied instance list. Editor
//! scenes have < 200 instances so per-frame rebuild is ~µs.

use ash::vk;
use thin_vec::ThinVec;

use crate::forge_master::master::{ForgeError, ForgeResult};
use crate::forge_master::ore::ForgeBuffer;
use crate::render::vulkan::VulkanContext;

pub struct RtPipeline {
    pub pipeline: vk::Pipeline,
    pub pipeline_layout: vk::PipelineLayout,
    pub set_layout: vk::DescriptorSetLayout,

    /// SBT backing buffer — single buffer holding raygen, miss, and hit
    /// regions back-to-back with `base_alignment`-aligned region starts.
    pub sbt_buffer: ForgeBuffer,
    pub raygen_region: vk::StridedDeviceAddressRegionKHR,
    pub miss_region: vk::StridedDeviceAddressRegionKHR,
    pub hit_region: vk::StridedDeviceAddressRegionKHR,
    pub callable_region: vk::StridedDeviceAddressRegionKHR,

    /// Storage for the most recently rebuilt TLAS instance buffer.
    pub instance_buf: Option<ForgeBuffer>,
    pub tlas: vk::AccelerationStructureKHR,
    pub tlas_buf: Option<ForgeBuffer>,
    pub tlas_scratch: Option<ForgeBuffer>,
}

impl RtPipeline {
    pub fn new(vulkan: &VulkanContext) -> ForgeResult<Self> {
        let device = &vulkan.device;
        let rt_loader = vulkan
            .rt_pipeline
            .as_ref()
            .ok_or(ForgeError::NoPhysicalDevice)?;

        // ── Descriptor set layout (bindings per Plan §9) ───────────────────
        // 0 TLAS (AS), 1 storage image (HDR), 2 lights UBO
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
                .descriptor_count(1)
                .stage_flags(
                    vk::ShaderStageFlags::RAYGEN_KHR | vk::ShaderStageFlags::CLOSEST_HIT_KHR,
                ),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::RAYGEN_KHR),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(
                    vk::ShaderStageFlags::RAYGEN_KHR
                        | vk::ShaderStageFlags::MISS_KHR
                        | vk::ShaderStageFlags::CLOSEST_HIT_KHR,
                ),
        ];
        let set_layout = unsafe {
            device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(ForgeError::Vk)?
        };

        // RayCameraPush — 128 bytes per plan §2.
        let push_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::RAYGEN_KHR)
            .offset(0)
            .size(128);
        let set_layouts = [set_layout];
        let push_ranges = [push_range];
        let pipeline_layout = unsafe {
            device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&set_layouts)
                        .push_constant_ranges(&push_ranges),
                    None,
                )
                .map_err(ForgeError::Vk)?
        };

        // ── Shader modules ────────────────────────────────────────────────
        let rgen_spv = include_bytes!("../../assets/shaders/raygen.rgen.spv");
        let pmiss_spv = include_bytes!("../../assets/shaders/primary_miss.rmiss.spv");
        let smiss_spv = include_bytes!("../../assets/shaders/shadow_miss.rmiss.spv");
        let chit_spv = include_bytes!("../../assets/shaders/primary_chit.rchit.spv");

        let rgen_mod = create_shader_module(device, rgen_spv)?;
        let pmiss_mod = create_shader_module(device, pmiss_spv)?;
        let smiss_mod = create_shader_module(device, smiss_spv)?;
        let chit_mod = create_shader_module(device, chit_spv)?;
        let entry_name = std::ffi::CString::new("main").unwrap();

        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::RAYGEN_KHR)
                .module(rgen_mod)
                .name(&entry_name),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::MISS_KHR)
                .module(pmiss_mod)
                .name(&entry_name),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::MISS_KHR)
                .module(smiss_mod)
                .name(&entry_name),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::CLOSEST_HIT_KHR)
                .module(chit_mod)
                .name(&entry_name),
        ];

        // 4 groups: raygen / miss / miss / hit
        let groups = [
            vk::RayTracingShaderGroupCreateInfoKHR::default()
                .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
                .general_shader(0)
                .closest_hit_shader(vk::SHADER_UNUSED_KHR)
                .any_hit_shader(vk::SHADER_UNUSED_KHR)
                .intersection_shader(vk::SHADER_UNUSED_KHR),
            vk::RayTracingShaderGroupCreateInfoKHR::default()
                .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
                .general_shader(1)
                .closest_hit_shader(vk::SHADER_UNUSED_KHR)
                .any_hit_shader(vk::SHADER_UNUSED_KHR)
                .intersection_shader(vk::SHADER_UNUSED_KHR),
            vk::RayTracingShaderGroupCreateInfoKHR::default()
                .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
                .general_shader(2)
                .closest_hit_shader(vk::SHADER_UNUSED_KHR)
                .any_hit_shader(vk::SHADER_UNUSED_KHR)
                .intersection_shader(vk::SHADER_UNUSED_KHR),
            vk::RayTracingShaderGroupCreateInfoKHR::default()
                .ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
                .general_shader(vk::SHADER_UNUSED_KHR)
                .closest_hit_shader(3)
                .any_hit_shader(vk::SHADER_UNUSED_KHR)
                .intersection_shader(vk::SHADER_UNUSED_KHR),
        ];

        let create_info = vk::RayTracingPipelineCreateInfoKHR::default()
            .stages(&stages)
            .groups(&groups)
            .max_pipeline_ray_recursion_depth(2)
            .layout(pipeline_layout);

        let pipelines = unsafe {
            rt_loader
                .create_ray_tracing_pipelines(
                    vk::DeferredOperationKHR::null(),
                    vk::PipelineCache::null(),
                    &[create_info],
                    None,
                )
                .map_err(|(_, e)| ForgeError::Vk(e))?
        };
        let pipeline = pipelines[0];
        unsafe {
            device.destroy_shader_module(rgen_mod, None);
            device.destroy_shader_module(pmiss_mod, None);
            device.destroy_shader_module(smiss_mod, None);
            device.destroy_shader_module(chit_mod, None);
        }

        // ── Shader Binding Table ──────────────────────────────────────────
        let handle_size = vulkan.rt_props.shader_group_handle_size as u64;
        let base_alignment = vulkan.rt_props.shader_group_base_alignment as u64;
        let handle_alignment = vulkan.rt_props.shader_group_handle_alignment as u64;

        // 4 groups → 4 handles. Region strides: raygen has 1 entry, miss has 2,
        // hit has 1; all regions base-aligned.
        let aligned_handle_size = align_up(handle_size, handle_alignment);
        let raygen_size = align_up(aligned_handle_size, base_alignment);
        let miss_size = align_up(aligned_handle_size * 2, base_alignment);
        let hit_size = align_up(aligned_handle_size, base_alignment);
        let sbt_size = raygen_size + miss_size + hit_size;

        let handle_data = unsafe {
            rt_loader
                .get_ray_tracing_shader_group_handles(pipeline, 0, 4, (handle_size * 4) as usize)
                .map_err(ForgeError::Vk)?
        };

        let mut sbt_staging = vec![0u8; sbt_size as usize];
        let hsz = handle_size as usize;
        // raygen → offset 0
        sbt_staging[0..hsz].copy_from_slice(&handle_data[0..hsz]);
        // miss[0] (primary)
        let miss_offset = raygen_size as usize;
        sbt_staging[miss_offset..miss_offset + hsz].copy_from_slice(&handle_data[hsz..2 * hsz]);
        // miss[1] (shadow) at miss_offset + aligned_handle_size
        let miss1_off = miss_offset + aligned_handle_size as usize;
        sbt_staging[miss1_off..miss1_off + hsz].copy_from_slice(&handle_data[2 * hsz..3 * hsz]);
        // hit
        let hit_offset = (raygen_size + miss_size) as usize;
        sbt_staging[hit_offset..hit_offset + hsz].copy_from_slice(&handle_data[3 * hsz..4 * hsz]);

        let mut sbt_buffer = ForgeBuffer::create(
            device,
            &vulkan.memory_properties,
            sbt_size,
            vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        sbt_buffer.write_bytes(device, &sbt_staging)?;
        let sbt_addr = unsafe {
            device.get_buffer_device_address(
                &vk::BufferDeviceAddressInfo::default().buffer(sbt_buffer.handle),
            )
        };

        let raygen_region = vk::StridedDeviceAddressRegionKHR::default()
            .device_address(sbt_addr)
            .stride(raygen_size)
            .size(raygen_size);
        let miss_region = vk::StridedDeviceAddressRegionKHR::default()
            .device_address(sbt_addr + raygen_size)
            .stride(aligned_handle_size)
            .size(miss_size);
        let hit_region = vk::StridedDeviceAddressRegionKHR::default()
            .device_address(sbt_addr + raygen_size + miss_size)
            .stride(aligned_handle_size)
            .size(hit_size);
        let callable_region = vk::StridedDeviceAddressRegionKHR::default();

        Ok(Self {
            pipeline,
            pipeline_layout,
            set_layout,
            sbt_buffer,
            raygen_region,
            miss_region,
            hit_region,
            callable_region,
            instance_buf: None,
            tlas: vk::AccelerationStructureKHR::null(),
            tlas_buf: None,
            tlas_scratch: None,
        })
    }

    /// Rebuild the TLAS from `instances`. Blocks on a fence — single frame
    /// editor budget is ≪ 1 ms for < 200 instances.
    pub fn rebuild_tlas(
        &mut self,
        vulkan: &VulkanContext,
        instances: &[vk::AccelerationStructureInstanceKHR],
    ) -> ForgeResult<()> {
        let device = &vulkan.device;
        let accel = vulkan
            .rt_accel
            .as_ref()
            .ok_or(ForgeError::NoPhysicalDevice)?;

        // Free previous instance / TLAS buffers.
        if let Some(mut b) = self.instance_buf.take() {
            unsafe {
                b.destroy(device);
            }
        }
        if self.tlas != vk::AccelerationStructureKHR::null() {
            unsafe {
                accel.destroy_acceleration_structure(self.tlas, None);
            }
            self.tlas = vk::AccelerationStructureKHR::null();
        }
        if let Some(mut b) = self.tlas_buf.take() {
            unsafe {
                b.destroy(device);
            }
        }
        if let Some(mut b) = self.tlas_scratch.take() {
            unsafe {
                b.destroy(device);
            }
        }

        if instances.is_empty() {
            return Ok(());
        }

        let inst_bytes_len = std::mem::size_of_val(instances);
        let mut inst_buf = ForgeBuffer::create(
            device,
            &vulkan.memory_properties,
            inst_bytes_len as u64,
            vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let bytes =
            unsafe { std::slice::from_raw_parts(instances.as_ptr() as *const u8, inst_bytes_len) };
        inst_buf.write_bytes(device, bytes)?;
        let inst_addr = unsafe {
            device.get_buffer_device_address(
                &vk::BufferDeviceAddressInfo::default().buffer(inst_buf.handle),
            )
        };

        let geometry = vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::INSTANCES)
            .geometry(vk::AccelerationStructureGeometryDataKHR {
                instances: vk::AccelerationStructureGeometryInstancesDataKHR::default()
                    .array_of_pointers(false)
                    .data(vk::DeviceOrHostAddressConstKHR {
                        device_address: inst_addr,
                    }),
            });
        let geometries = [geometry];
        let mut build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .flags(
                vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE
                    | vk::BuildAccelerationStructureFlagsKHR::ALLOW_UPDATE,
            )
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&geometries);
        let prim_counts = [instances.len() as u32];
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            accel.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &build_info,
                &prim_counts,
                &mut sizes,
            );
        }

        let tlas_buf = ForgeBuffer::create(
            device,
            &vulkan.memory_properties,
            sizes.acceleration_structure_size,
            vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let scratch_buf = ForgeBuffer::create(
            device,
            &vulkan.memory_properties,
            sizes.build_scratch_size,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let scratch_addr = unsafe {
            device.get_buffer_device_address(
                &vk::BufferDeviceAddressInfo::default().buffer(scratch_buf.handle),
            )
        };

        let create_info = vk::AccelerationStructureCreateInfoKHR::default()
            .buffer(tlas_buf.handle)
            .size(sizes.acceleration_structure_size)
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL);
        let tlas = unsafe {
            accel
                .create_acceleration_structure(&create_info, None)
                .map_err(ForgeError::Vk)?
        };

        build_info =
            build_info
                .dst_acceleration_structure(tlas)
                .scratch_data(vk::DeviceOrHostAddressKHR {
                    device_address: scratch_addr,
                });

        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(vulkan.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cb = unsafe {
            device
                .allocate_command_buffers(&alloc)
                .map_err(ForgeError::Vk)?[0]
        };
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            device
                .begin_command_buffer(cb, &begin)
                .map_err(ForgeError::Vk)?;
        }

        let range = vk::AccelerationStructureBuildRangeInfoKHR::default()
            .primitive_count(instances.len() as u32);
        let ranges = [range];
        let ranges_outer: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [&ranges];
        unsafe {
            accel.cmd_build_acceleration_structures(
                cb,
                std::slice::from_ref(&build_info),
                &ranges_outer,
            );
            device.end_command_buffer(cb).map_err(ForgeError::Vk)?;
        }
        let cbs = [cb];
        let submit = vk::SubmitInfo::default().command_buffers(&cbs);
        let fence = unsafe {
            device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .map_err(ForgeError::Vk)?
        };
        unsafe {
            device
                .queue_submit(vulkan.queue, &[submit], fence)
                .map_err(ForgeError::Vk)?;
            device
                .wait_for_fences(&[fence], true, u64::MAX)
                .map_err(ForgeError::Vk)?;
            device.destroy_fence(fence, None);
            device.free_command_buffers(vulkan.command_pool, &cbs);
        }

        self.tlas = tlas;
        self.tlas_buf = Some(tlas_buf);
        self.tlas_scratch = Some(scratch_buf);
        self.instance_buf = Some(inst_buf);
        Ok(())
    }

    /// # Safety
    /// All GPU work using this RT pipeline must have completed and `vulkan`
    /// must be the context used to create it.
    pub unsafe fn destroy(&mut self, vulkan: &VulkanContext) {
        let device = &vulkan.device;
        unsafe {
            if self.pipeline != vk::Pipeline::null() {
                device.destroy_pipeline(self.pipeline, None);
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                device.destroy_pipeline_layout(self.pipeline_layout, None);
            }
            if self.set_layout != vk::DescriptorSetLayout::null() {
                device.destroy_descriptor_set_layout(self.set_layout, None);
            }
            self.sbt_buffer.destroy(device);
            if let Some(mut b) = self.instance_buf.take() {
                b.destroy(device);
            }
            if let Some(mut b) = self.tlas_buf.take() {
                b.destroy(device);
            }
            if let Some(mut b) = self.tlas_scratch.take() {
                b.destroy(device);
            }
            if let (Some(accel), as_h) = (vulkan.rt_accel.as_ref(), self.tlas)
                && as_h != vk::AccelerationStructureKHR::null()
            {
                accel.destroy_acceleration_structure(as_h, None);
            }
        }
    }
}

fn align_up(x: u64, a: u64) -> u64 {
    (x + a - 1) & !(a - 1)
}

fn create_shader_module(device: &ash::Device, bytes: &[u8]) -> ForgeResult<vk::ShaderModule> {
    let words: ThinVec<u32> = bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let words_slice: &[u32] = &words;
    let info = vk::ShaderModuleCreateInfo::default().code(words_slice);
    unsafe {
        device
            .create_shader_module(&info, None)
            .map_err(ForgeError::Vk)
    }
}
