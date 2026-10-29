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
//! Two measures of what the process holds: `usage` is what the driver
//! counts, the memory blocks wgpu's allocator took from it and the texture
//! pages' memory of their own (dedicated.rs); `allocated` is the textures
//! and buffers alive in all that. The difference is the free room of the
//! blocks, which goes back to the driver only once every resource in a
//! block is gone. Texture pages used to be carved out of those blocks too:
//! on the grid, 3.9 GB of pages freed brought `usage` down by 1 GB only,
//! and the viewer could stay past the budget with its textures lowered.
//! They now give their memory back as they go, and the texture budget
//! counts `usage` (scene/texture_budget.rs in the viewer).

#[cfg(not(target_os = "macos"))]
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

/// AURORA_VRAM_BUDGET_MB: the driver's budget taken as this many megabytes
/// (a smaller card on a large one: the texture budget, the Performance
/// window's warning).
fn forced_budget() -> Option<u64> {
    static FORCED: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| {
        let mb: u64 = std::env::var("AURORA_VRAM_BUDGET_MB").ok()?.trim().parse().ok()?;
        Some(mb.clamp(256, 1 << 20) << 20)
    })
}

/// Reads [`VideoMemory`] from the adapter. A physical device query: it takes
/// no lock of the device and may run while the render thread draws.
pub struct VideoMemoryProbe {
    adapter: wgpu::Adapter,
    /// Index of the largest device-local heap, when the driver reports
    /// budgets.
    heap: Option<usize>,
}

/// Where the figures come from, for the log.
#[cfg(not(target_os = "macos"))]
const SOURCE: &str = "VK_EXT_memory_budget";
#[cfg(target_os = "macos")]
const SOURCE: &str = "MTLDevice working set";

impl VideoMemoryProbe {
    pub fn new(adapter: wgpu::Adapter) -> Self {
        let heap = Self::budget_heap(&adapter);
        match heap.and_then(|h| Self::query(&adapter, h)) {
            Some((budget, usage)) => log::info!(
                "GPU memory budget: {} MB for this process, {} MB used ({SOURCE})",
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
        let budget = forced_budget().unwrap_or(budget);
        // (wgpu's counters: a few atomics, no lock; every buffer counts,
        // those in system memory too, a few hundred MB at most)
        let live = device.get_internal_counters().hal;
        // (the texture pages in memory of their own are not in them)
        let allocated = (live.texture_memory.read() + live.buffer_memory.read()).max(0) as u64 + crate::dedicated::bytes();
        Some(VideoMemory { budget, usage, allocated })
    }

    /// The largest device-local heap (the card's own memory on a discrete
    /// GPU, the shared memory on an integrated one), when the extension and
    /// the query it extends are there.
    #[cfg(not(target_os = "macos"))]
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
    #[cfg(not(target_os = "macos"))]
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

    /// Metal has one pool, the unified memory on Apple Silicon: heap 0.
    #[cfg(target_os = "macos")]
    fn budget_heap(adapter: &wgpu::Adapter) -> Option<usize> {
        Self::query(adapter, 0).map(|_| 0)
    }

    /// (budget, usage) on Metal: the working set the system recommends for
    /// this device before performance suffers, and what this process's
    /// device has allocated. The Legacy Viewer reads a fixed figure once
    /// (`kCGLRPVideoMemoryMegabytes`, indra/llwindow/llopenglview-objc.mm);
    /// both are read again here, as on Vulkan.
    #[cfg(target_os = "macos")]
    fn query(adapter: &wgpu::Adapter, _heap: usize) -> Option<(u64, u64)> {
        use objc2_metal::MTLDevice;
        // SAFETY: the hal adapter is only borrowed for this call; MTLDevice
        // is thread-safe and these two properties change no state.
        let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Metal>() }?;
        let device = hal.raw_device();
        let budget = device.recommendedMaxWorkingSetSize();
        (budget > 0).then_some((budget, device.currentAllocatedSize() as u64))
    }
}
