//! Optional Vulkan extensions are negotiated before both ordinary and XR
//! device creation. No required wgpu feature or extension is removed.
use super::{Capabilities, Method, Mode};
use ash::{amd, ext, nv, vk};
use std::ffi::CStr;
use wgpu::hal::api::Vulkan;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Plan {
    pub(crate) offered: Capabilities,
    pub(crate) enabled: Capabilities,
    pub(crate) method: Method,
}
impl Plan {
    pub(crate) fn query(adapter: &wgpu::Adapter) -> Self {
        // SAFETY: read-only Vulkan adapter access for the guard's lifetime.
        let Some(hal) = (unsafe { adapter.as_hal::<Vulkan>() }) else {
            return Self::default();
        };
        let instance = hal.shared_instance().raw_instance();
        let physical = hal.raw_physical_device();
        // SAFETY: physical belongs to this live instance.
        let Ok(extensions) = (unsafe { instance.enumerate_device_extension_properties(physical) }) else {
            return Self::default();
        };
        let has = |name: &CStr| extensions.iter().any(|e| e.extension_name_as_c_str().ok() == Some(name));
        let mut offered = Capabilities {
            amd: has(amd::buffer_marker::NAME),
            nv: has(nv::device_diagnostic_checkpoints::NAME),
            fault: has(ext::device_fault::NAME),
            coherent: has(amd::device_coherent_memory::NAME),
        };
        let mut fault = vk::PhysicalDeviceFaultFeaturesEXT::default();
        let mut coherent = vk::PhysicalDeviceCoherentMemoryFeaturesAMD::default();
        let mut query = vk::PhysicalDeviceFeatures2::default();
        if offered.fault {
            query = query.push_next(&mut fault);
        }
        if offered.coherent {
            query = query.push_next(&mut coherent);
        }
        // SAFETY: only advertised extension structures are chained; storage
        // lives until this synchronous capability query returns.
        unsafe {
            instance.get_physical_device_features2(physical, &mut query);
        }
        let fault_offered = offered.fault;
        offered.fault &= fault.device_fault == vk::TRUE;
        offered.coherent &= coherent.device_coherent_memory == vk::TRUE;
        let mut plan = Self::choose(offered, Mode::environment());
        plan.offered.fault = fault_offered;
        plan
    }
    pub(super) fn choose(offered: Capabilities, mode: Mode) -> Self {
        let method = offered.select(mode);
        Self {
            offered,
            method,
            enabled: Capabilities {
                amd: method == Method::Amd,
                nv: method == Method::Nv,
                fault: offered.fault && mode != Mode::Off,
                coherent: offered.coherent && matches!(method, Method::Amd | Method::Transfer),
            },
        }
    }
    pub(crate) fn extensions(&self, names: &mut Vec<&'static CStr>) {
        for (on, name) in [
            (self.enabled.amd, amd::buffer_marker::NAME),
            (self.enabled.nv, nv::device_diagnostic_checkpoints::NAME),
            (self.enabled.fault, ext::device_fault::NAME),
            (self.enabled.coherent, amd::device_coherent_memory::NAME),
        ] {
            if on && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    pub(crate) fn active(&self) -> bool {
        self.enabled != Capabilities::default() || self.method == Method::Transfer
    }
    pub(crate) fn disable(&mut self) {
        self.enabled = Capabilities::default();
        self.method = Method::Unavailable;
    }
    pub(crate) fn fault_feature(&self) -> vk::PhysicalDeviceFaultFeaturesEXT<'static> {
        vk::PhysicalDeviceFaultFeaturesEXT::default()
            .device_fault(self.enabled.fault)
            .device_fault_vendor_binary(false)
    }
    pub(crate) fn coherent_feature(&self) -> vk::PhysicalDeviceCoherentMemoryFeaturesAMD<'static> {
        vk::PhysicalDeviceCoherentMemoryFeaturesAMD::default().device_coherent_memory(self.enabled.coherent)
    }
}

