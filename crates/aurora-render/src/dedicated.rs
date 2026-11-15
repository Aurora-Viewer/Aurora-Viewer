//! Texture pages in dedicated allocations: one `VkDeviceMemory` each,
//! given back to the driver the moment the page is destroyed. Prefer
//! device-local memory; on integrated GPUs only, retry a full local heap in
//! a separate host-visible system heap.
//!
//! wgpu's allocator carves its textures out of blocks of 128 to 256 MB and
//! hands a block back to the driver only once nothing is left in it. Pages
//! come and go all the time (streaming, textures lowered under memory
//! pressure, teleports), and those that stay end up spread over blocks they
//! leave mostly empty: in the streaming demo, once the textures had been
//! lowered from 717 MB of pages to 115 MB, the driver still counted 1,792 MB
//! for the viewer, its peak, the 115 MB holding five blocks of 256 MB. A
//! gap like that can take the viewer past the budget the driver grants it,
//! where Windows moves video memory out to system memory.
//!
//! The image and its memory are made here with ash and handed to wgpu
//! (`create_texture_from_hal`), which uses the page like any other texture;
//! its drop callback destroys both once wgpu is done with the page (after
//! the last submission that used it). Pages are made by the page thread
//! (textures.rs): the driver's allocation costs the frame nothing there, and
//! takes no lock of wgpu's allocator.
//!
//! Not used, wgpu's allocator instead: on a backend other than Vulkan, for
//! a format or a usage this module does not know, and past half of the
//! allocations the device allows (`maxMemoryAllocationCount`, often 4,096,
//! shared with wgpu's blocks).

use ash::vk;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Pages made here and alive, and the bytes of video memory they hold.
static LIVE: AtomicU32 = AtomicU32::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

/// Bytes of video memory held by the textures made here (wgpu's counters
/// do not see them).
pub fn bytes() -> u64 {
    BYTES.load(Ordering::Relaxed)
}

