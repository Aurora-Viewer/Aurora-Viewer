//! The GPU device lost for good: a driver reset (TDR), a card removed or
//! taken by another program, its video memory exhausted. wgpu treats a loss
//! found while waiting on the device as fatal: `Device::poll` panics
//! (`handle_error_fatal`, wgpu 30) instead of returning an error. Once the
//! device-lost callback said so, nothing waits on the device any more and no
//! frame is built (main_thread.rs): a photo asked a minute after a driver
//! reset ended the session with a Rust panic (#1237). The viewer then closes
//! properly (app/gpu_stall.rs).
//!
//! A loss can also land during a wait, or between the check and the wait:
//! every wait on the device goes through [`poll_guarded`], which takes that
//! panic as the loss it is (the flag set, the wait given up) instead of
//! ending the render thread. The panic hook asks [`in_guarded_poll`] so the
//! session is not reported as a Rust panic (aurora-viewer logging.rs).

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    static GUARDED: Cell<bool> = const { Cell::new(false) };
}

/// True while [`poll_guarded`] waits on this thread: a panic now whose
/// message is the loss ([`is_loss_panic`]) is wgpu's answer to a device
/// lost during the wait, caught there.
pub fn in_guarded_poll() -> bool {
    GUARDED.with(Cell::get)
}

/// A panic message of wgpu's for a lost device ("Error in Device::poll:
/// Parent device is lost", wgpu-core's `DeviceError::Lost`); any other
/// panic during a guarded wait is a bug, reported as one.
pub fn is_loss_panic(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("device is lost") || message.contains("device lost")
}

/// `device.poll`, `None` when wgpu found the device lost during it
/// (`handle_error_fatal`, wgpu 30). The panic comes once wgpu-core has
/// returned its error, with none of its locks held, so the device stays
/// usable for the rest of the teardown; it is never waited on again.
pub fn poll_guarded(device: &wgpu::Device, poll: wgpu::PollType) -> Option<Result<wgpu::PollStatus, wgpu::PollError>> {
    let result = guarded(|| device.poll(poll));
    if result.is_none() {
        log::error!("GPU device lost during a wait on the GPU");
    }
    result
}

/// Runs `wait`, its panic for a lost device taken as the loss (`None`),
/// the flag the panic hook reads set meanwhile; another panic goes on.
fn guarded<T>(wait: impl FnOnce() -> T) -> Option<T> {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            GUARDED.with(|g| g.set(false));
        }
    }
    GUARDED.with(|g| g.set(true));
    let _reset = Reset;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(wait)) {
        Ok(done) => Some(done),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied());
            if message.is_some_and(is_loss_panic) {
                None
            } else {
                std::panic::resume_unwind(payload)
            }
        }
    }
}

/// Shared between the device-lost callback, the main thread and the render
/// thread.
#[derive(Clone, Debug, Default)]
pub struct DeviceLost(Arc<AtomicBool>);

