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
//!
//! The budget given out is the lowest the driver reported over the last
//! minute ([`BudgetFloor`]): a drop counts at once, a rise once it has held
//! for [`BUDGET_WINDOW`]. The driver's figure follows the viewer's own
//! usage: on the grid (RTX 5070, 12 GB, a busy region) it read 10.3 GB
//! while the textures were loaded and 11.2 GB as soon as they were lowered
//! (8.4 and 11.2 GB with a second viewer open), so the texture target swung
//! between 2.7 and 4.9 GB, the pressure went 0 -> 1 -> 0 every few seconds
//! to minutes, and thousands of textures were lowered and loaded again each
//! time (10,219 in 40 s, #1557).
//!
//! Past the budget the viewer has only its textures to give back: two
//! viewers on that card, 6.6 and 5.9 GB, overfilled it, and Windows moved
//! memory to system RAM (GPU frames of 1 s, then a device lost).

#[cfg(not(target_os = "macos"))]
use ash::vk;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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

/// How long a higher budget must hold before it is given out (module).
pub const BUDGET_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// The lowest budget of the last [`BUDGET_WINDOW`]: the readings that could
/// still be the lowest, oldest first, each lower than the next (a sliding
/// minimum, constant time per reading).
#[derive(Debug, Default)]
pub struct BudgetFloor {
    readings: std::collections::VecDeque<(std::time::Instant, u64)>,
}

impl BudgetFloor {
    /// Take the driver's figure of `now`; the budget to give out.
    pub fn update(&mut self, raw: u64, now: std::time::Instant) -> u64 {
        while self.readings.back().is_some_and(|&(_, b)| b >= raw) {
            self.readings.pop_back();
        }
        self.readings.push_back((now, raw));
        while self
            .readings
            .front()
            .is_some_and(|&(t, _)| now.saturating_duration_since(t) > BUDGET_WINDOW)
        {
            self.readings.pop_front();
        }
        // (the reading just pushed is always there)
        self.readings.front().map_or(raw, |&(_, b)| b)
    }
}

/// The driver can block even a physical-device memory query (#1992: the
/// main thread in NVIDIA's driver, the render thread idle). Keep that call
/// off every frame/UI path; readers only copy the last complete sample.
pub struct VideoMemoryProbe {
    sampler: BudgetSampler,
    /// For the card's identity, read once on demand (`card`).
    #[cfg(not(target_os = "macos"))]
    adapter: wgpu::Adapter,
}

const PROBE_INTERVAL: Duration = Duration::from_millis(250);

/// The query runs without the cache lock. A stuck driver leaves the last
/// budget available and cannot hold up a frame or the viewer's shutdown.
struct BudgetSampler {
    latest: Arc<parking_lot::Mutex<Option<(u64, u64)>>>,
    stopped: Arc<AtomicBool>,
}

impl BudgetSampler {
    fn new(mut query: impl FnMut() -> Option<(u64, u64)> + Send + 'static, interval: Duration) -> Self {
        let latest = Arc::new(parking_lot::Mutex::new(None));
        let stopped = Arc::new(AtomicBool::new(false));
        let (output, stop) = (latest.clone(), stopped.clone());
        let spawned = std::thread::Builder::new().name("aurora-vram".into()).spawn(move || {
            let mut floor = BudgetFloor::default();
            let mut logged = None::<u64>;
            while !stop.load(Ordering::Acquire) {
                if let Some((raw, usage)) = query() {
                    let budget = floor.update(raw, std::time::Instant::now());
                    *output.lock() = Some((budget, usage));
                    if logged.is_none_or(|last| budget.abs_diff(last) >= BUDGET_LOG_STEP) {
                        logged = Some(budget);
                        log::info!(
                            "GPU memory budget: {} MB given out, the lowest of the last {} s ({} MB reported now, {} MB used, {SOURCE})",
                            budget >> 20,
                            BUDGET_WINDOW.as_secs(),
                            raw >> 20,
                            usage >> 20
                        );
                    }
                }
                std::thread::sleep(interval);
            }
        });
        if let Err(e) = spawned {
            log::warn!("GPU memory budget: sampling thread not started ({e}); texture memory follows its setting only");
        }
        Self { latest, stopped }
    }

    fn read(&self) -> Option<(u64, u64)> {
        *self.latest.lock()
    }
}

impl Drop for BudgetSampler {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

/// A change of the budget given out worth a log line.
const BUDGET_LOG_STEP: u64 = 256 << 20;

/// Where the figures come from, for the log.
#[cfg(not(target_os = "macos"))]
const SOURCE: &str = "VK_EXT_memory_budget";
#[cfg(target_os = "macos")]
const SOURCE: &str = "MTLDevice working set";

impl VideoMemoryProbe {
    pub fn new(adapter: wgpu::Adapter) -> Self {
        #[cfg(not(target_os = "macos"))]
        let card = adapter.clone();
        let heap = std::sync::OnceLock::new();
        let sampler = BudgetSampler::new(
            move || Self::query(&adapter, (*heap.get_or_init(|| Self::budget_heap(&adapter)))?),
            PROBE_INTERVAL,
        );
        VideoMemoryProbe {
            sampler,
            #[cfg(not(target_os = "macos"))]
            adapter: card,
        }
    }

