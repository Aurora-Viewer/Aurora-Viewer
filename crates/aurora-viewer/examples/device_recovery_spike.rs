//! Local experiment for the GPU device recovery (not committed): after a real
//! GPU reset, can wgpu 30 open a new device and present again in the same
//! window, and which way of getting the surface back does the driver accept?
//!
//! cargo run -p aurora-viewer --example device_recovery_spike -- \
//!     [same|new-surface|new-instance] [--hang-at <frames>] [--hangs <n>]
//!
//! It draws a slow colour cycle. `H` (or `--hang-at`) submits one frame whose
//! fragment shader runs about forever: the driver's watchdog resets the GPU
//! (Windows TDR after ~2 s, amdgpu's lockup timeout on Linux), as in the crash
//! reports. The old device is then never touched again (any wait on it
//! panics in wgpu 30), a new one is opened on the same adapter, and the
//! surface comes back the chosen way. `Esc` quits; the result line says how
//! long the recovery took and how many frames were presented after it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const SHADER: &str = r#"
struct Params { iterations: u32, t: f32 }
@group(0) @binding(0) var<uniform> p: Params;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    var x = pos.x * 0.001 + p.t;
    for (var i = 0u; i < p.iterations; i++) { x = fract(sin(x) * 43758.5453 + 0.1); }
    return vec4f(fract(p.t * 0.25), 0.2 + x * 0.1, 0.5, 1.0);
}
"#;