pub(crate) fn open(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor<'_>,
    plan: &Plan,
) -> Result<(wgpu::Device, wgpu::Queue), String> {
    if !plan.active() {
        return pollster::block_on(adapter.request_device(descriptor)).map_err(|e| e.to_string());
    }
    let mut fault = plan.fault_feature();
    let mut coherent = plan.coherent_feature();
    // SAFETY: the queried plan only adds supported extensions and feature
    // bits. Owned feature storage outlives the callback and vkCreateDevice.
    let opened = unsafe {
        let hal = adapter.as_hal::<Vulkan>().ok_or("not a Vulkan adapter")?;
        hal.open_with_callback(
            descriptor.required_features,
            &descriptor.required_limits,
            &descriptor.memory_hints,
            Some(Box::new(|args| {
                plan.extensions(args.extensions);
                if plan.enabled.fault {
                    *args.create_info = args.create_info.push_next(&mut fault);
                }
                if plan.enabled.coherent {
                    *args.create_info = args.create_info.push_next(&mut coherent);
                }
            })),
        )
        .map_err(|e| e.to_string())?
    };
    // SAFETY: HAL device was opened from exactly this adapter and descriptor.
    unsafe { adapter.create_device_from_hal::<Vulkan>(opened, descriptor) }.map_err(|e| e.to_string())
}