impl DeviceLost {
    pub fn set(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn get(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Waits for the GPU to finish the work submitted (read-backs: photos,
    /// captures, clicks). False, without waiting, once the device is lost:
    /// the read-back is then not mapped and the caller gives it up.
    pub fn wait(&self, device: &wgpu::Device) -> bool {
        self.poll_with(device, wgpu::PollType::wait_indefinitely())
            .is_some_and(|r| r.is_ok())
    }

    /// Runs the callbacks of the work already finished, without waiting.
    pub fn poll(&self, device: &wgpu::Device) {
        let _ = self.poll_with(device, wgpu::PollType::Poll);
    }

    /// Any wait on the device: `None`, without waiting, once it is lost, and
    /// the flag set when the loss comes during the wait itself.
    pub fn poll_with(&self, device: &wgpu::Device, poll: wgpu::PollType) -> Option<Result<wgpu::PollStatus, wgpu::PollError>> {
        if self.get() {
            return None;
        }
        let result = poll_guarded(device, poll);
        if result.is_none() {
            self.set();
        }
        result
    }
}

/// The renderers made so far in the run, for `AURORA_DEMO_GPU_LOST`'s list,
/// and whether the demo's hang was given to one already.
pub(crate) static DEMO_RENDERERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub(crate) static DEMO_HANG_TAKEN: AtomicBool = AtomicBool::new(false);

/// `AURORA_DEMO_GPU_HANG=<frame>`: a real GPU hang, for the recovery's test
/// on a real driver reset. One compute dispatch whose loop runs about 12 s
/// on a recent card: the driver's watchdog resets the GPU first (a Windows
/// TDR after 2 s, amdgpu's lockup timeout on Linux), as in the crash
/// reports; the loop ends by itself where no watchdog does. Other programs
/// using the GPU may lose their work in flight with it.
pub(crate) fn demo_hang(device: &wgpu::Device, queue: &wgpu::Queue) {
    const SHADER: &str = "
        @group(0) @binding(0) var<storage, read_write> out: array<f32>;
        @group(0) @binding(1) var<uniform> n: vec4<u32>;
        @compute @workgroup_size(64)
        fn main(@builtin(global_invocation_id) id: vec3<u32>) {
            var x = f32(id.x) * 0.001;
            for (var i = 0u; i < n.x; i++) { x = fract(sin(x) * 43758.5453 + 0.1); }
            out[id.x] = x;
        }";
    const GROUPS: u32 = 4096;
    // ~6e11 loop steps a second on an RX 9070 XT: about 12 s
    const STEPS: u32 = 30_000_000;
    log::warn!(
        "AURORA_DEMO_GPU_HANG: submitting a GPU hang ({STEPS} steps in each of {} threads)",
        GROUPS * 64
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("demo hang"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    use crate::pipeline_cache::CachedPipelines;
    let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("demo hang"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("demo hang"),
        size: u64::from(GROUPS) * 64 * 4,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let steps = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("demo hang steps"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&steps, 0, &[STEPS.to_le_bytes(), [0; 4], [0; 4], [0; 4]].concat());
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("demo hang"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: out.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: steps.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(GROUPS, 1, 1);
    }
    queue.submit([encoder.finish()]);
}

/// `AURORA_DEMO_GPU_LOST=<frame>`, with `AURORA_DEMO` only: the device is
/// taken as lost at that frame, as the device-lost callback would after a
/// driver reset.
pub(crate) fn demo_frame(demo: bool, var: Option<&str>) -> Option<u64> {
    demo_frame_of(demo, var, 0)
}

/// `AURORA_DEMO_GPU_LOST=<frame>,<frame>…`: the frame the `renderer`-th
/// renderer of the run (0 at the start, then one per GPU recovery) takes its
/// device as lost at, counted from its own start; none past the list.
pub(crate) fn demo_frame_of(demo: bool, var: Option<&str>, renderer: usize) -> Option<u64> {
    if !demo {
        return None;
    }
    var?.split(',').nth(renderer)?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_callback_s_copy_tells_every_other() {
        let lost = DeviceLost::default();
        let (callback, render_thread) = (lost.clone(), lost.clone());
        assert!(!render_thread.get());
        callback.set();
        assert!(lost.get() && render_thread.get());
    }

    /// wgpu's panic for a device lost during a wait (`handle_error_fatal`)
    /// is taken as the loss: the hook sees the guard while it panics, the
    /// caller gets nothing back, and the guard is off afterwards.
    #[test]
    fn a_panic_during_a_guarded_wait_is_the_loss() {
        static SEEN: AtomicBool = AtomicBool::new(false);
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| SEEN.store(in_guarded_poll(), Ordering::Relaxed)));
        let lost: Option<u32> = guarded(|| panic!("Device::poll: Validation Error: Parent device is lost"));
        std::panic::set_hook(before);
        assert_eq!(lost, None);
        assert!(SEEN.load(Ordering::Relaxed), "the hook knew the panic was the guarded wait's");
        assert!(!in_guarded_poll(), "the guard is reset after the panic");
        assert_eq!(guarded(|| 7), Some(7));
        assert!(!in_guarded_poll());
        // any other panic is not the loss: it goes on
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let bug = std::panic::catch_unwind(|| guarded(|| -> u32 { panic!("index out of bounds") }));
        std::panic::set_hook(before);
        assert!(bug.is_err());
        assert!(!in_guarded_poll());
    }

    #[test]
    fn each_renderer_of_a_run_takes_the_next_loss_of_the_list() {
        assert_eq!(demo_frame_of(true, Some("600,700, 800"), 0), Some(600));
        assert_eq!(demo_frame_of(true, Some("600,700, 800"), 2), Some(800));
        assert_eq!(demo_frame_of(true, Some("600,700, 800"), 3), None);
        assert_eq!(demo_frame_of(true, Some("600"), 1), None, "a single loss, the first renderer's");
    }

    #[test]
    fn the_demo_loss_needs_the_demo_and_a_frame() {
        assert_eq!(demo_frame(true, Some("600")), Some(600));
        assert_eq!(demo_frame(true, Some(" 12 ")), Some(12));
        assert_eq!(demo_frame(false, Some("600")), None);
        assert_eq!(demo_frame(true, None), None);
        assert_eq!(demo_frame(true, Some("soon")), None);
    }
}
