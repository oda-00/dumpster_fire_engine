use ash::vk;
use crate::render::vulkan::VulkanContext;
use crate::render::window::FRAMES_IN_FLIGHT;
use std::mem;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct UiVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}

pub struct RingBuffer {
    buffers: [vk::Buffer; FRAMES_IN_FLIGHT],
    memories: [vk::DeviceMemory; FRAMES_IN_FLIGHT],
    sizes: [usize; FRAMES_IN_FLIGHT],
    mapped_ptrs: [*mut u8; FRAMES_IN_FLIGHT],
    current_frame: usize,
    capacity: usize,
}

impl RingBuffer {
    pub fn new(
        device: &ash::Device,
        mem_props: &vk::PhysicalDeviceMemoryProperties,
        initial_capacity: usize,
    ) -> Self {
        let mut buffers: [vk::Buffer; FRAMES_IN_FLIGHT] = [vk::Buffer::null(); FRAMES_IN_FLIGHT];
        let mut memories: [vk::DeviceMemory; FRAMES_IN_FLIGHT] =
            [vk::DeviceMemory::null(); FRAMES_IN_FLIGHT];
        let mut mapped_ptrs: [*mut u8; FRAMES_IN_FLIGHT] = [std::ptr::null_mut(); FRAMES_IN_FLIGHT];

        for i in 0..FRAMES_IN_FLIGHT {
            let (buf, mem, ptr) = Self::create_buffer(device, mem_props, initial_capacity);
            buffers[i] = buf;
            memories[i] = mem;
            mapped_ptrs[i] = ptr;
        }

        Self {
            buffers,
            memories,
            sizes: [0; FRAMES_IN_FLIGHT],
            mapped_ptrs,
            current_frame: 0,
            capacity: initial_capacity,
        }
    }

    fn create_buffer(
        device: &ash::Device,
        mem_props: &vk::PhysicalDeviceMemoryProperties,
        size: usize,
    ) -> (vk::Buffer, vk::DeviceMemory, *mut u8) {
        let buf_info = vk::BufferCreateInfo::default()
            .size(size as u64)
            .usage(vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::INDEX_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buf = unsafe { device.create_buffer(&buf_info, None) }.unwrap();
        let mem_req = unsafe { device.get_buffer_memory_requirements(buf) };

        let mem_type = mem_props
            .memory_types
            .iter()
            .enumerate()
            .find(|(i, t)| {
                (mem_req.memory_type_bits & (1 << i)) != 0
                    && t.property_flags.contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
            })
            .unwrap()
            .0;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_req.size)
            .memory_type_index(mem_type as u32);

        let mem = unsafe { device.allocate_memory(&alloc_info, None) }.unwrap();
        unsafe { device.bind_buffer_memory(buf, mem, 0) }.unwrap();

        let ptr = unsafe {
            device.map_memory(mem, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty()).unwrap() as *mut u8
        };

        (buf, mem, ptr)
    }

    pub fn upload(&mut self, device: &ash::Device, data: &[u8]) {
        let idx = self.current_frame;
        if data.len() > self.capacity {
            self.resize(device, data.len() * 2);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.mapped_ptrs[idx],
                data.len(),
            );
        }
        self.sizes[idx] = data.len();
    }

    pub fn end_frame(&mut self, device: &ash::Device) {
        let idx = self.current_frame;
        unsafe {
            device.flush_mapped_memory_ranges(&[vk::MappedMemoryRange::default()
                .memory(self.memories[idx])
                .offset(0)
                .size(vk::WHOLE_SIZE)])
            .unwrap();
        }
        self.current_frame = (self.current_frame + 1) % FRAMES_IN_FLIGHT;
    }

    pub fn buffer(&self) -> vk::Buffer {
        self.buffers[self.current_frame]
    }

    pub fn size(&self) -> u32 {
        self.sizes[self.current_frame] as u32
    }

    fn resize(&mut self, device: &ash::Device, new_capacity: usize) {
        for i in 0..FRAMES_IN_FLIGHT {
            unsafe {
                device.destroy_buffer(self.buffers[i], None);
                device.free_memory(self.memories[i], None);
            }
        }

        let mem_props = unsafe {
            let instance = std::ptr::null::<ash::Instance>();
            (*instance).get_physical_device_memory_properties(vk::PhysicalDevice::null())
        };

        for i in 0..FRAMES_IN_FLIGHT {
            let (buf, mem, ptr) = Self::create_buffer(device, &mem_props, new_capacity);
            self.buffers[i] = buf;
            self.memories[i] = mem;
            self.mapped_ptrs[i] = ptr;
        }
        self.capacity = new_capacity;
    }
}

impl Drop for RingBuffer {
    fn drop(&mut self) {
        // Buffers and memory will be cleaned up by the Vulkan device
    }
}
