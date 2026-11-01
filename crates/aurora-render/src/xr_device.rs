//! OpenXR-compatible Vulkan bootstrap for the Linux/Windows viewer experiment.
//! Instance/device creation follows openxrs/examples/vulkan.rs (MIT/Apache-2.0).
//! HAL owns the imported graphics handles; ash clones only borrow their dispatch tables.
use ash::{vk, vk::Handle};
use openxr as xr;
use wgpu::hal;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(crate) struct Graphics {
    pub instance: wgpu::Instance,
    xr: xr::Instance,
    system: xr::SystemId,
    vk_entry: ash::Entry,
    raw_instance: ash::Instance,
    physical: vk::PhysicalDevice,
}
/// The OpenXR loader: built in on Windows (the `static` feature), the
/// distribution's on Linux, which its runtimes install.
fn entry() -> Result<xr::Entry> {
    #[cfg(target_os = "windows")]
    return Ok(xr::Entry::linked(&())?);
    // SAFETY: openxr's checked loading of the system loader library.
    #[cfg(not(target_os = "windows"))]
    return Ok(unsafe { xr::Entry::load(&())? });
}

/// Whether the active OpenXR runtime has a headset now (instance and
/// system only, no session): when VR was unavailable at startup because the
/// headset was not connected yet, a restart can then bring it.
pub fn headset_ready() -> bool {
    let probe = || -> Result<()> {
        let entry = entry()?;
        if !entry.enumerate_extensions()?.khr_vulkan_enable2 {
            return Err("XR_KHR_vulkan_enable2 unavailable".into());
        }
        let mut extensions = xr::ExtensionSet::default();
        extensions.khr_vulkan_enable2 = true;
        let xr = entry.create_instance(
            &xr::ApplicationInfo {
                application_name: "Aurora headset probe",
                application_version: 0,
                engine_name: "Aurora",
                engine_version: 0,
                api_version: xr::Version::new(1, 0, 0),
            },
            &extensions,
            &[],
            &(),
        )?;
        xr.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)?;
        Ok(())
    };
    match probe() {
        Ok(()) => true,
        Err(e) => {
            log::info!("OpenXR headset probe: {e}");
            false
        }
    }
}

