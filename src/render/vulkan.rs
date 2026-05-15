use std::ffi::CStr;
use std::sync::Arc;

use ash::vk;
use winit::raw_window_handle::RawDisplayHandle;

use crate::forge_master::{ForgeError, ForgeResult, MeshUploadCtx};

// Minimal compute-or-graphics Vulkan bootstrap. Owns the entry, instance,
// logical device, command pools, and queues. Drop tears down in the right
// order; consumers (Renderer + ForgeMaster) must be dropped first.
//
// Best-practice features:
//   - Discrete-GPU preference with VRAM tiebreaker.
//   - Dedicated transfer queue (TRANSFER-only family) when the device has
//     one; otherwise reuses the graphics queue/family. Exposed via
//     `mesh_upload_ctx()` for non-blocking-on-graphics asset uploads.
//   - `depth_format` chosen at boot from the best available DEPTH format.
//   - Validation layers enabled in `#[cfg(debug_assertions)]` if present.
pub struct VulkanContext {
    pub entry: ash::Entry,
    pub instance: ash::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub device: ash::Device,

    // Graphics / compute queue.
    pub queue: vk::Queue,
    pub queue_family_index: u32,
    pub command_pool: vk::CommandPool,

    // Dedicated transfer queue. When the device has no transfer-only family
    // these alias the graphics queue/family/pool — `mesh_upload_ctx()` still
    // works, the upload path just collapses to a single-queue submit.
    pub transfer_queue: vk::Queue,
    pub transfer_queue_family: u32,
    pub transfer_command_pool: vk::CommandPool,

    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub depth_format: vk::Format,
    pub device_name: Arc<str>,
}

impl VulkanContext {
    pub fn new() -> ForgeResult<Self> {
        Self::new_with_app_name("dumpster_fire_engine")
    }

    pub fn new_with_app_name(app_name: &str) -> ForgeResult<Self> {
        Self::build(app_name, None, false)
    }

    /// Bootstrap a Vulkan context with WSI extensions for a winit window.
    pub fn with_surface(display_handle: RawDisplayHandle) -> ForgeResult<Self> {
        Self::build("dumpster_fire_engine", Some(display_handle), true)
    }