    /// The most recent complete sample, at most one driver query per
    /// quarter second. A blocked query keeps the previous conservative
    /// budget; None until the first reading or when none is available.
    pub fn read(&self, device: &wgpu::Device) -> Option<VideoMemory> {
        let (budget, usage) = self.sampler.read()?;
        let budget = forced_budget().unwrap_or(budget);
        // (wgpu's counters: a few atomics, no lock; every buffer counts,
        // those in system memory too, a few hundred MB at most)
        let live = device.get_internal_counters().hal;
        // (the texture pages in memory of their own are not in them)
        let allocated = (live.texture_memory.read() + live.buffer_memory.read()).max(0) as u64 + crate::dedicated::bytes();
        Some(VideoMemory { budget, usage, allocated })
    }

    /// The card's identity for the system's adapter-wide counters (the LUID
    /// Windows names its adapters by, `high << 32 | low`), the size of its
    /// largest device-local heap, and whether that heap is the card's own
    /// memory (a discrete GPU: on an integrated one it is system RAM, often
    /// full by design, and no size to measure the card's load against); None
    /// without the Vulkan 1.1 query.
    #[cfg(not(target_os = "macos"))]
    pub fn card(&self) -> Option<(u64, u64, bool)> {
        // SAFETY: the hal adapter is only borrowed for this call; the queries
        // read properties and change no state.
        let hal = unsafe { self.adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
        let shared = hal.shared_instance();
        if shared.instance_api_version() < vk::API_VERSION_1_1 {
            return None;
        }
        let mut id = vk::PhysicalDeviceIDProperties::default();
        let mut props = vk::PhysicalDeviceProperties2::default().push_next(&mut id);
        // SAFETY: as above.
        unsafe {
            shared
                .raw_instance()
                .get_physical_device_properties2(hal.raw_physical_device(), &mut props);
        }
        let dedicated = props.properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU;
        if id.device_luid_valid != vk::TRUE {
            return None;
        }
        // SAFETY: as above.
        let memory = unsafe {
            shared
                .raw_instance()
                .get_physical_device_memory_properties(hal.raw_physical_device())
        };
        // (the heap budget_heap picks, without its memory_budget condition:
        // only the size is read here, never the blocking budget query)
        let total = memory
            .memory_heaps_as_slice()
            .iter()
            .filter(|h| h.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
            .map(|h| h.size)
            .max()?;
        Some((u64::from_le_bytes(id.device_luid), total, dedicated))
    }

    /// Metal has no adapter-wide counters to name.
    #[cfg(target_os = "macos")]
    pub fn card(&self) -> Option<(u64, u64, bool)> {
        None
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const MB: u64 = 1 << 20;

    #[test]
    fn a_blocked_driver_query_cannot_block_cached_reads_or_shutdown() {
        for initial in [None, Some((8_000 * MB, 4_000 * MB))] {
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let mut first = true;
            let sampler = BudgetSampler::new(
                move || {
                    if std::mem::take(&mut first) {
                        return initial;
                    }
                    let _ = entered_tx.send(());
                    let _ = release_rx.recv();
                    Some((7_000 * MB, 3_000 * MB))
                },
                Duration::from_millis(1),
            );
            entered_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("query entered after publishing the first sample");
            let start = Instant::now();
            assert_eq!(sampler.read(), initial);
            drop(sampler);
            assert!(
                start.elapsed() < Duration::from_millis(250),
                "cache reads and shutdown waited for the blocked query"
            );
            // The query is still blocked until after both operations.
            release_tx.send(()).expect("release the sampling thread");
        }
    }

    #[test]
    fn a_drop_counts_at_once_and_a_rise_once_it_has_held_a_minute() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut f = BudgetFloor::default();
        assert_eq!(f.update(11_174 * MB, at(0)), 11_174 * MB);
        // a second viewer comes: at once
        assert_eq!(f.update(8_385 * MB, at(1)), 8_385 * MB);
        assert_eq!(f.update(3_835 * MB, at(2)), 3_835 * MB);
        // it leaves: the low figure holds for the window
        assert_eq!(f.update(11_174 * MB, at(3)), 3_835 * MB);
        assert_eq!(f.update(11_174 * MB, at(62)), 3_835 * MB);
        // a minute after the last low reading, the higher one
        assert_eq!(f.update(11_174 * MB, at(63)), 11_174 * MB);
    }

    #[test]
    fn a_budget_swinging_every_few_seconds_gives_out_its_low_side() {
        // the grid's figures with a second viewer open: 8.4 GB with the
        // textures loaded, 11.2 GB once they are lowered, every 5 s
        let t0 = Instant::now();
        let mut f = BudgetFloor::default();
        let mut given = Vec::new();
        for i in 0..120u64 {
            let raw = if (i / 5) % 2 == 0 { 8_385 * MB } else { 11_174 * MB };
            given.push(f.update(raw, t0 + Duration::from_secs(i)));
        }
        assert!(given.iter().all(|&b| b == 8_385 * MB), "{given:?}");
    }

    #[test]
    fn the_floor_follows_the_lowest_reading_still_in_the_window() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut f = BudgetFloor::default();
        f.update(9_000 * MB, at(0));
        f.update(7_000 * MB, at(10));
        f.update(8_000 * MB, at(20));
        assert_eq!(f.update(10_000 * MB, at(30)), 7_000 * MB);
        // 7 GB out of the window: the 8 GB of t = 20 is the lowest left
        assert_eq!(f.update(10_000 * MB, at(71)), 8_000 * MB);
        assert_eq!(f.update(10_000 * MB, at(81)), 10_000 * MB);
        // the readings kept stay few: each lower than the next
        assert!(f.readings.len() <= 2, "{:?}", f.readings);
    }
}