impl Graphics {
    pub fn binding(&self) -> (xr::Instance, xr::SystemId) {
        (self.xr.clone(), self.system)
    }
    pub fn new(flags: wgpu::InstanceFlags) -> Result<Self> {
        // The Khronos loader is linked in (the `static` feature): it finds the
        // runtime the system has active, with nothing installed beside the exe.
        let entry = entry()?;
        let available = entry.enumerate_extensions()?;
        if !available.khr_vulkan_enable2 {
            return Err("XR_KHR_vulkan_enable2 unavailable".into());
        }
        let mut extensions = xr::ExtensionSet::default();
        extensions.khr_vulkan_enable2 = true;
        let xr = entry.create_instance(
            &xr::ApplicationInfo {
                application_name: "Aurora Vulkan interoperability",
                application_version: 0,
                engine_name: "Aurora experiment",
                engine_version: 0,
                api_version: xr::Version::new(1, 0, 0),
            },
            &extensions,
            &[],
            &(),
        )?;
        let system = xr.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)?;
        let requirements = xr.graphics_requirements::<xr::Vulkan>(system)?;
        let api = vk::API_VERSION_1_3;
        if requirements.min_api_version_supported > xr::Version::new(1, 3, 0) {
            return Err("Runtime requires newer Vulkan than this experiment".into());
        }
        // SAFETY: ash loads the system Vulkan loader; all handles below belong to it.
        let vk_entry = unsafe { ash::Entry::load()? };
        let instance_extensions = hal::vulkan::Instance::desired_extensions(&vk_entry, api, flags)?;
        let pointers: Vec<_> = instance_extensions.iter().map(|s| s.as_ptr()).collect();
        let app = vk::ApplicationInfo::default().api_version(api);
        let create = vk::InstanceCreateInfo::default()
            .application_info(&app)
            .enabled_extension_names(&pointers);
        // SAFETY: Proc address types have the same Vulkan ABI. Create-info and its pointees
        // remain valid during the synchronous OpenXR call. OpenXR adds its required extensions.
        let raw = unsafe {
            xr.create_vulkan_instance(
                system,
                std::mem::transmute::<vk::PFN_vkGetInstanceProcAddr, xr::sys::platform::VkGetInstanceProcAddr>(
                    vk_entry.static_fn().get_instance_proc_addr,
                ),
                &create as *const _ as *const _,
            )
        }?
        .map_err(vk::Result::from_raw)?;
        // SAFETY: The runtime returned a valid instance from this loader.
        let raw_instance = unsafe { ash::Instance::load(vk_entry.static_fn(), vk::Instance::from_raw(raw as _)) };
        let physical = vk::PhysicalDevice::from_raw(unsafe {
            // SAFETY: Querying the runtime-selected device for the live instance.
            xr.vulkan_graphics_device(system, raw_instance.handle().as_raw() as _)? as _
        });
        // SAFETY: The created instance enables wgpu's desired extensions. Ownership moves
        // into HAL, then wgpu; the ash clone is only borrowed for OpenXR bootstrap.
        let hi = unsafe {
            hal::vulkan::Instance::from_raw(
                vk_entry.clone(),
                raw_instance.clone(),
                api,
                0,
                None,
                instance_extensions,
                flags,
                Default::default(),
                false,
                None,
            )?
        };
        let instance = unsafe { wgpu::Instance::from_hal::<hal::api::Vulkan>(hi) };
        Ok(Self {
            instance,
            xr,
            system,
            vk_entry,
            raw_instance,
            physical,
        })
    }
    pub fn open(&self, adapter: &wgpu::Adapter, descriptor: &wgpu::DeviceDescriptor<'_>) -> Result<(wgpu::Device, wgpu::Queue)> {
        let Self {
            xr,
            system,
            vk_entry,
            raw_instance,
            physical,
            ..
        } = self;
        let system = *system;
        let physical = *physical;
        let features = descriptor.required_features;
        let limits = descriptor.required_limits.clone();
        // SAFETY: Read-only handle comparison; both handles remain live for this call.
        if unsafe { adapter.as_hal::<hal::api::Vulkan>() }.is_none_or(|a| a.raw_physical_device() != physical) {
            return Err("Viewer GPU differs from the runtime-selected GPU".into());
        }
        // SAFETY: Backend is Vulkan, adapter remains alive throughout bootstrap.
        let ha = unsafe { adapter.as_hal::<hal::api::Vulkan>() }.ok_or("Not Vulkan")?;
        let device_extensions = ha.required_device_extensions(features);
        let names: Vec<_> = device_extensions.iter().map(|s| s.as_ptr()).collect();
        let mut enabled_features = ha.physical_device_features(&device_extensions, features);
        // SAFETY: physical belongs to raw_instance and is valid until wgpu is dropped.
        let family = unsafe { raw_instance.get_physical_device_queue_family_properties(physical) }
            .iter()
            .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or("No graphics queue")? as u32;
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&[1.0])];
        let create = enabled_features.add_to_device_create(
            vk::DeviceCreateInfo::default()
                .queue_create_infos(&queues)
                .enabled_extension_names(&names),
        );
        // SAFETY: All create-info feature/extension storage lives through the synchronous call.
        let raw = unsafe {
            xr.create_vulkan_device(
                system,
                std::mem::transmute::<vk::PFN_vkGetInstanceProcAddr, xr::sys::platform::VkGetInstanceProcAddr>(
                    vk_entry.static_fn().get_instance_proc_addr,
                ),
                physical.as_raw() as _,
                &create as *const _ as *const _,
            )
        }?
        .map_err(vk::Result::from_raw)?;
        // SAFETY: Runtime-created device belongs to the selected adapter and includes HAL features.
        let raw_device = unsafe { ash::Device::load(raw_instance.fp_v1_0(), vk::Device::from_raw(raw as _)) };
        let hd = unsafe {
            ha.device_from_raw(
                raw_device.clone(),
                None,
                &device_extensions,
                features,
                &limits,
                &descriptor.memory_hints,
                family,
                0,
            )?
        };
        let result = unsafe { adapter.create_device_from_hal::<hal::api::Vulkan>(hd, descriptor)? };
        Ok(result)
    }
}