static LOG: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Every line goes to the log file too: a reset can take the terminal down.
fn say(line: &str) {
    use std::io::Write;
    let line = format!("[{:8.3}] {line}", START.get_or_init(Instant::now).elapsed().as_secs_f64());
    println!("{line}");
    if let Ok(mut log) = LOG.lock()
        && let Some(file) = log.as_mut()
    {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Strategy {
    Same,
    NewSurface,
    NewInstance,
    /// A new window too: the abandoned swapchain goes with the old one.
    NewWindow,
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    group: wgpu::BindGroup,
}

struct Spike {
    strategy: Strategy,
    hang_at: Option<u64>,
    hangs_left: u32,
    start: Instant,
    window: Option<Arc<Window>>,
    instance: Option<wgpu::Instance>,
    adapter: Option<wgpu::Adapter>,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    gpu: Option<Gpu>,
    lost: Arc<AtomicBool>,
    hang_now: bool,
    frames: u64,
    lost_at: Option<Instant>,
    recoveries: Vec<String>,
    last_status: &'static str,
    bad_frames: u32,
    present_mode: wgpu::PresentMode,
    wait_in_hang: bool,
    poll_after_loss: bool,
    raw_surface: bool,
    frames_after: u64,
    hang_secs: f64,
    hang_iterations: u32,
}

impl Spike {
    fn log(&self, line: impl AsRef<str>) {
        say(line.as_ref());
    }

    fn new_instance() -> wgpu::Instance {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::VULKAN;
        wgpu::Instance::new(desc)
    }

    fn pick_adapter(&self, instance: &wgpu::Instance, surface: &wgpu::Surface<'static>) -> Option<wgpu::Adapter> {
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
        let wanted = self.adapter.as_ref().map(|a| a.get_info().name);
        adapters
            .into_iter()
            .filter(|a| a.is_surface_supported(surface))
            .find(|a| wanted.as_ref().is_none_or(|name| &a.get_info().name == name))
    }

    fn open(&self, adapter: &wgpu::Adapter) -> Result<Gpu, wgpu::RequestDeviceError> {
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("recovery spike"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))?;
        let errors = Arc::new(std::sync::atomic::AtomicU64::new(0));
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            let n = errors.fetch_add(1, Ordering::Relaxed) + 1;
            if n <= 3 || n.is_multiple_of(100_000) {
                say(&format!("wgpu error #{n}: {e}"));
            }
        }));
        let lost = self.lost.clone();
        device.set_device_lost_callback(move |reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                say(&format!("device lost callback: {reason:?}: {message}"));
                lost.store(true, Ordering::Release);
            }
        });
        let format = self.config.as_ref().map_or(wgpu::TextureFormat::Bgra8UnormSrgb, |c| c.format);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spike"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spike"),
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
                    format,
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
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("params"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }],
        });
        Ok(Gpu {
            device,
            queue,
            pipeline,
            params,
            group,
        })
    }

    /// The old device is lost: forget it (a drop could wait on it), open a
    /// new one, bring the surface back the chosen way.
    fn recover(&mut self, event_loop: &ActiveEventLoop) {
        let lost_at = self.lost_at.unwrap_or_else(Instant::now);
        if self.poll_after_loss
            && let Some(gpu) = &self.gpu
        {
            // The case the renderer's guard is for: a poll that is the first
            // to meet a device the driver already lost.
            self.log("polling the lost device once (device.poll Wait)");
            let before = std::panic::take_hook();
            std::panic::set_hook(Box::new(|info| say(&format!("caught wgpu panic: {info}"))));
            let device = gpu.device.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                device.poll(wgpu::PollType::wait_indefinitely())
            }));
            std::panic::set_hook(before);
            match result {
                Ok(r) => self.log(format!("poll returned: {r:?}")),
                Err(_) => self.log("poll panicked: caught, going on"),
            }
        }
        std::mem::forget(self.gpu.take());
        self.log(format!("recovering ({:?})", self.strategy));
        if self.strategy == Strategy::NewWindow {
            // The surface first, then the window it was made for.
            std::mem::forget(self.surface.take());
            let old = self.window.take().expect("window");
            let size = old.inner_size();
            if !self.raw_surface {
                self.log("(the abandoned surface still holds the old window: it stays open)");
            }
            drop(old);
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Aurora device recovery spike (recovered)")
                            .with_inner_size(size),
                    )
                    .expect("new window"),
            );
            self.log(format!("new window {}x{}", size.width, size.height));
            self.window = Some(window);
        }
        let window = self.window.clone().expect("window");
        let mut strategy = self.strategy;
        let mut tries = 0;
        let gpu = loop {
            tries += 1;
            if strategy == Strategy::NewSurface || strategy == Strategy::NewInstance {
                std::mem::forget(self.surface.take());
            }
            if strategy == Strategy::NewInstance {
                std::mem::forget(self.adapter.take());
                std::mem::forget(self.instance.take());
                self.instance = Some(Self::new_instance());
            }
            if self.surface.is_none() {
                let instance = self.instance.as_ref().expect("instance");
                match make_surface(instance, &window, self.raw_surface) {
                    Ok(surface) => self.surface = Some(surface),
                    Err(e) => {
                        self.log(format!("  try {tries}: create_surface failed: {e}"));
                        std::thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                }
            }
            if strategy == Strategy::NewInstance {
                let instance = self.instance.clone().expect("instance");
                let surface = self.surface.as_ref().expect("surface");
                match self.pick_adapter(&instance, surface) {
                    Some(adapter) => self.adapter = Some(adapter),
                    None => {
                        self.log(format!("  try {tries}: no adapter yet"));
                        std::thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                }
            }
            let adapter = self.adapter.clone().expect("adapter");
            match self.open(&adapter) {
                Ok(gpu) => break gpu,
                Err(e) => {
                    self.log(format!("  try {tries}: request_device failed: {e}"));
                    if tries == 6 && strategy != Strategy::NewInstance {
                        self.log("  falling back to a new instance");
                        strategy = Strategy::NewInstance;
                    }
                    if tries > 60 {
                        self.log("RESULT: gave up after 60 tries");
                        std::process::exit(2);
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        };
        let config = self.config.clone().expect("config");
        self.surface.as_ref().expect("surface").configure(&gpu.device, &config);
        self.lost.store(false, Ordering::Release);
        self.gpu = Some(gpu);
        let took = lost_at.elapsed();
        self.log(format!(
            "new device open and surface configured after {} ms ({tries} tries)",
            took.as_millis()
        ));
        self.recoveries.push(format!("{:?} in {} ms", strategy, took.as_millis()));
        self.lost_at = None;
        self.frames = 0;
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        if self.lost.load(Ordering::Acquire) {
            self.lost_at.get_or_insert_with(Instant::now);
            self.recover(event_loop);
            return;
        }
        let Some(gpu) = &self.gpu else { return };
        let surface = self.surface.as_ref().expect("surface");
        let texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                let status = short(&other);
                self.bad_frames += 1;
                if !self.recoveries.is_empty() && self.bad_frames == 300 {
                    self.log(format!(
                        "RESULT: FAILED: {status} for 300 frames after recovering ({})",
                        self.recoveries.join(", ")
                    ));
                    std::process::exit(3);
                }
                if self.last_status != status {
                    self.log(format!("get_current_texture: {status}"));
                    self.last_status = status;
                }
                if matches!(other, wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated) {
                    let config = self.config.clone().expect("config");
                    surface.configure(&gpu.device, &config);
                }
                return;
            }
        };
        self.bad_frames = 0;
        if self.last_status != "Success" {
            self.log(format!("get_current_texture: Success again after {}", self.last_status));
            self.last_status = "Success";
        }
        self.frames += 1;
        let hang = std::mem::take(&mut self.hang_now) || (self.hangs_left > 0 && self.hang_at == Some(self.frames));
        let iterations: u32 = if hang { self.hang_iterations } else { 1 };
        if hang {
            self.hangs_left = self.hangs_left.saturating_sub(1);
            self.lost_at = Some(Instant::now());
            self.log(format!(
                "submitting the hanging frame ({} iterations a pixel, about {} s of GPU work)",
                self.hang_iterations, self.hang_secs
            ));
        }
        let t = self.start.elapsed().as_secs_f32();
        let mut bytes = [0u8; 16];
        bytes[..4].copy_from_slice(&iterations.to_le_bytes());
        bytes[4..8].copy_from_slice(&t.to_le_bytes());
        gpu.queue.write_buffer(&gpu.params, 0, &bytes);
        let view = texture.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spike"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&gpu.pipeline);
            pass.set_bind_group(0, &gpu.group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.queue.present(texture);
        if hang && self.wait_in_hang {
            // As the renderer's frame pacing does: wait on the GPU while the
            // loss lands. wgpu 30 panics then (handle_error_fatal); caught as
            // aurora-render's poll_guarded does.
            self.log("waiting on the hung GPU (device.poll Wait)");
            let before = std::panic::take_hook();
            std::panic::set_hook(Box::new(|info| say(&format!("caught wgpu panic: {info}"))));
            let device = gpu.device.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                device.poll(wgpu::PollType::wait_indefinitely())
            }));
            std::panic::set_hook(before);
            match result {
                Ok(r) => self.log(format!("wait returned: {r:?}")),
                Err(_) => {
                    self.log("wait panicked: taken as the loss");
                    self.lost.store(true, Ordering::Release);
                }
            }
        }
        if !self.recoveries.is_empty() && self.frames == self.frames_after && self.hangs_left == 0 {
            self.log(format!(
                "RESULT: recovered {} time(s): {}; {} frames presented after the last one",
                self.recoveries.len(),
                self.recoveries.join(", "),
                self.frames_after,
            ));
            event_loop.exit();
        }
    }
}

