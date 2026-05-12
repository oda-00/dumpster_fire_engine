use std::ffi::CStr;
use std::sync::Arc;

use ash::vk;

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

        let instance_info = vk::InstanceCreateInfo::default().application_info(&app_info);
        let instance = unsafe { entry.create_instance(&instance_info, None)? };

        // Pick the first physical device that exposes a COMPUTE queue family.
        let physicals = unsafe { instance.enumerate_physical_devices()? };
        if physicals.is_empty() {
            unsafe { instance.destroy_instance(None) };
            return Err(ForgeError::NoPhysicalDevice);
        }
        let mut chosen: Option<(vk::PhysicalDevice, u32)> = None;
        for pd in physicals {
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(pd) };
            if let Some((idx, _)) = families
                .iter()
                .enumerate()
                .find(|(_, f)| f.queue_flags.contains(vk::QueueFlags::COMPUTE))
            {
                chosen = Some((pd, idx as u32));
                break;
            }
        }
        let (physical_device, queue_family_index) = match chosen {
            Some(c) => c,
            None => {
                unsafe { instance.destroy_instance(None) };
                return Err(ForgeError::NoCompatibleQueue);
            }
        };

        let props = unsafe { instance.get_physical_device_properties(physical_device) };
        let device_name: Arc<str> = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }
            .to_string_lossy()
            .as_ref()
            .into();

        let priorities = [1.0f32];
        let queue_create = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priorities)];
        let device_info = vk::DeviceCreateInfo::default().queue_create_infos(&queue_create);

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
