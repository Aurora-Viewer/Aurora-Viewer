//! Shared OpenXR frame ownership; native Vulkan or Metal image import.
use crate::stereo_view::{Anchor, EyeView, Fov, Pose};
#[cfg(not(target_os = "macos"))]
use ash::vk::{self, Handle};
use openxr as xr;
#[cfg(not(target_os = "macos"))]
use wgpu::hal;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[cfg(target_os = "macos")]
type GraphicsApi = crate::xr_device::Metal;
#[cfg(not(target_os = "macos"))]
type GraphicsApi = xr::Vulkan;
const VIEWS: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;
/// Frames in a row the head must hold within `SETTLE_STEP` (metres per
/// frame) before the view is anchored on it (`Session::begin`).
const SETTLE_FRAMES: u32 = 8;
const SETTLE_STEP: f32 = 0.05;
/// After anchoring, a head move over `JUMP` metres in one frame during
/// `WATCH` anchors again: no one moves that fast, a runtime fixing its pose
/// does.
const JUMP: f32 = 0.4;
const WATCH: std::time::Duration = std::time::Duration::from_secs(2);

pub(crate) struct Eyes {
    pub cameras: [EyeView; 2],
    pub textures: [wgpu::Texture; 2],
}
pub(crate) struct Session {
    session: xr::Session<GraphicsApi>,
    waiter: xr::FrameWaiter,
    stream: xr::FrameStream<GraphicsApi>,
    local: xr::Space,
    head: xr::Space,
    swaps: Vec<xr::Swapchain<GraphicsApi>>,
    images: Vec<Vec<wgpu::Texture>>,
    pub width: u32,
    pub height: u32,
    running: bool,
    anchor: Option<Anchor>,
    /// Tracked head positions seen in a row, still within `SETTLE_STEP` of
    /// each other, before anchoring; then, for `WATCH`, the last one and
    /// when the anchor was taken (a jump means the pose it used was a
    /// placeholder).
    settle: (u32, Option<glam::Vec3>),
    anchored: Option<(std::time::Instant, glam::Vec3)>,
    frame: Option<(xr::Time, Vec<xr::View>)>,
    started: Option<std::time::Instant>,
    requested_stop: bool,
    stop_wanted: bool,
    /// The eyes drawn in the viewer's format when the runtime's images are
    /// in its sRGB twin; copied into those images at `end`.
    staging: Option<[wgpu::Texture; 2]>,
    /// The swapchain image acquired for each eye in `begin`.
    acquired: [usize; 2],
    device: wgpu::Device,
    queue: wgpu::Queue,
}
impl Session {
    pub fn new(
        instance: &xr::Instance,
        system: xr::SystemId,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        #[cfg(not(target_os = "macos"))]
        let (session, waiter, stream) = {
            // SAFETY: Borrow live Vulkan handles; session retains the device for its full lifetime.
            let h = unsafe { device.as_hal::<hal::api::Vulkan>() }.ok_or("Not a Vulkan device")?;
            let vi = h.shared_instance().raw_instance();
            // SAFETY: The handles and queue are from this runtime-compatible device.
            let (session, waiter, stream) = unsafe {
                instance
                    .create_session::<xr::Vulkan>(
                        system,
                        &xr::vulkan::SessionCreateInfo {
                            instance: vi.handle().as_raw() as _,
                            physical_device: h.raw_physical_device().as_raw() as _,
                            device: h.raw_device().handle().as_raw() as _,
                            queue_family_index: h.queue_family_index(),
                            queue_index: h.queue_index(),
                        },
                    )
                    .map_err(|e| format!("xrCreateSession: {e}"))?
            };
            (session, waiter, stream)
        };
        #[cfg(target_os = "macos")]
        let (session, waiter, stream) = crate::xr_device::create_session(instance, system, queue)?;
        // The eyes are drawn in the viewer's output format. A runtime that
        // offers only its sRGB twin, or the reverse (SteamVR on Linux with a
        // Vive Pro, 2026-11-01, had no B8G8R8A8_UNORM), gets the same bytes
        // copied into its images each frame: the same layout, already encoded
        // for the display, so the picture is unchanged.
        let offered = session.enumerate_swapchain_formats()?;
        let twin = if format.is_srgb() {
            format.remove_srgb_suffix()
        } else {
            format.add_srgb_suffix()
        };
        let eye_format = if offered.contains(&raw_format(format)?) {
            format
        } else if twin != format && offered.contains(&raw_format(twin)?) {
            log::info!("XR world: the runtime has no {format:?} images, drawing in {format:?} and copying into {twin:?}");
            twin
        } else {
            log::warn!("XR world: the runtime offers only these image formats: {offered:?}");
            return Err("Runtime does not support the viewer output format".into());
        };
        let format_raw = raw_format(eye_format)?;
        let copied = eye_format != format;
        let views = instance.enumerate_view_configuration_views(system, VIEWS)?;
        if views.len() != 2 {
            return Err("Expected stereo views".into());
        }
        let width = views
            .iter()
            .map(|v| v.recommended_image_rect_width)
            .max()
            .ok_or("No views")?
            .min(1600);
        let height = views
            .iter()
            .map(|v| v.recommended_image_rect_height)
            .max()
            .ok_or("No views")?
            .min(1600);
        let local = session.create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)?;
        let head = session.create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)?;
        let mut swap_usage =
            xr::SwapchainUsageFlags::COLOR_ATTACHMENT | xr::SwapchainUsageFlags::TRANSFER_SRC | xr::SwapchainUsageFlags::SAMPLED;
        let mut eye_usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING;
        if copied {
            swap_usage |= xr::SwapchainUsageFlags::TRANSFER_DST;
            eye_usage |= wgpu::TextureUsages::COPY_DST;
        }
        let mut swaps = Vec::new();
        let mut images = Vec::new();
        for _ in 0..2 {
            let swap = session.create_swapchain(&xr::SwapchainCreateInfo {
                create_flags: xr::SwapchainCreateFlags::EMPTY,
                usage_flags: swap_usage,
                format: format_raw,
                sample_count: 1,
                width,
                height,
                face_count: 1,
                array_size: 1,
                mip_count: 1,
            })?;
            let desc = wgpu::TextureDescriptor {
                label: Some("XR world eye"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: eye_format,
                usage: eye_usage,
                view_formats: &[],
            };
            let mut imported = Vec::new();
            for image in swap.enumerate_images()? {
                #[cfg(not(target_os = "macos"))]
                {
                    // SAFETY: The session retains the live compatible Vulkan device.
                    let h = unsafe { device.as_hal::<hal::api::Vulkan>() }.ok_or("Not Vulkan")?;
                    let hd = hal::TextureDescriptor {
                        label: desc.label,
                        size: desc.size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: desc.dimension,
                        format: eye_format,
                        usage: wgpu::TextureUses::COLOR_TARGET
                            | wgpu::TextureUses::COPY_SRC
                            | wgpu::TextureUses::RESOURCE
                            | if copied {
                                wgpu::TextureUses::COPY_DST
                            } else {
                                wgpu::TextureUses::empty()
                            },
                        memory_flags: hal::MemoryFlags::empty(),
                        view_formats: vec![],
                    };
                    // SAFETY: This runtime-owned image belongs to the bound device and matches hd.
                    // External memory and no-op destruction retain ownership in OpenXR. The
                    // imported wrappers are destroyed before the swapchains in Drop below.
                    let raw = unsafe {
                        h.texture_from_raw(
                            vk::Image::from_raw(image),
                            &hd,
                            Some(Box::new(|| {})),
                            hal::vulkan::TextureMemory::External,
                        )
                    };
                    // SAFETY: Raw image and descriptor agree; initial content may be discarded.
                    imported
                        .push(unsafe { device.create_texture_from_hal::<hal::api::Vulkan>(raw, &desc, wgpu::TextureUses::UNINITIALIZED) });
                }
                #[cfg(target_os = "macos")]
                imported.push(crate::xr_device::import_texture(device, image, &desc)?);
            }
            swaps.push(swap);
            images.push(imported);
        }
        let staging = copied.then(|| {
            let desc = wgpu::TextureDescriptor {
                label: Some("XR world eye, copied to the runtime"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            };
            [device.create_texture(&desc), device.create_texture(&desc)]
        });
        Ok(Self {
            session,
            waiter,
            stream,
            local,
            head,
            swaps,
            images,
            width,
            height,
            running: false,
            anchor: None,
            settle: (0, None),
            anchored: None,
            frame: None,
            started: None,
            requested_stop: false,
            stop_wanted: false,
            staging,
            acquired: [0; 2],
            device: device.clone(),
            queue: queue.clone(),
        })
    }
    pub fn request_stop(&mut self) {
        self.stop_wanted = true;
    }

    pub fn begin(&mut self, instance: &xr::Instance, desktop_view: glam::Mat4, near: f32) -> Result<Option<Eyes>> {
        let mut event_buffer = xr::EventDataBuffer::new();
        while let Some(event) = instance.poll_event(&mut event_buffer)? {
            if let xr::Event::SessionStateChanged(e) = event {
                // a session dropped while being set up leaves its own events
                // in the instance's queue; its READY would begin this one a
                // second time (XR_ERROR_SESSION_RUNNING)
                if e.session() != self.session.as_raw() {
                    continue;
                }
                log::info!("XR world session: {:?}", e.state());
                match e.state() {
                    xr::SessionState::READY => {
                        self.session.begin(VIEWS)?;
                        self.running = true;
                        self.started = Some(std::time::Instant::now());
                    }
                    xr::SessionState::STOPPING => {
                        self.session.end()?;
                        self.running = false;
                    }
                    xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => return Err("XR session finished".into()),
                    _ => {}
                }
            } else if let xr::Event::ReferenceSpaceChangePending(e) = event
                && e.session() == self.session.as_raw()
                && e.reference_space_type() == xr::ReferenceSpaceType::LOCAL
            {
                // the runtime recentred the space the head is located in
                // (Virtual Desktop does it once at the start of its first
                // session): anchor again, or the old origin's height stays
                // between the eyes and the avatar's
                log::info!("XR world: the runtime recentred the local space, anchoring again");
                self.anchor = None;
                self.settle = (0, None);
                self.anchored = None;
            }
        }
        if !self.running {
            return Ok(None);
        }
        if !self.requested_stop
            && (self.stop_wanted
                || (std::env::var_os("AURORA_DEMO_XR_WORLD").is_some() && self.started.is_some_and(|s| s.elapsed().as_secs() >= 15)))
        {
            self.session.request_exit()?;
            self.requested_stop = true;
        }
        let frame = self.waiter.wait()?;
        self.stream.begin()?;
        let (flags, views) = self.session.locate_views(VIEWS, frame.predicted_display_time, &self.local)?;
        if !frame.should_render
            || views.len() != 2
            || !flags.contains(xr::ViewStateFlags::POSITION_VALID | xr::ViewStateFlags::ORIENTATION_VALID)
        {
            self.stream
                .end(frame.predicted_display_time, xr::EnvironmentBlendMode::OPAQUE, &[])?;
            return Ok(None);
        }
        // Virtual Desktop's first frames can report a tracked head at a
        // placeholder place (0, -1.64, 0 on 2026-11-01), then the real one:
        // anchored on the placeholder, the view floated above the avatar.
        // The head must hold still for a few frames before anchoring, and a
        // jump soon after anchors again.
        if let Some((at, last)) = self.anchored {
            if at.elapsed() < WATCH {
                let head = self.head.locate(&self.local, frame.predicted_display_time)?;
                let p = head.pose.position;
                let now = glam::Vec3::new(p.x, p.y, p.z);
                if now.distance(last) > JUMP {
                    log::info!(
                        "XR world: the head jumped {:.2} m after anchoring, anchoring again",
                        now.distance(last)
                    );
                    self.anchor = None;
                    self.settle = (0, None);
                    self.anchored = None;
                } else {
                    self.anchored = Some((at, now));
                }
            } else {
                self.anchored = None;
            }
        }
        if self.anchor.is_none() {
            let head = self.head.locate(&self.local, frame.predicted_display_time)?;
            // tracked, not only valid: a first session's pose can be a
            // runtime's placeholder until the headset reports itself
            if !head.location_flags.contains(
                xr::SpaceLocationFlags::POSITION_VALID
                    | xr::SpaceLocationFlags::ORIENTATION_VALID
                    | xr::SpaceLocationFlags::POSITION_TRACKED
                    | xr::SpaceLocationFlags::ORIENTATION_TRACKED,
            ) {
                self.settle = (0, None);
                self.stream
                    .end(frame.predicted_display_time, xr::EnvironmentBlendMode::OPAQUE, &[])?;
                return Ok(None);
            }
            let p = head.pose.position;
            let now = glam::Vec3::new(p.x, p.y, p.z);
            let still = self.settle.1.is_some_and(|last| now.distance(last) <= SETTLE_STEP);
            self.settle = (if still { self.settle.0 + 1 } else { 0 }, Some(now));
            if self.settle.0 < SETTLE_FRAMES {
                self.stream
                    .end(frame.predicted_display_time, xr::EnvironmentBlendMode::OPAQUE, &[])?;
                return Ok(None);
            }
            self.anchored = Some((std::time::Instant::now(), now));
            self.anchor = Anchor::new(desktop_view, pose(head.pose));
            let at = desktop_view.inverse().w_axis.truncate();
            let h = head.pose.position;
            log::info!(
                "XR world anchored at the desktop view {:.2} {:.2} {:.2}, head at {:.2} {:.2} {:.2} in the local space",
                at.x,
                at.y,
                at.z,
                h.x,
                h.y,
                h.z
            );
        }
        let anchor = self.anchor.ok_or("Invalid desktop anchor")?;
        let camera = |i: usize| -> Result<EyeView> {
            let v = views[i];
            anchor
                .eye(
                    pose(v.pose),
                    Fov {
                        left: v.fov.angle_left,
                        right: v.fov.angle_right,
                        down: v.fov.angle_down,
                        up: v.fov.angle_up,
                    },
                    near,
                )
                .ok_or_else(|| "Invalid runtime eye pose".into())
        };
        let cameras = [camera(0)?, camera(1)?];
        let mut output = Vec::new();
        for i in 0..2 {
            let index = self.swaps[i].acquire_image()? as usize;
            self.swaps[i].wait_image(xr::Duration::INFINITE)?;
            self.acquired[i] = index;
            output.push(match &self.staging {
                Some(staging) => staging[i].clone(),
                None => self.images[i][index].clone(),
            });
        }
        self.frame = Some((frame.predicted_display_time, views));
        Ok(Some(Eyes {
            cameras,
            textures: [output.remove(0), output.remove(0)],
        }))
    }
    pub fn end(&mut self) -> Result<()> {
        let (time, views) = self.frame.take().ok_or("No acquired XR frame")?;
        if let Some(staging) = &self.staging {
            // after the eyes' drawing on the same queue, before the release
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("XR world eye copy"),
            });
            for (i, eye) in staging.iter().enumerate() {
                encoder.copy_texture_to_texture(
                    eye.as_image_copy(),
                    self.images[i][self.acquired[i]].as_image_copy(),
                    wgpu::Extent3d {
                        width: self.width,
                        height: self.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            // OpenXR takes the image back in the colour attachment layout; the
            // copy left it as a transfer destination. An empty pass that keeps
            // its content moves it there.
            for (i, eyes) in self.images.iter().enumerate() {
                let view = eyes[self.acquired[i]].create_view(&wgpu::TextureViewDescriptor::default());
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("XR world eye layout"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            }
            self.queue.submit([encoder.finish()]);
        }
        for swap in &mut self.swaps {
            swap.release_image()?;
        }
        let rect = xr::Rect2Di {
            offset: xr::Offset2Di { x: 0, y: 0 },
            extent: xr::Extent2Di {
                width: self.width as i32,
                height: self.height as i32,
            },
        };
        let layers: Vec<_> = views
            .iter()
            .enumerate()
            .map(|(i, v)| {
                xr::CompositionLayerProjectionView::new().pose(v.pose).fov(v.fov).sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&self.swaps[i])
                        .image_array_index(0)
                        .image_rect(rect),
                )
            })
            .collect();
        self.stream.end(
            time,
            xr::EnvironmentBlendMode::OPAQUE,
            &[&xr::CompositionLayerProjection::new().space(&self.local).views(&layers)],
        )?;
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        // Teardown only: keep runtime-owned images alive until submitted rendering completes.
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        self.images.clear();
    }
}
/// The runtime's image format code for an eye format.
#[cfg(not(target_os = "macos"))]
fn raw_format(format: wgpu::TextureFormat) -> Result<u32> {
    let raw = match format {
        wgpu::TextureFormat::Bgra8Unorm => vk::Format::B8G8R8A8_UNORM,
        wgpu::TextureFormat::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
        wgpu::TextureFormat::Bgra8UnormSrgb => vk::Format::B8G8R8A8_SRGB,
        wgpu::TextureFormat::Rgba8UnormSrgb => vk::Format::R8G8B8A8_SRGB,
        _ => return Err("Unsupported eye output format".into()),
    };
    Ok(raw.as_raw() as u32)
}
#[cfg(target_os = "macos")]
fn raw_format(format: wgpu::TextureFormat) -> Result<u32> {
    crate::xr_device::image_format(format)
}
fn pose(p: xr::Posef) -> Pose {
    Pose {
        position: glam::Vec3::new(p.position.x, p.position.y, p.position.z),
        orientation: glam::Quat::from_xyzw(p.orientation.x, p.orientation.y, p.orientation.z, p.orientation.w),
    }
}