/// Retains wgpu's device (and its shared instance) until all raw diagnostic
/// objects and any asynchronous driver query are finished. No wgpu access
/// happens from a loss callback or while a borrowed HAL encoder is active.
pub(super) struct Native {
    owner: wgpu::Device,
    raw: ash::Device,
    queue: vk::Queue,
    loss_probe: vk::Fence,
    amd: Option<amd::buffer_marker::Device>,
    nv: Option<nv::device_diagnostic_checkpoints::Device>,
    nv_stages: vk::PipelineStageFlags,
    fault: Option<ext::device_fault::Device>,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: usize,
    pub(super) allocation_bytes: u64,
}
impl Native {
    pub(super) fn new(device: &wgpu::Device, plan: &Plan) -> Result<Self, vk::Result> {
        // SAFETY: borrowed handles are copied while live; owner below retains
        // their actual lifetime. No raw handle owns/destroys wgpu's device.
        let hal = unsafe { device.as_hal::<Vulkan>() }.ok_or(vk::Result::ERROR_FEATURE_NOT_PRESENT)?;
        let instance = hal.shared_instance().raw_instance();
        let raw = hal.raw_device();
        let mut native = Self {
            owner: device.clone(),
            raw: raw.clone(),
            queue: hal.raw_queue(),
            loss_probe: vk::Fence::null(),
            amd: plan.enabled.amd.then(|| amd::buffer_marker::Device::new(instance, raw)),
            nv: plan
                .enabled
                .nv
                .then(|| nv::device_diagnostic_checkpoints::Device::new(instance, raw)),
            nv_stages: checkpoint_stages(instance, hal.raw_physical_device(), hal.queue_family_index(), plan.enabled.nv),
            fault: plan.enabled.fault.then(|| ext::device_fault::Device::new(instance, raw)),
            buffer: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            mapped: 0,
            allocation_bytes: 0,
        };
        if plan.active() {
            // SAFETY: unsignalled fence is never submitted. Its status gives
            // a typed ERROR_DEVICE_LOST check before any post-loss query.
            native.loss_probe = unsafe { raw.create_fence(&vk::FenceCreateInfo::default(), None)? };
        }
        if !matches!(plan.method, Method::Amd | Method::Transfer) {
            return Ok(native);
        }
        // SAFETY: diagnostic-only buffer; never imported into wgpu's tracker.
        unsafe {
            native.buffer = raw.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size((super::PAYLOAD_WORDS * 4) as u64)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )?;
            let requirements = raw.get_buffer_memory_requirements(native.buffer);
            let properties = instance.get_physical_device_memory_properties(hal.raw_physical_device());
            let flags = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let memory_type = properties.memory_types[..properties.memory_type_count as usize]
                .iter()
                .enumerate()
                .filter(|(i, m)| requirements.memory_type_bits & (1 << i) != 0 && m.property_flags.contains(flags))
                // Device-coherent types are illegal unless their feature was enabled.
                .filter(|(_, m)| plan.enabled.coherent || !m.property_flags.contains(vk::MemoryPropertyFlags::DEVICE_COHERENT_AMD))
                .max_by_key(|(_, m)| m.property_flags.contains(vk::MemoryPropertyFlags::DEVICE_COHERENT_AMD))
                .map(|(i, _)| i as u32)
                .ok_or(vk::Result::ERROR_FEATURE_NOT_PRESENT)?;
            native.memory = raw.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(memory_type),
                None,
            )?;
            native.allocation_bytes = requirements.size;
            raw.bind_buffer_memory(native.buffer, native.memory, 0)?;
            native.mapped = raw.map_memory(native.memory, 0, requirements.size, vk::MemoryMapFlags::empty())? as usize;
            // Before any GPU access; coherent mapping needs no flush.
            std::ptr::write_bytes(native.mapped as *mut u8, 0, super::PAYLOAD_WORDS * 4);
        }
        Ok(native)
    }
    pub(super) fn record(&self, encoder: &mut wgpu::CommandEncoder, method: Method, offset: usize, token: u32, begin: bool) {
        // SAFETY: the pass has ended, as_hal_mut supplies a live recording
        // buffer, and this closure calls no wgpu methods. Only our dedicated
        // buffer is touched. Each word is written once per completed epoch.
        unsafe {
            encoder.as_hal_mut::<Vulkan, _, _>(|hal| {
                let Some(hal) = hal else {
                    return;
                };
                let command = hal.raw_handle();
                let stage = if begin {
                    vk::PipelineStageFlags::TOP_OF_PIPE
                } else {
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE
                };
                match method {
                    Method::Amd => {
                        if let Some(amd) = &self.amd {
                            amd.cmd_write_buffer_marker(command, stage, self.buffer, (offset * 4) as u64, token);
                        }
                    }
                    Method::Nv => {
                        if let Some(nv) = &self.nv {
                            // Vulkan treats this as an opaque identifier and never
                            // dereferences it. Finite nonzero epochs do not wrap;
                            // decoding verifies membership in this device's ring.
                            nv.cmd_set_checkpoint(command, std::ptr::without_provenance(token as usize));
                        }
                    }
                    Method::Transfer => {
                        self.raw.cmd_pipeline_barrier(
                            command,
                            vk::PipelineStageFlags::ALL_COMMANDS,
                            vk::PipelineStageFlags::TRANSFER,
                            vk::DependencyFlags::empty(),
                            &[],
                            &[],
                            &[],
                        );
                        self.raw.cmd_fill_buffer(command, self.buffer, (offset * 4) as u64, 4, token);
                        self.raw.cmd_pipeline_barrier(
                            command,
                            vk::PipelineStageFlags::TRANSFER,
                            vk::PipelineStageFlags::ALL_COMMANDS,
                            vk::DependencyFlags::empty(),
                            &[],
                            &[],
                            &[],
                        );
                    }
                    Method::Unavailable => {}
                }
            });
        }
    }
    pub(super) fn host_barrier(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.buffer == vk::Buffer::null() {
            return;
        }
        // SAFETY: dedicated buffer only, pass ended; makes completed writes
        // visible to a host snapshot after the submission completion callback.
        unsafe {
            encoder.as_hal_mut::<Vulkan, _, _>(|hal| {
                if let Some(hal) = hal {
                    self.raw.cmd_pipeline_barrier(
                        hal.raw_handle(),
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::PipelineStageFlags::HOST,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[vk::BufferMemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                            .dst_access_mask(vk::AccessFlags::HOST_READ)
                            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                            .buffer(self.buffer)
                            .offset(0)
                            .size(vk::WHOLE_SIZE)],
                        &[],
                    );
                }
            });
        }
    }
    pub(super) fn words(&self) -> Option<[u32; super::PAYLOAD_WORDS]> {
        if self.mapped == 0 {
            return None;
        }
        // SAFETY: mapping is retained by self.owner. Called only after known
        // completion or confirmed device loss; never during live GPU writes.
        // Contents after loss are undefined by Vulkan: validate every token.
        Some(std::array::from_fn(|i| unsafe {
            ((self.mapped as *const u32).add(i)).read_volatile()
        }))
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        // SAFETY: Recorder's submitted frames/worker retain it until completed
        // or confirmed loss. Raw objects are destroyed before owner is dropped.
        unsafe {
            if self.loss_probe != vk::Fence::null() {
                self.raw.destroy_fence(self.loss_probe, None);
            }
            if self.mapped != 0 {
                self.raw.unmap_memory(self.memory);
            }
            if self.buffer != vk::Buffer::null() {
                self.raw.destroy_buffer(self.buffer, None);
            }
            if self.memory != vk::DeviceMemory::null() {
                self.raw.free_memory(self.memory, None);
            }
        }
        let _ = &self.owner;
    }
}