/// One frame of a known number of iterations, timed, before any hang: the
/// hang's iterations for about `secs` seconds on this GPU and window size.
fn calibrate(gpu: &Gpu, config: &wgpu::SurfaceConfiguration, secs: f64) -> u32 {
    const PROBE: u32 = 20_000;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("calibration"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut bytes = [0u8; 16];
    bytes[..4].copy_from_slice(&PROBE.to_le_bytes());
    gpu.queue.write_buffer(&gpu.params, 0, &bytes);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("calibration"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&gpu.pipeline);
        pass.set_bind_group(0, &gpu.group, &[]);
        pass.draw(0..3, 0..1);
    }
    let started = Instant::now();
    gpu.queue.submit([encoder.finish()]);
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let took = started.elapsed().as_secs_f64().max(1e-4);
    let iterations = (secs / took * f64::from(PROBE)).min(f64::from(u32::MAX)) as u32;
    say(&format!(
        "calibration: {PROBE} iterations a pixel took {:.1} ms; the hang uses {iterations} (~{secs} s)",
        took * 1000.0
    ));
    iterations
}

/// The window's surface. With `raw`, made from its raw handles, so that the
/// surface does not keep the window alive: an abandoned surface then lets the
/// old window close (it holds an `Arc` of it otherwise).
fn make_surface(instance: &wgpu::Instance, window: &Arc<Window>, raw: bool) -> Result<wgpu::Surface<'static>, String> {
    if !raw {
        return instance.create_surface(window.clone()).map_err(|e| e.to_string());
    }
    // SAFETY: experiment only. The window must outlive the surface; it does
    // while the surface is used, and an abandoned surface is never touched
    // again after its window closes.
    unsafe {
        let target = wgpu::SurfaceTargetUnsafe::from_display_and_window(&**window, &**window).map_err(|e| e.to_string())?;
        instance.create_surface_unsafe(target).map_err(|e| e.to_string())
    }
}

