//! Native Metal/OpenXR binding (XR_KHR_metal_enable), sharing the Vulkan frame loop.
//! openxr 0.22 exposes the raw extension but has no typed Metal Graphics adapter.
//! Runtime images and the application's exact Metal command queue stay GPU-owned.
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLPixelFormat, MTLTexture, MTLTextureType};
use openxr as xr;
use std::{ffi::c_void, ptr};
use wgpu::hal;
use xr::sys::Handle as _;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(crate) struct Graphics {
    pub instance: wgpu::Instance,
    xr: xr::Instance,
    system: xr::SystemId,
    // Runtime owns this device while xr is alive; only compare its identity.
    metal_device: usize,
}
impl Graphics {
    pub fn new(flags: wgpu::InstanceFlags) -> Result<Self> {
        // SAFETY: The installed Khronos loader dispatches the configured runtime.
        let entry = if let Some(path) = std::env::var_os("AURORA_OPENXR_LOADER") {
            // SAFETY: Explicitly configured native loader; the caller installs it.
            unsafe { xr::Entry::load_from(std::path::Path::new(&path), &())? }
        } else {
            // SAFETY: Standard platform loader lookup, with desktop fallback on failure.
            unsafe { xr::Entry::load(&())? }
        };
        if !entry.enumerate_extensions()?.khr_metal_enable {
            return Err("XR_KHR_metal_enable unavailable".into());
        }
        let xr = entry.create_instance(
            &xr::ApplicationInfo {
                application_name: "Aurora Metal interoperability",
                application_version: 0,
                engine_name: "Aurora",
                engine_version: 0,
                api_version: xr::Version::new(1, 0, 0),
            },
            &{
                // a non-exhaustive struct: set the one extension on a default
                let mut extensions = xr::ExtensionSet::default();
                extensions.khr_metal_enable = true;
                extensions
            },
            &[],
            &(),
        )?;
        let system = xr.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)?;
        let metal_device = xr.graphics_requirements::<Metal>(system)?;
        if metal_device == 0 {
            return Err("Runtime returned a null Metal device".into());
        }
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::METAL;
        desc.flags = flags;
        Ok(Self {
            instance: wgpu::Instance::new(desc),
            xr,
            system,
            metal_device,
        })
    }
    pub fn binding(&self) -> (xr::Instance, xr::SystemId) {
        (self.xr.clone(), self.system)
    }
    pub fn open(&self, adapter: &wgpu::Adapter, desc: &wgpu::DeviceDescriptor<'_>) -> Result<(wgpu::Device, wgpu::Queue)> {
        // SAFETY: Read-only identity check while the adapter and runtime remain live.
        let raw = unsafe { adapter.as_hal::<hal::api::Metal>() }.ok_or("Not a Metal adapter")?;
        if ptr::from_ref(raw.raw_device()).cast::<c_void>() as usize != self.metal_device {
            return Err("Viewer GPU differs from the runtime-selected Metal GPU".into());
        }
        drop(raw);
        Ok(pollster::block_on(adapter.request_device(desc))?)
    }
}