/// A texture in memory of its own. None when this path does not apply (the
/// caller creates the texture through wgpu), Err when the driver refused it.
pub fn texture(device: &wgpu::Device, desc: &wgpu::TextureDescriptor, integrated: bool) -> Option<Result<wgpu::Texture, String>> {
    let format = vk_format(desc.format)?;
    let (usage, hal_usage) = usages(desc.usage)?;
    if desc.dimension != wgpu::TextureDimension::D2 || desc.sample_count != 1 || !desc.view_formats.is_empty() {
        return None;
    }
    // what wgpu would refuse goes through wgpu, which says why (Vulkan is
    // called directly here, without wgpu's validation)
    let allowed = device.limits();
    let size = desc.size;
    let longest = size.width.max(size.height);
    if size.width == 0
        || size.height == 0
        || longest > allowed.max_texture_dimension_2d
        || !(1..=allowed.max_texture_array_layers).contains(&size.depth_or_array_layers)
        || !(1..=32 - longest.leading_zeros()).contains(&desc.mip_level_count)
        || (format == vk::Format::BC7_UNORM_BLOCK
            && (!device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
                || !size.width.is_multiple_of(4)
                || !size.height.is_multiple_of(4)))
    {
        return None;
    }
    let hal_desc = wgpu::hal::TextureDescriptor {
        label: desc.label,
        size: desc.size,
        mip_level_count: desc.mip_level_count,
        sample_count: 1,
        dimension: desc.dimension,
        format: desc.format,
        usage: hal_usage,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    // SAFETY: the hal device is only borrowed for this call, and only used
    // to create the image and its memory below and to wrap them.
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
    let instance = hal.shared_instance().raw_instance();
    let physical = hal.raw_physical_device();
    // SAFETY: physical device queries, valid from any thread.
    let (limits, memory) = unsafe {
        (
            instance.get_physical_device_properties(physical).limits,
            instance.get_physical_device_memory_properties(physical),
        )
    };
    if LIVE.load(Ordering::Relaxed) >= limits.max_memory_allocation_count / 2 {
        return None;
    }
    let raw = hal.raw_device();
    // the image wgpu-hal would create for this descriptor
    // (create_image_without_memory_with_tiling in wgpu-hal's vulkan/device.rs)
    let flags = if hal_desc.is_cube_compatible() {
        vk::ImageCreateFlags::CUBE_COMPATIBLE
    } else {
        vk::ImageCreateFlags::empty()
    };
    let info = vk::ImageCreateInfo::default()
        .flags(flags)
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width: desc.size.width,
            height: desc.size.height,
            depth: 1,
        })
        .mip_levels(desc.mip_level_count)
        .array_layers(desc.size.depth_or_array_layers)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    // SAFETY: a valid create info on the device wgpu opened; the image is
    // destroyed on every path below where it is not handed to wgpu.
    let image = match unsafe { raw.create_image(&info, None) } {
        Ok(image) => image,
        Err(e) => return Some(Err(format!("image not created ({e})"))),
    };
    // SAFETY: the image was just created on this device.
    let needs = unsafe { raw.get_image_memory_requirements(image) };
    let local_type = memory_type(&memory, needs.memory_type_bits);
    let local_heap = local_type.map(|i| memory.memory_types_as_slice()[i as usize].heap_index);
    let shared_type = || shared_memory_type(&memory, needs.memory_type_bits, local_heap);
    let Some(type_index) = local_type.or_else(|| integrated.then(shared_type).flatten()) else {
        // SAFETY: the image is not used anywhere yet.
        unsafe { raw.destroy_image(image, None) };
        return None;
    };
    let mut own = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let mut allocate = |type_index| {
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(needs.size)
            .memory_type_index(type_index)
            .push_next(&mut own);
        // SAFETY: allocation and binding use memory types Vulkan says are
        // compatible with this image; failures leave the image unbound.
        unsafe {
            raw.allocate_memory(&alloc, None)
                .and_then(|mem| match raw.bind_image_memory(image, mem, 0) {
                    Ok(()) => Ok(mem),
                    Err(e) => {
                        raw.free_memory(mem, None);
                        Err(e)
                    }
                })
        }
    };
    let bound = match allocate(type_index) {
        Err(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY) if integrated && local_type.is_some() => {
            let Some(shared_type) = shared_type() else {
                // SAFETY: the image is still unbound.
                unsafe { raw.destroy_image(image, None) };
                return Some(Err("device-local heap is full and no separate shared heap is available".into()));
            };
            log::debug!("integrated GPU texture page: device-local heap is full; retrying in shared system memory");
            allocate(shared_type)
        }
        result => result,
    };
    let mem = match bound {
        Ok(mem) => mem,
        Err(e) => {
            // SAFETY: the image is not used anywhere yet.
            unsafe { raw.destroy_image(image, None) };
            return Some(Err(format!("video memory not allocated ({e})")));
        }
    };
    LIVE.fetch_add(1, Ordering::Relaxed);
    BYTES.fetch_add(needs.size, Ordering::Relaxed);
    let owner = raw.clone();
    let size = needs.size;
    let release: wgpu::hal::DropCallback = Box::new(move || {
        // SAFETY: wgpu-hal calls this once, when it destroys the texture,
        // after the last submission that used it: nothing uses the image
        // nor its memory any more.
        unsafe {
            owner.destroy_image(image, None);
            owner.free_memory(mem, None);
        }
        LIVE.fetch_sub(1, Ordering::Relaxed);
        BYTES.fetch_sub(size, Ordering::Relaxed);
    });
    // SAFETY: the image was created for `hal_desc` on this device; with a
    // drop callback and External memory, wgpu-hal destroys neither and
    // leaves both to `release`.
    let hal_texture = unsafe { hal.texture_from_raw(image, &hal_desc, Some(release), wgpu::hal::vulkan::TextureMemory::External) };
    drop(hal);
    // SAFETY: the hal texture was created on this device for `desc`, in the
    // UNDEFINED layout of a new image. Its texels are undefined: like the
    // pages made by wgpu, no layer is read before it is written (textures.rs).
    let texture = unsafe { device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(hal_texture, desc, wgpu::TextureUses::UNINITIALIZED) };
    Some(Ok(texture))
}

/// The Vulkan format of the page formats (fixed: only depth and stencil
/// formats depend on the device in wgpu-hal's `map_texture_format`).
fn vk_format(format: wgpu::TextureFormat) -> Option<vk::Format> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => Some(vk::Format::R8G8B8A8_UNORM),
        wgpu::TextureFormat::Bc7RgbaUnorm => Some(vk::Format::BC7_UNORM_BLOCK),
        _ => None,
    }
}