impl Native {
    pub(super) fn completed_words(&self, slot: usize) -> Option<[u32; super::WORDS]> {
        if self.mapped == 0 || slot >= super::SLOTS {
            return None;
        }
        // SAFETY: caller holds the ring lock and this slot's submission has
        // completed with a HOST visibility barrier; no concurrent slot reuse.
        Some(std::array::from_fn(|i| unsafe {
            ((self.mapped as *const u32).add(slot * super::WORDS + i)).read_volatile()
        }))
    }
    pub(super) fn collect(&self, ring: &std::sync::Mutex<super::Ring>, report: &mut super::Report, private: &mut Vec<u8>) {
        // Once per confirmed device loss, on the long-lived worker. Nothing
        // waits for this method: even a permanently stuck driver query only
        // retains this one device and leaves the report's pending status.
        if self.loss_probe == vk::Fence::null() {
            return;
        }
        // wgpu also invalidates devices for OOM/Unexpected errors. The EXT
        // query requires actual Vulkan device loss, not merely wgpu invalidity.
        // This driver call runs only here, never on a crash/reporting thread.
        // SAFETY: unsignalled live fence created on this retained device.
        if unsafe { self.raw.get_fence_status(self.loss_probe) } != Err(vk::Result::ERROR_DEVICE_LOST) {
            report.fault = super::FaultStatus::Unconfirmed;
            return;
        }
        if let Ok(ring) = ring.lock() {
            report.saturated = ring.saturated;
            if let Some(words) = self.words() {
                let observed = words
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, token)| ring.decode(report.generation, i, token))
                    .max_by_key(|o| o.token);
                if let Some(observed) = observed {
                    report.observed = Some(observed);
                    report.stage = if report.method == Method::Transfer {
                        super::Stage::Transfer
                    } else if observed.boundary.begin() {
                        super::Stage::Top
                    } else {
                        super::Stage::Bottom
                    };
                }
            }
        }
        if let Some(nv) = &self.nv {
            let mut checkpoints = [vk::CheckpointDataNV::default(); 64];
            let mut count = checkpoints.len() as u32;
            // SAFETY: fixed valid capacity, live queue, initialized structures.
            // Use the raw entry point: ash's helper asserts the returned count
            // equals the capacity, which need not hold after a loss.
            unsafe {
                (nv.fp().get_queue_checkpoint_data_nv)(self.queue, &mut count, checkpoints.as_mut_ptr());
            }
            if let Ok(ring) = ring.lock() {
                for checkpoint in checkpoints.iter().take((count as usize).min(checkpoints.len())) {
                    let Ok(token) = u32::try_from(checkpoint.p_checkpoint_marker as usize) else {
                        continue;
                    };
                    if let Some(observed) = ring.checkpoint(token)
                        && report.observed.is_none_or(|old| old.token <= observed.token)
                    {
                        report.observed = Some(observed);
                        report.stage = match checkpoint.stage & self.nv_stages {
                            vk::PipelineStageFlags::TOP_OF_PIPE => super::Stage::Top,
                            vk::PipelineStageFlags::BOTTOM_OF_PIPE => super::Stage::Bottom,
                            _ => super::Stage::Other,
                        };
                    }
                }
            }
        }
        let Some(fault) = &self.fault else {
            return;
        };
        let mut counts = vk::DeviceFaultCountsEXT::default();
        // SAFETY: only invoked after confirmed device loss, feature enabled at
        // creation. First call queries counts and does not supply output arrays.
        let result = unsafe { (fault.fp().get_device_fault_info_ext)(self.raw.handle(), &mut counts, std::ptr::null_mut()) };
        if result != vk::Result::SUCCESS && result != vk::Result::INCOMPLETE {
            report.fault = super::FaultStatus::Failed;
            private_error(private, result);
            return;
        }
        let budget = FaultBudget::new(&counts);
        let (address_count, vendor_count) = (budget.addresses, budget.vendors);
        let mut truncated = budget.truncated || result == vk::Result::INCOMPLETE;
        let mut addresses = Vec::new();
        let mut vendors = Vec::new();
        if addresses.try_reserve_exact(address_count).is_err()
            || vendors.try_reserve_exact(vendor_count).is_err()
            || private.try_reserve(PRIVATE_LIMIT).is_err()
        {
            report.fault = super::FaultStatus::Failed;
            return;
        }
        addresses.resize(address_count, vk::DeviceFaultAddressInfoEXT::default());
        vendors.resize(vendor_count, vk::DeviceFaultVendorInfoEXT::default());
        counts.address_info_count = address_count as u32;
        counts.vendor_info_count = vendor_count as u32;
        counts.vendor_binary_size = 0; // Vendor binary feature is deliberately not enabled.
        let mut info = vk::DeviceFaultInfoEXT {
            p_address_infos: addresses.as_mut_ptr(),
            p_vendor_infos: vendors.as_mut_ptr(),
            ..vk::DeviceFaultInfoEXT::default()
        };
        // SAFETY: bounded arrays describe their capacities. VK_INCOMPLETE is
        // retained as truncation, never retried with unbounded allocations.
        let result = unsafe { (fault.fp().get_device_fault_info_ext)(self.raw.handle(), &mut counts, &mut info) };
        if result != vk::Result::SUCCESS && result != vk::Result::INCOMPLETE {
            report.fault = super::FaultStatus::Failed;
            private_error(private, result);
            return;
        }
        truncated |= result == vk::Result::INCOMPLETE
            || counts.address_info_count > address_count as u32
            || counts.vendor_info_count > vendor_count as u32;
        let description = info.description;
        report.addresses = counts.address_info_count.min(address_count as u32);
        report.vendors = counts.vendor_info_count.min(vendor_count as u32);
        use std::io::Write;
        let _ = writeln!(
            private,
            "Private Vulkan device fault detail\nresult: {result:?}\ndescription: {}",
            driver_text(&description)
        );
        for address in addresses.iter().take(report.addresses as usize) {
            let bit = match address.address_type {
                vk::DeviceFaultAddressTypeEXT::READ_INVALID => 0,
                vk::DeviceFaultAddressTypeEXT::WRITE_INVALID => 1,
                vk::DeviceFaultAddressTypeEXT::EXECUTE_INVALID => 2,
                vk::DeviceFaultAddressTypeEXT::INSTRUCTION_POINTER_UNKNOWN
                | vk::DeviceFaultAddressTypeEXT::INSTRUCTION_POINTER_INVALID
                | vk::DeviceFaultAddressTypeEXT::INSTRUCTION_POINTER_FAULT => 3,
                _ => 4,
            };
            report.address_kinds |= 1 << bit;
            let _ = writeln!(
                private,
                "address type {:?}, address {:#x}, precision {}",
                address.address_type, address.reported_address, address.address_precision
            );
        }
        for vendor in vendors.iter().take(report.vendors as usize) {
            let _ = writeln!(
                private,
                "vendor code {:#x}, data {:#x}, description {}",
                vendor.vendor_fault_code,
                vendor.vendor_fault_data,
                driver_text(&vendor.description)
            );
        }
        if private.len() > PRIVATE_LIMIT {
            private.truncate(PRIVATE_LIMIT);
            truncated = true;
        }
        report.fault = if truncated {
            super::FaultStatus::Truncated
        } else if report.addresses == 0 && report.vendors == 0 && driver_text(&description).is_empty() {
            super::FaultStatus::NoDetail
        } else {
            super::FaultStatus::Collected
        };
    }
}
const PRIVATE_LIMIT: usize = 512 * 1024;
struct FaultBudget {
    addresses: usize,
    vendors: usize,
    truncated: bool,
}
impl FaultBudget {
    fn new(counts: &vk::DeviceFaultCountsEXT<'_>) -> Self {
        let addresses = counts.address_info_count.min(1024) as usize;
        let vendors = counts.vendor_info_count.min(256) as usize;
        Self {
            addresses,
            vendors,
            truncated: counts.address_info_count > addresses as u32
                || counts.vendor_info_count > vendors as u32
                || counts.vendor_binary_size > 0,
        }
    }
}
fn checkpoint_stages(instance: &ash::Instance, physical: vk::PhysicalDevice, family: u32, enabled: bool) -> vk::PipelineStageFlags {
    if !enabled {
        return vk::PipelineStageFlags::empty();
    }
    // SAFETY: physical belongs to the live instance. Bounded property arrays
    // own every pNext structure through the synchronous query.
    unsafe {
        let count = instance.get_physical_device_queue_family_properties2_len(physical);
        if count > 64 || family as usize >= count {
            return vk::PipelineStageFlags::empty();
        }
        let mut checkpoints = vec![vk::QueueFamilyCheckpointPropertiesNV::default(); count];
        let mut families: Vec<_> = checkpoints
            .iter_mut()
            .map(|checkpoint| vk::QueueFamilyProperties2::default().push_next(checkpoint))
            .collect();
        instance.get_physical_device_queue_family_properties2(physical, &mut families);
        checkpoints[family as usize].checkpoint_execution_stage_mask
    }
}
fn private_error(private: &mut Vec<u8>, result: vk::Result) {
    if private.try_reserve(256).is_ok() {
        use std::io::Write;
        let _ = writeln!(private, "Private Vulkan device fault query failed: {result:?}");
    }
}
fn driver_text(chars: &[std::ffi::c_char]) -> String {
    let bytes: Vec<_> = chars.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn breadcrumb_fault_counts_cannot_allocate_more_than_the_budget() {
        let counts = vk::DeviceFaultCountsEXT {
            address_info_count: u32::MAX,
            vendor_info_count: u32::MAX,
            vendor_binary_size: u64::MAX,
            ..Default::default()
        };
        let budget = FaultBudget::new(&counts);
        assert!(budget.truncated);
        assert_eq!((budget.addresses, budget.vendors), (1024, 256));
        let bytes = budget.addresses * std::mem::size_of::<vk::DeviceFaultAddressInfoEXT>()
            + budget.vendors * std::mem::size_of::<vk::DeviceFaultVendorInfoEXT>()
            + PRIVATE_LIMIT;
        assert!(bytes < 1024 * 1024);
        assert!(!FaultBudget::new(&vk::DeviceFaultCountsEXT::default()).truncated);
    }
    #[test]
    fn breadcrumb_creation_preserves_required_extensions_for_ordinary_and_xr_devices() {
        let offered = Capabilities {
            amd: true,
            nv: true,
            fault: true,
            coherent: true,
        };
        let plan = Plan::choose(offered, Mode::Auto);
        let mut ordinary = vec![ash::khr::swapchain::NAME];
        let mut xr = ordinary.clone();
        plan.extensions(&mut ordinary);
        plan.extensions(&mut xr);
        assert_eq!(ordinary, xr);
        assert!(ordinary.contains(&ash::khr::swapchain::NAME));
        assert!(ordinary.contains(&amd::buffer_marker::NAME));
        assert!(ordinary.contains(&ext::device_fault::NAME));
        assert!(!ordinary.contains(&nv::device_diagnostic_checkpoints::NAME));
        assert_eq!(plan.fault_feature().device_fault, vk::TRUE);
        assert_eq!(plan.fault_feature().device_fault_vendor_binary, vk::FALSE);
        plan.extensions(&mut ordinary);
        assert_eq!(ordinary, xr, "no duplicate extension names");
        let mut fallback = plan;
        fallback.disable();
        assert!(!fallback.active());
        assert_eq!(fallback.offered, offered);
        let mut unsupported = vec![ash::khr::swapchain::NAME];
        Plan::default().extensions(&mut unsupported);
        assert_eq!(unsupported, [ash::khr::swapchain::NAME]);
    }
}