    /// Bundle the device handles into a `MeshUploadCtx` for `GpuMesh::upload`.
    pub fn mesh_upload_ctx(&self) -> MeshUploadCtx<'_> {
        MeshUploadCtx {
            device:                &self.device,
            memory_properties:     &self.memory_properties,
            transfer_queue:        self.transfer_queue,
            transfer_queue_family: self.transfer_queue_family,
            transfer_command_pool: self.transfer_command_pool,
            graphics_queue:        self.queue,
            graphics_queue_family: self.queue_family_index,
            graphics_command_pool: self.command_pool,
        }
    }

    fn build(
        app_name: &str,
        display_handle: Option<RawDisplayHandle>,
        want_graphics: bool,
    ) -> ForgeResult<Self> {
        let entry = unsafe { ash::Entry::load()? };

        let app_name_c = std::ffi::CString::new(app_name)
            .unwrap_or_else(|_| std::ffi::CString::new("dumpster_fire_engine").unwrap());
        let engine_name_c = std::ffi::CString::new("dumpster_fire_engine").unwrap();

        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name_c)
            .application_version(0)
            .engine_name(&engine_name_c)
            .engine_version(0)
            .api_version(vk::API_VERSION_1_2);

        let instance_extensions: Vec<*const i8> = match display_handle {
            Some(dh) => ash_window::enumerate_required_extensions(dh)
                .map_err(ForgeError::Vk)?
                .to_vec(),
            None => Vec::new(),
        };

        // Validation layer setup (debug only). Held outside the cfg block so
        // its lifetime spans `create_instance`.
        let validation_layer_name =
            std::ffi::CString::new("VK_LAYER_KHRONOS_validation").unwrap();
        #[allow(unused_mut)]
        let mut enabled_layers: Vec<*const i8> = Vec::new();
        #[cfg(debug_assertions)]
        {
            let available =
                unsafe { entry.enumerate_instance_layer_properties() }.unwrap_or_default();
            let present = available.iter().any(|p| {
                let name = unsafe { CStr::from_ptr(p.layer_name.as_ptr()) };
                name == validation_layer_name.as_c_str()
            });
            if present {
                enabled_layers.push(validation_layer_name.as_ptr());
                println!("vulkan: validation layer enabled");
            } else {
                println!("vulkan: validation layer requested but not installed; continuing without");
            }
        }
        let _ = &validation_layer_name; // hold lifetime in release too

        let instance_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_layer_names(&enabled_layers)
            .enabled_extension_names(&instance_extensions);
        let instance = unsafe { entry.create_instance(&instance_info, None)? };

        // Pick the strongest physical device that exposes the needed queue.
        let physicals = unsafe { instance.enumerate_physical_devices()? };
        if physicals.is_empty() {
            unsafe { instance.destroy_instance(None) };
            return Err(ForgeError::NoPhysicalDevice);
        }
        let required_flags = if want_graphics {
            vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE
        } else {
            vk::QueueFlags::COMPUTE
        };

        let mut chosen: Option<(vk::PhysicalDevice, u32)> = None;
        let mut best_score: (u8, u64) = (0, 0);

        for pd in physicals {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(pd) };
            let Some((qfi, _)) = families
                .iter()
                .enumerate()
                .find(|(_, f)| f.queue_flags.contains(required_flags))
            else {
                continue;
            };

            let props = unsafe { instance.get_physical_device_properties(pd) };
            let type_tier: u8 = match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU   => 4,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 3,
                vk::PhysicalDeviceType::VIRTUAL_GPU    => 2,
                vk::PhysicalDeviceType::CPU            => 1,
                _                                      => 0,
            };

            let mem = unsafe { instance.get_physical_device_memory_properties(pd) };
            let vram: u64 = (0..mem.memory_heap_count as usize)
                .filter(|&i| mem.memory_heaps[i]
                    .flags
                    .contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
                .map(|i| mem.memory_heaps[i].size)
                .sum();

            let score = (type_tier, vram);
            if score > best_score {
                best_score = score;
                chosen = Some((pd, qfi as u32));
            }
        }

        let (physical_device, queue_family_index) = match chosen {
            Some(c) => c,
            None => {
                unsafe { instance.destroy_instance(None) };
                return Err(ForgeError::NoCompatibleQueue);
            }
        };

        let selected_props =
            unsafe { instance.get_physical_device_properties(physical_device) };
        let device_name: Arc<str> =
            unsafe { CStr::from_ptr(selected_props.device_name.as_ptr()) }
                .to_string_lossy()
                .as_ref()
                .into();
        println!(
            "GPU: {} ({:.0} MB DEVICE_LOCAL, score tier {})",
            device_name,
            best_score.1 as f64 / 1_048_576.0,
            best_score.0,
        );

        // Find a dedicated transfer queue family if one exists: TRANSFER but
        // NOT GRAPHICS or COMPUTE. On discrete GPUs this is the DMA engine.
        let families =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
        let mut transfer_family: u32 = queue_family_index;
        for (i, f) in families.iter().enumerate() {
            let supports_transfer = f.queue_flags.contains(vk::QueueFlags::TRANSFER)
                // Some devices don't advertise TRANSFER explicitly when GRAPHICS
                // is set; GRAPHICS implies TRANSFER per spec. We want only the
                // dedicated-DMA case here.
                && !f.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                && !f.queue_flags.contains(vk::QueueFlags::COMPUTE);
            if supports_transfer {
                transfer_family = i as u32;
                break;
            }
        }
        let dedicated_transfer = transfer_family != queue_family_index;
        if dedicated_transfer {
            println!("vulkan: dedicated transfer queue family {transfer_family} (DMA engine)");
        }

        let priorities = [1.0f32];
        let mut queue_create_infos: Vec<vk::DeviceQueueCreateInfo> = Vec::new();
        queue_create_infos.push(
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(queue_family_index)
                .queue_priorities(&priorities),
        );
        if dedicated_transfer {
            queue_create_infos.push(
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(transfer_family)
                    .queue_priorities(&priorities),
            );
        }

        let swapchain_ext = ash::khr::swapchain::NAME.as_ptr();
        let device_extensions: Vec<*const i8> = if want_graphics {
            vec![swapchain_ext]
        } else {
            Vec::new()
        };
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extensions);

        let device = match unsafe { instance.create_device(physical_device, &device_info, None) } {
            Ok(d) => d,
            Err(e) => {
                unsafe { instance.destroy_instance(None) };
                return Err(ForgeError::Vk(e));
            }
        };

        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
        let transfer_queue = if dedicated_transfer {
            unsafe { device.get_device_queue(transfer_family, 0) }
        } else {
            queue
        };

        let make_pool = |fam: u32| -> Result<vk::CommandPool, vk::Result> {
            let info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(fam)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
            unsafe { device.create_command_pool(&info, None) }
        };

        let command_pool = match make_pool(queue_family_index) {
            Ok(p) => p,
            Err(e) => {
                unsafe {
                    device.destroy_device(None);
                    instance.destroy_instance(None);
                }
                return Err(ForgeError::Vk(e));
            }
        };
        let transfer_command_pool = if dedicated_transfer {
            match make_pool(transfer_family) {
                Ok(p) => p,
                Err(e) => {
                    unsafe {
                        device.destroy_command_pool(command_pool, None);
                        device.destroy_device(None);
                        instance.destroy_instance(None);
                    }
                    return Err(ForgeError::Vk(e));
                }
            }
        } else {
            command_pool
        };

        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };

        // Pick a depth format the device actually supports. Prefer D32_SFLOAT
        // (highest precision, no stencil); fall back through the usual list.
        let depth_format = find_supported_format(
            &instance,
            physical_device,
            &[
                vk::Format::D32_SFLOAT,
                vk::Format::D32_SFLOAT_S8_UINT,
                vk::Format::D24_UNORM_S8_UINT,
                vk::Format::D16_UNORM,
            ],
            vk::ImageTiling::OPTIMAL,
            vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT,
        )
        .unwrap_or(vk::Format::D32_SFLOAT);

        Ok(Self {
            entry,
            instance,
            physical_device,
            device,
            queue,
            queue_family_index,
            command_pool,
            transfer_queue,
            transfer_queue_family: transfer_family,
            transfer_command_pool,
            memory_properties,
            depth_format,
            device_name,
        })
    }
}

fn find_supported_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    candidates: &[vk::Format],
    tiling: vk::ImageTiling,
    features: vk::FormatFeatureFlags,
) -> Option<vk::Format> {
    for &f in candidates {
        let props = unsafe { instance.get_physical_device_format_properties(physical_device, f) };
        let supported = match tiling {
            vk::ImageTiling::LINEAR  => props.linear_tiling_features.contains(features),
            vk::ImageTiling::OPTIMAL => props.optimal_tiling_features.contains(features),
            _ => false,
        };
        if supported {
            return Some(f);
        }
    }
    None
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();
            // Only destroy the transfer pool if it's a distinct handle.
            if self.transfer_command_pool != self.command_pool
                && self.transfer_command_pool != vk::CommandPool::null()
            {
                self.device.destroy_command_pool(self.transfer_command_pool, None);
                self.transfer_command_pool = vk::CommandPool::null();
            }
            if self.command_pool != vk::CommandPool::null() {
                self.device.destroy_command_pool(self.command_pool, None);
                self.command_pool = vk::CommandPool::null();
            }
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
