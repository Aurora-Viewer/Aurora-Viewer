//! Video memory as it is now: what the system lets this process use of the
//! GPU's own memory (VK_EXT_memory_budget), and what the process holds
//! there. The texture streamer keeps what it holds under that budget
//! (`scene/texture_budget.rs` in the viewer).
//!
//! Why not the size of the heap, read once at startup: other programs share
//! the card (a second viewer, a browser), and past the budget Windows moves
//! allocations to system memory. Render targets created there, after a
//! settings change for instance, made a frame cost 85 ms of GPU time instead
//! of 4. Firestorm takes its video memory figure from the same system
//! budget on Windows (the `Budget` of QueryVideoMemoryInfo, in
//! indra/llwindow/llwindowwin32.cpp), read once; here it is read again as
//! it changes.
//!
//! Two measures of what the process holds, and why the budget is kept on
//! the second: `usage` is what the driver counts, the memory blocks wgpu's
//! allocator took from it; `allocated` is the textures and buffers alive in
//! those blocks. A texture page that is freed leaves a hole in its block,
//! which goes back to the driver only once every resource in it is gone: on
//! the grid, 3.9 GB of pages freed brought `usage` down by 1 GB only. The
//! holes are not lost, the next pages and render targets fill them, but
//! counted against the textures they would keep the budget exceeded however
//! much the textures give back.

use ash::vk;

/// Bytes of the device-local heap, see the module: `budget` is what the
/// system lets this process use at the moment, `usage` what the driver
/// counts for it, `allocated` its live textures and buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoMemory {
    pub budget: u64,
    pub usage: u64,
    pub allocated: u64,
}

/// Reads [`VideoMemory`] from the adapter. A physical device query: it takes
/// no lock of the device and may run while the render thread draws.
pub struct VideoMemoryProbe {
    adapter: wgpu::Adapter,
    /// Index of the largest device-local heap, when the driver reports
    /// budgets.
    heap: Option<usize>,
}

impl VideoMemoryProbe {
    pub fn new(adapter: wgpu::Adapter) -> Self {
        let heap = Self::budget_heap(&adapter);
        match heap.and_then(|h| Self::query(&adapter, h)) {
            Some((budget, usage)) => log::info!(
                "GPU memory budget: {} MB for this process, {} MB used (VK_EXT_memory_budget)",
                budget >> 20,
                usage >> 20
            ),
            None => log::info!("GPU memory budget: not reported by the driver (texture memory follows its setting only)"),
        }
        VideoMemoryProbe { adapter, heap }
    }

    /// The figures of the moment; None when the driver does not report them.
    pub fn read(&self, device: &wgpu::Device) -> Option<VideoMemory> {
        let (budget, usage) = Self::query(&self.adapter, self.heap?)?;
        // (wgpu's counters: a few atomics, no lock; every buffer counts,
        // those in system memory too, a few hundred MB at most)
        let live = device.get_internal_counters().hal;
        let allocated = (live.texture_memory.read() + live.buffer_memory.read()).max(0) as u64;
        Some(VideoMemory { budget, usage, allocated })
    }

    /// The largest device-local heap (the card's own memory on a discrete
    /// GPU, the shared memory on an integrated one), when the extension and
    /// the query it extends are there.
    fn budget_heap(adapter: &wgpu::Adapter) -> Option<usize> {
        // SAFETY: the hal adapter is only borrowed for this call; the
        // queries below read properties and change no state.
        let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
        let shared = hal.shared_instance();
        // get_physical_device_memory_properties2 is core since Vulkan 1.1
        if shared.instance_api_version() < vk::API_VERSION_1_1
            || !hal.physical_device_capabilities().supports_extension(ash::ext::memory_budget::NAME)
        {
            return None;
        }
        // SAFETY: as above.
        let props = unsafe {
            shared
                .raw_instance()
                .get_physical_device_memory_properties(hal.raw_physical_device())
        };
        props
            .memory_heaps_as_slice()
            .iter()
            .enumerate()
            .filter(|(_, h)| h.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
            .max_by_key(|(_, h)| h.size)
            .map(|(i, _)| i)
    }

    /// (budget, usage) of a heap.
    fn query(adapter: &wgpu::Adapter, heap: usize) -> Option<(u64, u64)> {
        // SAFETY: the hal adapter is only borrowed for this call; a physical
        // device query, valid from any thread, with the budget structure the
        // extension checked in `budget_heap` defines.
        let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
        let mut budget = vk::PhysicalDeviceMemoryBudgetPropertiesEXT::default();
        let mut props = vk::PhysicalDeviceMemoryProperties2::default().push_next(&mut budget);
        // SAFETY: as above.
        unsafe {
            hal.shared_instance()
                .raw_instance()
                .get_physical_device_memory_properties2(hal.raw_physical_device(), &mut props);
        }
        let (budget, usage) = (*budget.heap_budget.get(heap)?, *budget.heap_usage.get(heap)?);
        // (a driver that fills nothing in reports no budget)
        (budget > 0).then_some((budget, usage))
    }
}