pub(crate) enum Metal {}
fn checked(result: xr::sys::Result) -> xr::Result<()> {
    if result.into_raw() < 0 { Err(result) } else { Ok(()) }
}
impl xr::Graphics for Metal {
    type Requirements = usize;
    type SessionCreateInfo = *mut c_void;
    type Format = u32;
    type SwapchainImage = *mut c_void;
    fn raise_format(value: i64) -> u32 {
        value as u32
    }
    fn lower_format(value: u32) -> i64 {
        i64::from(value)
    }
    fn requirements(instance: &xr::Instance, system: xr::SystemId) -> xr::Result<usize> {
        let extension = instance
            .exts()
            .khr_metal_enable
            .as_ref()
            .ok_or(xr::sys::Result::ERROR_EXTENSION_NOT_PRESENT)?;
        // SAFETY: Correct extension output header and live instance/system handles.
        unsafe {
            let mut requirements = xr::sys::GraphicsRequirementsMetalKHR::out(ptr::null_mut());
            checked((extension.get_metal_graphics_requirements)(
                instance.as_raw(),
                system,
                requirements.as_mut_ptr(),
            ))?;
            Ok(requirements.assume_init().metal_device as usize)
        }
    }
    unsafe fn create_session(instance: &xr::Instance, system: xr::SystemId, queue: &*mut c_void) -> xr::Result<xr::sys::Session> {
        let binding = xr::sys::GraphicsBindingMetalKHR {
            ty: xr::sys::GraphicsBindingMetalKHR::TYPE,
            next: ptr::null(),
            command_queue: *queue,
        };
        let info = xr::sys::SessionCreateInfo {
            ty: xr::sys::SessionCreateInfo::TYPE,
            next: ptr::from_ref(&binding).cast(),
            create_flags: Default::default(),
            system_id: system,
        };
        let mut session = xr::sys::Session::NULL;
        // SAFETY: Caller supplies a live queue on the runtime-required device and
        // retains it until the session is destroyed. The binding lives through this call.
        checked(unsafe { (instance.fp().create_session)(instance.as_raw(), &info, &mut session) })?;
        Ok(session)
    }
    fn enumerate_swapchain_images(swapchain: &xr::Swapchain<Self>) -> xr::Result<Vec<*mut c_void>> {
        let enumerate = swapchain.instance().fp().enumerate_swapchain_images;
        let mut count = 0;
        // SAFETY: Query only; count is writable, no image storage is supplied.
        checked(unsafe { enumerate(swapchain.as_raw(), 0, &mut count, ptr::null_mut()) })?;
        loop {
            let capacity = count;
            let mut images = vec![
                xr::sys::SwapchainImageMetalKHR {
                    ty: xr::sys::SwapchainImageMetalKHR::TYPE,
                    next: ptr::null_mut(),
                    texture: ptr::null_mut(),
                };
                capacity as usize
            ];
            // SAFETY: Every output has the Metal header, and capacity matches storage.
            let result = unsafe { enumerate(swapchain.as_raw(), capacity, &mut count, images.as_mut_ptr().cast()) };
            if result == xr::sys::Result::ERROR_SIZE_INSUFFICIENT {
                continue;
            }
            checked(result)?;
            images.truncate(count as usize);
            return Ok(images.into_iter().map(|image| image.texture).collect());
        }
    }
}

pub(crate) fn create_session(
    instance: &xr::Instance,
    system: xr::SystemId,
    queue: &wgpu::Queue,
) -> Result<(xr::Session<Metal>, xr::FrameWaiter, xr::FrameStream<Metal>)> {
    // SAFETY: Read the exact command queue wgpu submits eye rendering to.
    let raw = unsafe { queue.as_hal::<hal::api::Metal>() }.ok_or("Not a Metal queue")?;
    let command_queue = ptr::from_ref(raw.as_raw()).cast_mut().cast::<c_void>();
    // SAFETY: Session retains a wgpu queue clone; its device matches requirements.
    // Sharing the same Metal queue gives the runtime ordered access to submitted images.
    Ok(unsafe { instance.create_session::<Metal>(system, &command_queue)? })
}

pub(crate) fn image_format(format: wgpu::TextureFormat) -> Result<u32> {
    let raw = match format {
        wgpu::TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb => MTLPixelFormat::BGRA8Unorm_sRGB,
        wgpu::TextureFormat::Rgba8Unorm => MTLPixelFormat::RGBA8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb => MTLPixelFormat::RGBA8Unorm_sRGB,
        _ => return Err("Unsupported Metal eye output format".into()),
    };
    Ok(raw.0 as u32)
}

pub(crate) fn import_texture(device: &wgpu::Device, image: *mut c_void, desc: &wgpu::TextureDescriptor<'_>) -> Result<wgpu::Texture> {
    // SAFETY: OpenXR supplies a live MTLTexture on the bound device. Retain our own
    // reference until wgpu finishes rendering; the session still owns the swapchain.
    let texture = unsafe { Retained::retain(image.cast::<ProtocolObject<dyn MTLTexture>>()) }.ok_or("Null Metal eye texture")?;
    // SAFETY: Runtime image matches the 2D, one-layer, one-mip swapchain descriptor.
    let raw = unsafe { hal::metal::Device::texture_from_raw(texture, desc.format, MTLTextureType::Type2D, 1, 1, desc.size.into(), None) };
    // SAFETY: Imported image and descriptor agree; no content is preserved on acquisition.
    Ok(unsafe { device.create_texture_from_hal::<hal::api::Metal>(raw, desc, wgpu::TextureUses::UNINITIALIZED) })
}
