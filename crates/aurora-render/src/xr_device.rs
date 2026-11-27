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
    /// The instance carries the DLSS extensions (a build with DLSS whose
    /// files are set up, dlss.rs): the device is then asked for DLSS's too.
    with_dlss: bool,
}
/// The OpenXR loader: built in on Windows (the `static` feature), the
/// distribution's on Linux, which its runtimes install.
fn entry() -> Result<xr::Entry> {
    #[cfg(feature = "xr-static-loader")]
    return Ok(xr::Entry::linked(&())?);
    // SAFETY: openxr's checked loading of the system loader library.
    #[cfg(not(feature = "xr-static-loader"))]
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
        // Only for the log: a runtime that cannot say its name still works.
        match xr.properties() {
            Ok(properties) => log::info!("OpenXR runtime loaded: {}", runtime_class(&properties.runtime_name)),
            Err(e) => {
                log::info!("OpenXR runtime name unavailable: {e}");
                log::info!("OpenXR runtime loaded: other OpenXR runtime");
            }
        }
        let system = xr.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)?;
        let requirements = xr.graphics_requirements::<xr::Vulkan>(system)?;
        let api = vk::API_VERSION_1_3;
        if requirements.min_api_version_supported > xr::Version::new(1, 3, 0) {
            return Err("Runtime requires newer Vulkan than this experiment".into());
        }
        // Windows, with DLSS set up: Streamline's interposer as the loader,
        // so that it sees the instance and device made (dlss.rs)
        #[cfg(windows)]
        let interposer = crate::dlss::xr_entry();
        #[cfg(not(windows))]
        let interposer: Option<ash::Entry> = None;
        let vk_entry = match interposer {
            Some(entry) => entry,
            // SAFETY: ash loads the system Vulkan loader; all handles below belong to it.
            None => unsafe { ash::Entry::load()? },
        };
        let instance_extensions = hal::vulkan::Instance::desired_extensions(&vk_entry, api, flags)?;
        // Streamline's too, when DLSS is set up, so that DLSS draws in the headset
        // as on the desktop (dlss.rs); wgpu-hal is told only of its own
        let dlss_extensions = crate::dlss::xr_instance_extensions(&vk_entry);
        let with_dlss = dlss_extensions.is_some();
        let mut enabled = instance_extensions.clone();
        for name in dlss_extensions.into_iter().flatten() {
            if !enabled.contains(&name) {
                enabled.push(name);
            }
        }
        let create_instance = |names: &[&std::ffi::CStr]| -> Result<_> {
            let pointers: Vec<_> = names.iter().map(|s| s.as_ptr()).collect();
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
            Ok(raw)
        };
        // a runtime that refuses the DLSS extensions still gets its headset:
        // the instance is made as before and DLSS is not offered in VR
        let (raw, with_dlss) = match create_instance(&enabled) {
            Ok(raw) => (raw, with_dlss),
            Err(e) if with_dlss => {
                log::error!("DLSS: the VR runtime refused the instance with the DLSS extensions ({e}): made without them, FSR draws");
                (create_instance(&instance_extensions)?, false)
            }
            Err(e) => return Err(e),
        };
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
            with_dlss,
        })
    }
    pub fn open(&self, adapter: &wgpu::Adapter, descriptor: &wgpu::DeviceDescriptor<'_>) -> Result<(wgpu::Device, wgpu::Queue)> {
        let Self {
            xr,
            system,
            vk_entry,
            raw_instance,
            physical,
            with_dlss,
            ..
        } = self;
        // the device opened for DLSS before is forgotten, whatever happens
        // below (dlss.rs); false: Streamline is still busy on a lost device, and
        // this one opens without it
        let dlss_free = crate::dlss::opening_device(true);
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
        // the DLSS device extensions beside wgpu's (wgpu-hal is told only of
        // its own, as when dlss.rs opens a device); None, the device opens
        // as before and DLSS is not offered in VR
        let dlss_extensions = if *with_dlss && dlss_free {
            crate::dlss::xr_device_extensions(&ha)
        } else {
            None
        };
        let with_dlss = dlss_extensions.is_some();
        let mut enabled = device_extensions.clone();
        for name in dlss_extensions.into_iter().flatten() {
            if !enabled.contains(&name) {
                enabled.push(name);
            }
        }
        // SAFETY: physical belongs to raw_instance and is valid until wgpu is dropped.
        let family = unsafe { raw_instance.get_physical_device_queue_family_properties(physical) }
            .iter()
            .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or("No graphics queue")? as u32;
        let create_device = |names: &[&std::ffi::CStr]| -> Result<_> {
            let names: Vec<_> = names.iter().map(|s| s.as_ptr()).collect();
            let mut enabled_features = ha.physical_device_features(&device_extensions, features);
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
            Ok(raw)
        };
        // refused with the DLSS extensions: opened as before, and DLSS is not
        // offered in VR (Preferences says why)
        let (raw, with_dlss) = match create_device(&enabled) {
            Ok(raw) => (raw, with_dlss),
            Err(e) if with_dlss => {
                log::error!("DLSS: the VR runtime refused the device with the DLSS extensions ({e}): opened without them, FSR draws");
                (create_device(&device_extensions)?, false)
            }
            Err(e) => return Err(e),
        };
        log::info!("OpenXR runtime created Vulkan device");
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
        crate::dlss::opened_for_xr(adapter, &result.0, with_dlss);
        Ok(result)
    }
}

fn runtime_class(name: &str) -> &'static str {
    let name = name.to_ascii_lowercase();
    if name.contains("steamvr") || name.contains("steam vr") {
        "SteamVR"
    } else if name.contains("oculus") || name.contains("meta") {
        "Meta"
    } else if name.contains("virtual desktop") || name.contains("vdxr") {
        "Virtual Desktop"
    } else if name.contains("monado") {
        "Monado"
    } else if name.contains("windows mixed reality") || name.contains("microsoft") {
        "Windows Mixed Reality"
    } else {
        "other OpenXR runtime"
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn runtime_names_are_reduced_to_a_closed_public_vocabulary() {
        for (name, expected) in [
            ("SteamVR/OpenXR", "SteamVR"),
            ("Oculus runtime", "Meta"),
            ("Virtual Desktop VDXR", "Virtual Desktop"),
            ("Monado", "Monado"),
            ("Vendor private runtime", "other OpenXR runtime"),
        ] {
            assert_eq!(super::runtime_class(name), expected);
        }
    }
}
