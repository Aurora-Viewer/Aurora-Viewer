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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
pub struct DeviceLost {
    lost: Arc<AtomicBool>,
    work: Arc<Mutex<Work>>,
}

/// The GPU work the render thread last gave the device and the last it saw
/// finished: at a loss, [`DeviceLost::work_at_loss`] tells a hang from a
/// fault, which the crash reports could not ([`hang_or_fault`]).
#[derive(Debug, Default)]
struct Work {
    /// Frame and time of the newest submission.
    submitted: Option<(u64, Instant)>,
    /// Frame and time of the newest submission the GPU finished.
    finished: Option<(u64, Instant)>,
}

impl DeviceLost {
    pub fn set(&self) {
        self.lost.store(true, Ordering::Release);
    }

    pub fn get(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    /// Frame `frame` was just submitted (renderer.rs, after `queue.submit`).
    pub fn submitted(&self, frame: u64) {
        if let Ok(mut w) = self.work.lock() {
            w.submitted = Some((frame, Instant::now()));
        }
    }

    /// The GPU finished frame `frame` (`on_submitted_work_done`).
    pub fn finished(&self, frame: u64) {
        if let Ok(mut w) = self.work.lock()
            && w.finished.is_none_or(|(done, _)| done < frame)
        {
            w.finished = Some((frame, Instant::now()));
        }
    }

    /// What the GPU had in hand at the loss (the device-lost callback logs
    /// it): see [`hang_or_fault`].
    pub fn work_at_loss(&self) -> String {
        let now = Instant::now();
        let Ok(w) = self.work.lock() else {
            return "unknown".to_owned();
        };
        let age = |at: Instant| now.saturating_duration_since(at);
        hang_or_fault(
            w.submitted.map(|(frame, at)| (frame, age(at))),
            w.finished.map(|(frame, at)| (frame, age(at))),
        )
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

/// A loss that comes this long after a submission the GPU never finished
/// is the driver's watchdog (a Windows TDR resets the GPU after 2 s).
const WATCHDOG: Duration = Duration::from_millis(1500);

/// Hang or fault: the line the device-lost callback logs, from the newest
/// submission and the newest finished one, each with its age at the loss.
/// A submission the GPU had not finished after the watchdog's delay is a
/// hang (a pass that never ends, or too long); a loss with the last
/// submission finished, or right after it, is a fault the driver found in
/// that work (an invalid access on NVIDIA: VK_ERROR_DEVICE_LOST at once).
/// The crash reports showed neither: the main thread's own stall warning
/// (STALL_AFTER, 2 s like the watchdog) comes too late for most hangs.
fn hang_or_fault(submitted: Option<(u64, Duration)>, finished: Option<(u64, Duration)>) -> String {
    let secs = |d: Duration| format!("{:.3} s", d.as_secs_f64());
    let Some((frame, age)) = submitted else {
        return "nothing submitted yet (lost while starting)".to_owned();
    };
    let done = finished.is_some_and(|(f, _)| f >= frame);
    let last = match finished {
        Some((f, a)) => format!("frame {f} finished {} before", secs(a)),
        None => "none finished yet".to_owned(),
    };
    // (no colon, semicolon or apostrophe: the crash report's table keeps
    // the line as it is, aurora-viewer crash/summary.rs `label`)
    let verdict = if done {
        "a fault found after that frame ended (not a hang)"
    } else if age >= WATCHDOG {
        "a hang (the watchdog of the driver reset the card while that frame ran)"
    } else {
        "a fault in the work of that frame (not a hang)"
    };
    format!(
        "last submission frame {frame}, {} before, {}, {last}, {verdict}",
        secs(age),
        if done { "finished" } else { "not finished" }
    )
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

    /// The line of the loss tells a watchdog reset (the last frame not
    /// finished after the watchdog's delay) from a fault (the last frame
    /// finished, or lost right after its submission).
    #[test]
    fn the_loss_names_a_hang_or_a_fault_from_the_work_in_hand() {
        let ms = Duration::from_millis;
        assert_eq!(hang_or_fault(None, None), "nothing submitted yet (lost while starting)");
        let hang = hang_or_fault(Some((812, ms(2040))), Some((811, ms(2055))));
        assert_eq!(
            hang,
            "last submission frame 812, 2.040 s before, not finished, frame 811 finished 2.055 s before, \
             a hang (the watchdog of the driver reset the card while that frame ran)"
        );
        let fault = hang_or_fault(Some((812, ms(9))), Some((812, ms(1))));
        assert!(fault.ends_with("a fault found after that frame ended (not a hang)"), "{fault}");
        let early = hang_or_fault(Some((812, ms(120))), Some((811, ms(130))));
        assert!(early.ends_with("a fault in the work of that frame (not a hang)"), "{early}");
        let first = hang_or_fault(Some((0, ms(3000))), None);
        assert!(first.contains("none finished yet") && first.contains("a hang"), "{first}");
        // the shared copies see the same work
        let lost = DeviceLost::default();
        let render_thread = lost.clone();
        render_thread.submitted(5);
        render_thread.finished(4);
        render_thread.finished(3);
        assert!(
            lost.work_at_loss().starts_with("last submission frame 5, 0."),
            "{}",
            lost.work_at_loss()
        );
        assert!(
            lost.work_at_loss().contains("frame 4 finished"),
            "the newest finished frame is kept"
        );
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