fn short(t: &wgpu::CurrentSurfaceTexture) -> &'static str {
    match t {
        wgpu::CurrentSurfaceTexture::Success(_) => "Success",
        wgpu::CurrentSurfaceTexture::Suboptimal(_) => "Suboptimal",
        wgpu::CurrentSurfaceTexture::Timeout => "Timeout",
        wgpu::CurrentSurfaceTexture::Occluded => "Occluded",
        wgpu::CurrentSurfaceTexture::Outdated => "Outdated",
        wgpu::CurrentSurfaceTexture::Lost => "Lost",
        wgpu::CurrentSurfaceTexture::Validation => "Validation",
    }
}

impl ApplicationHandler for Spike {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Aurora device recovery spike")
                        .with_inner_size(winit::dpi::LogicalSize::new(800, 600)),
                )
                .expect("window"),
        );
        let instance = Self::new_instance();
        let surface = make_surface(&instance, &window, self.raw_surface).expect("surface");
        let adapter = self.pick_adapter(&instance, &surface).expect("adapter");
        let info = adapter.get_info();
        self.log(format!(
            "adapter: {} ({:?}, driver {} {})",
            info.name, info.backend, info.driver, info.driver_info
        ));
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface config");
        config.present_mode = self.present_mode;
        self.config = Some(config.clone());
        self.instance = Some(instance);
        self.adapter = Some(adapter.clone());
        let gpu = self.open(&adapter).expect("device");
        surface.configure(&gpu.device, &config);
        self.hang_iterations = calibrate(&gpu, &config, self.hang_secs);
        self.surface = Some(surface);
        self.gpu = Some(gpu);
        self.window = Some(window);
        self.log(format!(
            "drawing; strategy {:?}; press H to hang the GPU, Esc to quit",
            self.strategy
        ));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => match logical_key {
                Key::Named(NamedKey::Escape) => event_loop.exit(),
                Key::Character(c) if c.eq_ignore_ascii_case("h") => self.hang_now = true,
                _ => {}
            },
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                if let (Some(config), Some(gpu), Some(surface)) = (&mut self.config, &self.gpu, &self.surface) {
                    config.width = size.width;
                    config.height = size.height;
                    if !self.lost.load(Ordering::Acquire) {
                        surface.configure(&gpu.device, config);
                    }
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("wgpu_core=warn,wgpu_hal=warn")).init();
    let mut args = std::env::args().skip(1);
    let mut spike = Spike {
        strategy: Strategy::Same,
        hang_at: None,
        hangs_left: 1,
        start: Instant::now(),
        window: None,
        instance: None,
        adapter: None,
        surface: None,
        config: None,
        gpu: None,
        lost: Arc::new(AtomicBool::new(false)),
        hang_now: false,
        frames: 0,
        lost_at: None,
        recoveries: Vec::new(),
        last_status: "Success",
        bad_frames: 0,
        present_mode: wgpu::PresentMode::Fifo,
        wait_in_hang: false,
        poll_after_loss: false,
        raw_surface: false,
        frames_after: 300,
        hang_secs: 12.0,
        hang_iterations: 1,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "same" => spike.strategy = Strategy::Same,
            "new-surface" => spike.strategy = Strategy::NewSurface,
            "new-instance" => spike.strategy = Strategy::NewInstance,
            "new-window" => spike.strategy = Strategy::NewWindow,
            "--wait-in-hang" => spike.wait_in_hang = true,
            "--poll-after-loss" => spike.poll_after_loss = true,
            "--raw-surface" => spike.raw_surface = true,
            "--frames-after" => spike.frames_after = args.next().and_then(|v| v.parse().ok()).unwrap_or(300),
            "--present" => {
                spike.present_mode = match args.next().as_deref() {
                    Some("mailbox") => wgpu::PresentMode::Mailbox,
                    Some("immediate") => wgpu::PresentMode::Immediate,
                    _ => wgpu::PresentMode::Fifo,
                }
            }
            "--hang-at" => spike.hang_at = args.next().and_then(|v| v.parse().ok()),
            "--hang-secs" => spike.hang_secs = args.next().and_then(|v| v.parse().ok()).unwrap_or(12.0),
            "--log" => {
                let path = args.next().expect("--log <file>");
                *LOG.lock().expect("log") = Some(std::fs::File::create(&path).expect("log file"));
            }
            "--hangs" => spike.hangs_left = args.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            other => panic!("unknown argument {other}"),
        }
    }
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut spike).expect("run");
    // A raw-handle surface does not keep its window alive: it goes first,
    // its swapchain torn down while the window still exists.
    drop(spike.gpu.take());
    drop(spike.surface.take());
    drop(spike.window.take());
    say("closed cleanly");
}