/// The image usage and wgpu-hal's usage of the page usages (wgpu-hal's
/// `conv::map_texture_usage`).
fn usages(usage: wgpu::TextureUsages) -> Option<(vk::ImageUsageFlags, wgpu::TextureUses)> {
    use wgpu::TextureUsages as U;
    if !(U::TEXTURE_BINDING | U::COPY_DST | U::COPY_SRC).contains(usage) {
        return None;
    }
    let mut vk_usage = vk::ImageUsageFlags::empty();
    let mut hal = wgpu::TextureUses::empty();
    for (from, image, uses) in [
        (U::TEXTURE_BINDING, vk::ImageUsageFlags::SAMPLED, wgpu::TextureUses::RESOURCE),
        (U::COPY_DST, vk::ImageUsageFlags::TRANSFER_DST, wgpu::TextureUses::COPY_DST),
        (U::COPY_SRC, vk::ImageUsageFlags::TRANSFER_SRC, wgpu::TextureUses::COPY_SRC),
    ] {
        if usage.contains(from) {
            vk_usage |= image;
            hal |= uses;
        }
    }
    Some((vk_usage, hal))
}

/// The device-local memory type for a page among those `bits` allows: an
/// exact device-local type first (where wgpu's allocator puts its textures),
/// else a device-local type without unusual flags.
fn memory_type(memory: &vk::PhysicalDeviceMemoryProperties, bits: u32) -> Option<u32> {
    let plain = vk::MemoryPropertyFlags::DEVICE_LOCAL
        | vk::MemoryPropertyFlags::HOST_VISIBLE
        | vk::MemoryPropertyFlags::HOST_COHERENT
        | vk::MemoryPropertyFlags::HOST_CACHED;
    let types = memory.memory_types_as_slice();
    let allowed = |i: usize| bits & (1 << i) != 0;
    let find = |ok: &dyn Fn(vk::MemoryPropertyFlags) -> bool| {
        types
            .iter()
            .enumerate()
            .find(|(i, t)| allowed(*i) && ok(t.property_flags))
            .map(|(i, _)| i as u32)
    };
    find(&|f| f == vk::MemoryPropertyFlags::DEVICE_LOCAL)
        .or_else(|| find(&|f| f.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL) && plain.contains(f)))
}

