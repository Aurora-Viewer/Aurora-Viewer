//! Linux-only interoperability experiment; no viewer or grid connection.
//! Vulkan bootstrap follows openxrs/examples/vulkan.rs (MIT/Apache-2.0).
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This experiment requires Linux Vulkan/OpenXR.");
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    linux::run()
}

#[cfg(target_os = "linux")]
mod linux {
    use ash::{vk, vk::Handle};
    use openxr as xr;
    use std::{
        error::Error,
        time::{Duration, Instant},
    };
    use wgpu::hal;
    type Result<T> = std::result::Result<T, Box<dyn Error>>;
    const VIEW: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;

    pub fn run() -> Result<()> {
        // SAFETY: The installed OpenXR loader is loaded through openxr's checked entry API.
        let entry = unsafe { xr::Entry::load(&())? };
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
        let flags = wgpu::InstanceFlags::empty();
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
        let adapter = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .find(|a| {
                // SAFETY: Read-only access to Vulkan backend handles.
                unsafe { a.as_hal::<hal::api::Vulkan>().is_some_and(|h| h.raw_physical_device() == physical) }
            })
            .ok_or("Runtime GPU is unavailable to wgpu")?;
        println!("Runtime GPU: {}", adapter.get_info().name);
        let features = wgpu::Features::empty();
        let limits = wgpu::Limits::default();
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
                &Default::default(),
                family,
                0,
            )?
        };
        let (device, queue) = unsafe {
            adapter.create_device_from_hal::<hal::api::Vulkan>(
                hd,
                &wgpu::DeviceDescriptor {
                    required_features: features,
                    required_limits: limits,
                    ..Default::default()
                },
            )?
        };
        drop(ha);
        // Keep the Vulkan instance/device alive across two independent OpenXR sessions.
        for cycle in 1..=2 {
            println!("Cycle {cycle}: creating session; stereo sphere/checker floor for 15 seconds");
            cycle_session(&xr, system, &raw_instance, physical, &raw_device, family, &device, &queue)?;
            println!("Cycle {cycle}: session destroyed; graphics device retained");
            std::thread::sleep(Duration::from_secs(2));
        }
        println!("Two session cycles complete");
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn cycle_session(
        xr: &xr::Instance,
        system: xr::SystemId,
        vi: &ash::Instance,
        physical: vk::PhysicalDevice,
        vd: &ash::Device,
        family: u32,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<()> {
        // SAFETY: Graphics binding uses the same live Vulkan instance/device/queue as wgpu.
        let (session, mut waiter, mut stream) = unsafe {
            xr.create_session::<xr::Vulkan>(
                system,
                &xr::vulkan::SessionCreateInfo {
                    instance: vi.handle().as_raw() as _,
                    physical_device: physical.as_raw() as _,
                    device: vd.handle().as_raw() as _,
                    queue_family_index: family,
                    queue_index: 0,
                },
            )?
        };
        let space = session.create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)?;
        let available = session.enumerate_swapchain_formats()?;
        let candidates = [
            (vk::Format::R8G8B8A8_SRGB, wgpu::TextureFormat::Rgba8UnormSrgb),
            (vk::Format::B8G8R8A8_SRGB, wgpu::TextureFormat::Bgra8UnormSrgb),
        ];
        let (format, wf) = candidates
            .into_iter()
            .find(|(f, _)| available.contains(&(f.as_raw() as u32)))
            .ok_or("No supported sRGB swapchain format")?;
        let views = xr.enumerate_view_configuration_views(system, VIEW)?;
        if views.len() != 2 {
            return Err("Expected two eyes".into());
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
        let mut swapchain = session.create_swapchain(&xr::SwapchainCreateInfo {
            create_flags: xr::SwapchainCreateFlags::EMPTY,
            usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT,
            format: format.as_raw() as _,
            sample_count: 1,
            width,
            height,
            face_count: 1,
            array_size: 2,
            mip_count: 1,
        })?;
        let desc = wgpu::TextureDescriptor {
            label: Some("XR borrowed eye image"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 2,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wf,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let hd = unsafe {
            // SAFETY: Read-only Vulkan device access for importing its own runtime images.
            device.as_hal::<hal::api::Vulkan>()
        }
        .ok_or("Not Vulkan")?;
        let mut textures = Vec::new();
        for image in swapchain.enumerate_images()? {
            let hdesc = hal::TextureDescriptor {
                label: desc.label,
                size: desc.size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: desc.dimension,
                format: wf,
                usage: wgpu::TextureUses::COLOR_TARGET,
                memory_flags: hal::MemoryFlags::empty(),
                view_formats: vec![],
            };
            // SAFETY: Image belongs to this device's XR swapchain, with matching descriptor.
            // External memory and a no-op callback prevent HAL destroying runtime-owned images.
            // Imported textures are dropped before the swapchain and only used while acquired.
            let texture = unsafe {
                hd.texture_from_raw(
                    vk::Image::from_raw(image),
                    &hdesc,
                    Some(Box::new(|| {})),
                    hal::vulkan::TextureMemory::External,
                )
            };
            textures.push(unsafe { device.create_texture_from_hal::<hal::api::Vulkan>(texture, &desc, wgpu::TextureUses::UNINITIALIZED) });
        }
        drop(hd);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("XR stereo ray scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("xr_spike.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("XR prototype"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wf,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let buffers: Vec<_> = (0..2)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Eye camera"),
                    size: 96,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let groups: Vec<_> = buffers
            .iter()
            .map(|b| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: b.as_entire_binding(),
                    }],
                })
            })
            .collect();
        let mut event = xr::EventDataBuffer::new();
        let mut running = false;
        let mut started = None;
        let mut requested = false;
        let deadline = Instant::now() + Duration::from_secs(35);
        let mut frames = 0;
        loop {
            while let Some(e) = xr.poll_event(&mut event)? {
                if let xr::Event::SessionStateChanged(e) = e {
                    println!("Session {:?}", e.state());
                    match e.state() {
                        xr::SessionState::READY => {
                            session.begin(VIEW)?;
                            running = true;
                            started = Some(Instant::now());
                        }
                        xr::SessionState::STOPPING => {
                            session.end()?;
                            running = false;
                        }
                        xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => break,
                        _ => {}
                    }
                }
            }
            if requested && !running {
                break;
            }
            if Instant::now() > deadline {
                return Err("Session did not stop within deadline".into());
            }
            if !running {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            if !requested && started.is_some_and(|s| s.elapsed() >= Duration::from_secs(15)) {
                session.request_exit()?;
                requested = true;
            }
            let frame = waiter.wait()?;
            stream.begin()?;
            if !frame.should_render {
                stream.end(frame.predicted_display_time, xr::EnvironmentBlendMode::OPAQUE, &[])?;
                continue;
            }
            let (flags, eyes) = session.locate_views(VIEW, frame.predicted_display_time, &space)?;
            if !flags.contains(xr::ViewStateFlags::ORIENTATION_VALID | xr::ViewStateFlags::POSITION_VALID) {
                stream.end(frame.predicted_display_time, xr::EnvironmentBlendMode::OPAQUE, &[])?;
                continue;
            }
            let index = swapchain.acquire_image()?;
            swapchain.wait_image(xr::Duration::INFINITE)?;
            let mut encoder = device.create_command_encoder(&Default::default());
            for (eye, view) in eyes.iter().enumerate() {
                let p = view.pose.position;
                let q = view.pose.orientation;
                let transform =
                    glam::Mat4::from_rotation_translation(glam::Quat::from_xyzw(q.x, q.y, q.z, q.w), glam::Vec3::new(p.x, p.y, p.z));
                let mut data = Vec::<f32>::from(transform.to_cols_array());
                data.extend([
                    view.fov.angle_left.tan(),
                    view.fov.angle_right.tan(),
                    view.fov.angle_down.tan(),
                    view.fov.angle_up.tan(),
                ]);
                data.extend([0.0f32; 4]);
                queue.write_buffer(&buffers[eye], 0, bytemuck::cast_slice(&data));
                let target = textures[index as usize].create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: eye as u32,
                    array_layer_count: Some(1),
                    ..Default::default()
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("XR eye"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &groups[eye], &[]);
                pass.draw(0..3, 0..1);
            }
            // Vulkan binding permits release after submission on its bound queue; no CPU/GPU wait.
            queue.submit([encoder.finish()]);
            swapchain.release_image()?;
            let rect = xr::Rect2Di {
                offset: xr::Offset2Di { x: 0, y: 0 },
                extent: xr::Extent2Di {
                    width: width as i32,
                    height: height as i32,
                },
            };
            let layers: Vec<_> = eyes
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    xr::CompositionLayerProjectionView::new().pose(v.pose).fov(v.fov).sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(&swapchain)
                            .image_array_index(i as u32)
                            .image_rect(rect),
                    )
                })
                .collect();
            stream.end(
                frame.predicted_display_time,
                xr::EnvironmentBlendMode::OPAQUE,
                &[&xr::CompositionLayerProjection::new().space(&space).views(&layers)],
            )?;
            frames += 1;
        }
        // One teardown wait is permitted; never wait for device idle in the frame loop.
        device.poll(wgpu::PollType::wait_indefinitely())?;
        drop(groups);
        drop(buffers);
        drop(pipeline);
        drop(textures);
        drop(swapchain);
        println!("Rendered {frames} stereo frames");
        Ok(())
    }
}
