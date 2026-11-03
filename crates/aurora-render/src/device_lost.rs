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
/// fault, which the crash reports could not ([`hang_or_fault`]). Each
/// submission has its own token: the eyes of an XR frame, or the journal's
/// submits and the frame's own, share a frame number.
#[derive(Debug, Default)]
struct Work {
    /// Tokens handed out.
    count: u64,
    /// The newest submission and its time.
    submitted: Option<(Submission, Instant)>,
    /// The newest submission the GPU finished and its time.
    finished: Option<(Submission, Instant)>,
}

/// One `queue.submit`, noted by [`DeviceLost::submitting`]: its order among
/// all the submissions and the frame it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Submission {
    token: u64,
    frame: u64,
}

impl DeviceLost {
    pub fn set(&self) {
        self.lost.store(true, Ordering::Release);
    }

    pub fn get(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    /// A submission of frame `frame` is about to be made: noted before
    /// `queue.submit`, which can itself find the loss or block across a
    /// reset, and without the lock held across it ([`submit_tracked`]).
    pub fn submitting(&self, frame: u64) -> Submission {
        let Ok(mut w) = self.work.lock() else {
            return Submission { token: u64::MAX, frame };
        };
        w.count += 1;
        let submission = Submission { token: w.count, frame };
        w.submitted = Some((submission, Instant::now()));
        submission
    }

    /// The GPU finished `submission` (`on_submitted_work_done`).
    pub fn finished(&self, submission: Submission) {
        if let Ok(mut w) = self.work.lock()
            && w.finished.is_none_or(|(done, _)| done.token < submission.token)
        {
            w.finished = Some((submission, Instant::now()));
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
        let done = match (w.submitted, w.finished) {
            (Some((s, _)), Some((f, _))) => f.token >= s.token,
            _ => false,
        };
        hang_or_fault(
            w.submitted.map(|(s, at)| (s.frame, age(at))),
            w.finished.map(|(f, at)| (f.frame, age(at))),
            done,
        )
    }
}

/// `queue.submit` noted for the line of a device loss: the submission is
/// noted before the call, and the GPU's own news of its end marks it
/// finished. Every submit of the render thread goes through it: the
/// frame's, the journal's copies, a skipped frame's uploads.
pub fn submit_tracked(
    lost: &DeviceLost,
    queue: &wgpu::Queue,
    frame: u64,
    commands: impl IntoIterator<Item = wgpu::CommandBuffer>,
) -> wgpu::SubmissionIndex {
    let submission = lost.submitting(frame);
    let index = queue.submit(commands);
    let lost = lost.clone();
    queue.on_submitted_work_done(move || lost.finished(submission));
    index
}

/// The fixed endings of the line of a loss, in the order of
/// [`hang_or_fault`]'s cases: the crash report only publishes one of them
/// (aurora-viewer crash/summary.rs).
pub const GPU_WORK_VERDICTS: [&str; 4] = [
    "a fault found after that frame ended (not a hang)",
    "a hang (the driver reset the card while that frame ran)",
    "a fault in the work of that frame (not a hang)",
    "undecided (a slow frame that faulted, or a hang)",
];

/// The line of a loss before any submission.
pub const GPU_WORK_NONE: &str = "nothing submitted yet (lost while starting)";

impl DeviceLost {
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

/// A submission the GPU had not finished this long after it, when the
/// loss came, hung: the driver's watchdog reset the card. Measured, not
/// the 2 s of the Windows TDR setting: an RTX 5070 Ti (driver 621.01)
/// declared `AURORA_DEMO_GPU_HANG`'s submission lost 1.101 s after it.
const HANG_AFTER: Duration = Duration::from_millis(500);
/// A loss this soon after a submission the GPU had not finished is a fault
/// the driver found in that work (an invalid access on NVIDIA:
/// VK_ERROR_DEVICE_LOST at once, well within a frame).
const FAULT_WITHIN: Duration = Duration::from_millis(100);

/// Hang or fault: the line the device-lost callback logs, from the newest
/// submission and the newest finished one, each with its frame and its age
/// at the loss, and whether the newest submission is the finished one
/// (`done`). A loss with the last submission finished is a fault found
/// after it; one with it not finished is a hang past [`HANG_AFTER`], a
/// fault within [`FAULT_WITHIN`], and left undecided between the two (a
/// frame that slow may have faulted, or hung). The crash reports showed
/// neither: the main thread's own stall warning (STALL_AFTER, 2 s) comes
/// too late for a hang.
fn hang_or_fault(submitted: Option<(u64, Duration)>, finished: Option<(u64, Duration)>, done: bool) -> String {
    let secs = |d: Duration| format!("{:.3} s", d.as_secs_f64());
    let Some((frame, age)) = submitted else {
        return GPU_WORK_NONE.to_owned();
    };
    let last = match finished {
        Some((f, a)) => format!("frame {f} finished {} before", secs(a)),
        None => "none finished yet".to_owned(),
    };
    // (fixed words and numbers: the crash report parses the line and
    // publishes it rebuilt, aurora-viewer crash/summary.rs)
    let verdict = if done {
        GPU_WORK_VERDICTS[0]
    } else if age >= HANG_AFTER {
        GPU_WORK_VERDICTS[1]
    } else if age < FAULT_WITHIN {
        GPU_WORK_VERDICTS[2]
    } else {
        GPU_WORK_VERDICTS[3]
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

    /// The line of the loss tells a hang (the last frame not finished after
    /// `HANG_AFTER`: the real hang of `AURORA_DEMO_GPU_HANG` on an RTX 5070
    /// Ti was declared lost 1.101 s after its submission) from a fault (the
    /// last frame finished, or lost within `FAULT_WITHIN` of its
    /// submission), and leaves the band between undecided.
    #[test]
    fn the_loss_names_a_hang_or_a_fault_from_the_work_in_hand() {
        let ms = Duration::from_millis;
        assert_eq!(hang_or_fault(None, None, false), GPU_WORK_NONE);
        let hang = hang_or_fault(Some((601, ms(1101))), Some((599, ms(1103))), false);
        assert_eq!(
            hang,
            "last submission frame 601, 1.101 s before, not finished, frame 599 finished 1.103 s before, \
             a hang (the driver reset the card while that frame ran)"
        );
        assert!(hang_or_fault(Some((812, ms(500))), Some((811, ms(510))), false).ends_with(GPU_WORK_VERDICTS[1]));
        let fault = hang_or_fault(Some((812, ms(9))), Some((812, ms(1))), true);
        assert!(fault.ends_with(GPU_WORK_VERDICTS[0]), "{fault}");
        let early = hang_or_fault(Some((812, ms(8))), Some((811, ms(9))), false);
        assert!(early.ends_with(GPU_WORK_VERDICTS[2]), "{early}");
        let slow = hang_or_fault(Some((812, ms(120))), Some((811, ms(130))), false);
        assert!(slow.ends_with(GPU_WORK_VERDICTS[3]), "{slow}");
        let first = hang_or_fault(Some((0, ms(3000))), None, false);
        assert!(
            first.contains("none finished yet") && first.ends_with(GPU_WORK_VERDICTS[1]),
            "{first}"
        );
        // the shared copies see the same work; the newest finished
        // submission is kept, whatever order the news comes in
        let lost = DeviceLost::default();
        let render_thread = lost.clone();
        let s3 = render_thread.submitting(3);
        let s4 = render_thread.submitting(4);
        let s5 = render_thread.submitting(5);
        render_thread.finished(s4);
        render_thread.finished(s3);
        let line = lost.work_at_loss();
        assert!(line.starts_with("last submission frame 5, 0."), "{line}");
        assert!(line.contains("not finished, frame 4 finished"), "{line}");
        render_thread.finished(s5);
        assert!(lost.work_at_loss().ends_with(GPU_WORK_VERDICTS[0]));
    }

    /// Two submissions of one frame (the eyes of an XR frame, the journal's
    /// copies then the frame's own): the first one finishing does not mark
    /// the second finished.
    #[test]
    fn submissions_of_one_frame_are_told_apart() {
        let lost = DeviceLost::default();
        let left = lost.submitting(601);
        let right = lost.submitting(601);
        assert_ne!(left, right);
        lost.finished(left);
        let line = lost.work_at_loss();
        assert!(line.contains("not finished, frame 601 finished"), "{line}");
        assert!(line.ends_with(GPU_WORK_VERDICTS[2]), "{line}");
        lost.finished(right);
        assert!(lost.work_at_loss().ends_with(GPU_WORK_VERDICTS[0]));
        // the first eye's late news does not undo the second's
        lost.finished(left);
        assert!(lost.work_at_loss().ends_with(GPU_WORK_VERDICTS[0]));
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