/// A distinct host-visible system heap for an integrated GPU. Discrete cards
/// never call this fallback; a unified local heap is already tried first.
fn shared_memory_type(memory: &vk::PhysicalDeviceMemoryProperties, bits: u32, exclude_heap: Option<u32>) -> Option<u32> {
    use vk::MemoryPropertyFlags as F;
    let plain = F::DEVICE_LOCAL | F::HOST_VISIBLE | F::HOST_COHERENT | F::HOST_CACHED;
    let types = memory.memory_types_as_slice();
    let heaps = memory.memory_heaps_as_slice();
    types
        .iter()
        .enumerate()
        .filter(|(i, ty)| {
            bits & (1 << i) != 0
                && ty.property_flags.contains(F::HOST_VISIBLE)
                && plain.contains(ty.property_flags)
                && Some(ty.heap_index) != exclude_heap
                && heaps.get(ty.heap_index as usize).is_some()
        })
        // coherent first, and uncached among those: a cached (snooped)
        // type is slower for the GPU to sample
        .max_by_key(|(_, ty)| {
            (
                ty.property_flags.contains(F::HOST_COHERENT),
                !ty.property_flags.contains(F::HOST_CACHED),
            )
        })
        .map(|(i, _)| i as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(flags: &[vk::MemoryPropertyFlags]) -> vk::PhysicalDeviceMemoryProperties {
        let mut p = vk::PhysicalDeviceMemoryProperties {
            memory_type_count: flags.len() as u32,
            ..Default::default()
        };
        for (i, f) in flags.iter().enumerate() {
            p.memory_types[i].property_flags = *f;
        }
        p
    }

    #[test]
    fn pages_go_to_the_cards_own_memory() {
        use vk::MemoryPropertyFlags as F;
        let host = F::HOST_VISIBLE | F::HOST_COHERENT;
        // a discrete card with resizable BAR: the type the CPU cannot see
        let p = props(&[host, F::DEVICE_LOCAL | host, F::DEVICE_LOCAL]);
        assert_eq!(memory_type(&p, 0b111), Some(2));
        // only the CPU-visible one allowed (an integrated GPU)
        assert_eq!(memory_type(&p, 0b011), Some(1));
        assert_eq!(shared_memory_type(&p, 0b111, Some(0)), None);
        // never system memory, nor a type with a special property
        assert_eq!(memory_type(&p, 0b001), None);
        let q = props(&[F::DEVICE_LOCAL | F::LAZILY_ALLOCATED, F::DEVICE_LOCAL | F::PROTECTED]);
        assert_eq!(memory_type(&q, 0b11), None);
    }

    #[test]
    fn integrated_pages_can_fall_back_to_a_separate_shared_heap() {
        use vk::MemoryPropertyFlags as F;
        let mut p = vk::PhysicalDeviceMemoryProperties {
            memory_heap_count: 2,
            memory_type_count: 4,
            ..Default::default()
        };
        p.memory_heaps[0].flags = vk::MemoryHeapFlags::DEVICE_LOCAL;
        p.memory_heaps[1].flags = vk::MemoryHeapFlags::empty();
        p.memory_types[0].heap_index = 0;
        p.memory_types[0].property_flags = F::DEVICE_LOCAL;
        p.memory_types[1].heap_index = 0;
        p.memory_types[1].property_flags = F::DEVICE_LOCAL | F::HOST_VISIBLE | F::HOST_COHERENT;
        p.memory_types[2].heap_index = 1;
        p.memory_types[2].property_flags = F::HOST_VISIBLE | F::HOST_COHERENT;
        p.memory_types[3].heap_index = 1;
        p.memory_types[3].property_flags = F::HOST_VISIBLE | F::PROTECTED;

        assert_eq!(memory_type(&p, 0b111), Some(0));
        assert_eq!(shared_memory_type(&p, 0b111, Some(0)), Some(2));
        assert_eq!(memory_type(&p, 0b110), Some(1));
        assert_eq!(shared_memory_type(&p, 0b110, Some(0)), Some(2));
    }

    #[test]
    fn integrated_fallback_prefers_an_uncached_coherent_type() {
        use vk::MemoryPropertyFlags as F;
        let mut p = vk::PhysicalDeviceMemoryProperties {
            memory_heap_count: 2,
            memory_type_count: 3,
            ..Default::default()
        };
        p.memory_heaps[0].flags = vk::MemoryHeapFlags::DEVICE_LOCAL;
        p.memory_types[0].heap_index = 0;
        p.memory_types[0].property_flags = F::DEVICE_LOCAL;
        // AMD's GTT: write-combined first, then the cached (snooped) type
        p.memory_types[1].heap_index = 1;
        p.memory_types[1].property_flags = F::HOST_VISIBLE | F::HOST_COHERENT;
        p.memory_types[2].heap_index = 1;
        p.memory_types[2].property_flags = F::HOST_VISIBLE | F::HOST_COHERENT | F::HOST_CACHED;
        assert_eq!(shared_memory_type(&p, 0b111, Some(0)), Some(1));
        // only the cached one allowed: it is still taken
        assert_eq!(shared_memory_type(&p, 0b101, Some(0)), Some(2));
    }

    #[test]
    fn integrated_fallback_can_use_a_distinct_device_local_host_visible_heap() {
        use vk::MemoryPropertyFlags as F;
        let mut p = vk::PhysicalDeviceMemoryProperties {
            memory_heap_count: 2,
            memory_type_count: 2,
            ..Default::default()
        };
        p.memory_heaps[0].flags = vk::MemoryHeapFlags::DEVICE_LOCAL;
        p.memory_heaps[1].flags = vk::MemoryHeapFlags::DEVICE_LOCAL;
        p.memory_types[0].heap_index = 0;
        p.memory_types[0].property_flags = F::DEVICE_LOCAL;
        p.memory_types[1].heap_index = 1;
        p.memory_types[1].property_flags = F::DEVICE_LOCAL | F::HOST_VISIBLE | F::HOST_COHERENT;

        assert_eq!(memory_type(&p, 0b11), Some(0));
        assert_eq!(shared_memory_type(&p, 0b11, Some(0)), Some(1));
        assert_eq!(shared_memory_type(&p, 0b11, Some(1)), None);
    }

    #[test]
    fn only_the_page_usages_and_formats() {
        use wgpu::TextureUsages as U;
        let (image, hal) = usages(U::TEXTURE_BINDING | U::COPY_DST | U::COPY_SRC).expect("page usages");
        assert_eq!(
            image,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC
        );
        assert_eq!(
            hal,
            wgpu::TextureUses::RESOURCE | wgpu::TextureUses::COPY_DST | wgpu::TextureUses::COPY_SRC
        );
        assert!(usages(U::TEXTURE_BINDING | U::RENDER_ATTACHMENT).is_none());
        assert!(vk_format(wgpu::TextureFormat::Rgba8UnormSrgb).is_none());
        assert_eq!(vk_format(wgpu::TextureFormat::Bc7RgbaUnorm), Some(vk::Format::BC7_UNORM_BLOCK));
    }
}
