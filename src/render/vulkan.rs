use std::ffi::CStr;
use std::sync::Arc;

use ash::vk;
use winit::raw_window_handle::RawDisplayHandle;

use crate::forge_master::{ForgeError, ForgeResult};

// Minimal compute-only Vulkan bootstrap. Owns the entry, instance, logical
// device, command pool, and queue. Callers hand the device/queue/pool/memprops
// into ForgeMaster::new; we hold the same fields here so Drop can tear them
// down in the right order after the consumer (Renderer + ForgeMaster) goes
// out of scope.
//
// Drop order at the call site:
//   1. Renderer drops -> walks windows -> Frames destroy their Ingots' Vulkan
//      handles using a cloned ash::Device.
//   2. ForgeMaster (inside Renderer) drops -> fence + descriptor pool destroyed.
//   3. VulkanContext drops -> command pool, device, instance destroyed.
//
// ash::Device and ash::Instance are clone-by-handle wrappers; the actual
// vkDestroyDevice / vkDestroyInstance only fires from this struct's Drop.
pub struct VulkanContext {
    pub entry: ash::Entry,
    pub instance: ash::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub device: ash::Device,
    pub queue: vk::Queue,
    pub queue_family_index: u32,
    pub command_pool: vk::CommandPool,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
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
    ///
    /// Use this when the consumer is going to create a `VkSurfaceKHR` and a
    /// swapchain (e.g. the hello_triangle binary). `display_handle` comes
    /// from `winit::window::Window::display_handle()` — we only need it for
    /// `ash_window::enumerate_required_extensions`, so it's passed by value
    /// and not stored.
    ///
    /// The selected queue family supports both GRAPHICS and COMPUTE so the
    /// same `ForgeMaster` setup keeps working — the demo's compute path
    /// stays alive even when we're drawing pixels.
    pub fn with_surface(display_handle: RawDisplayHandle) -> ForgeResult<Self> {
        Self::build("dumpster_fire_engine", Some(display_handle), true)
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

        // Surface-creation extensions come from ash_window when we know the
        // platform. Without a display handle this is empty and we get a
        // compute-only instance (matches the original `new()` behavior).
        let instance_extensions: Vec<*const i8> = match display_handle {
            Some(dh) => ash_window::enumerate_required_extensions(dh)
                .map_err(ForgeError::Vk)?
                .to_vec(),
            None => Vec::new(),
        };

        let instance_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&instance_extensions);
        let instance = unsafe { entry.create_instance(&instance_info, None)? };

        // Pick the strongest physical device that exposes the needed queue.
        //
        // Ranking (higher = better):
        //   4  DISCRETE_GPU   — dedicated card, almost always the fastest
        //   3  INTEGRATED_GPU — CPU-embedded GPU
        //   2  VIRTUAL_GPU    — inside a VM hypervisor
        //   1  CPU            — software rasterizer fallback
        //   0  OTHER / unknown
        //
        // Within the same tier, the device with the most DEVICE_LOCAL VRAM
        // wins (picks the higher-VRAM card in a dual-GPU desktop).
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
        let mut best_score: (u8, u64) = (0, 0); // (type_tier, vram_bytes)

        for pd in physicals {
            // Must have the required queue family.
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

            // Sum all DEVICE_LOCAL heaps as a VRAM estimate.
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

        let priorities = [1.0f32];
        let queue_create = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priorities)];

        // VK_KHR_swapchain is required for any surface present, even though
        // it's a device extension (instance extensions came from
        // enumerate_required_extensions above).
        let swapchain_ext = ash::khr::swapchain::NAME.as_ptr();
        let device_extensions: Vec<*const i8> = if want_graphics {
            vec![swapchain_ext]
        } else {
            Vec::new()
        };
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create)
            .enabled_extension_names(&device_extensions);

        let device = match unsafe { instance.create_device(physical_device, &device_info, None) } {
            Ok(d) => d,
            Err(e) => {
                unsafe { instance.destroy_instance(None) };
                return Err(ForgeError::Vk(e));
            }
        };

        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };

        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let command_pool = match unsafe { device.create_command_pool(&pool_info, None) } {
            Ok(p) => p,
            Err(e) => {
                unsafe {
                    device.destroy_device(None);
                    instance.destroy_instance(None);
                }
                return Err(ForgeError::Vk(e));
            }
        };

        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };

        Ok(Self {
            entry,
            instance,
            physical_device,
            device,
            queue,
            queue_family_index,
            command_pool,
            memory_properties,
            device_name,
        })
    }
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();
            if self.command_pool != vk::CommandPool::null() {
                self.device.destroy_command_pool(self.command_pool, None);
                self.command_pool = vk::CommandPool::null();
            }
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
