//! Submission lifetime and first-loss worker ownership.
use super::{model::*, vulkan};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
static GENERATION: AtomicU64 = AtomicU64::new(1);

// Initialized once by the crash subsystem, before devices exist. The sink is
// called at device startup or from the diagnostic worker, never from a driver/loss callback.
type Sink = dyn Fn(&Report, &[u8]) + Send + Sync;
static SINK: OnceLock<Box<Sink>> = OnceLock::new();
static FIRST_LOSS: OnceLock<Mutex<Report>> = OnceLock::new();
pub fn set_sink(sink: impl Fn(&Report, &[u8]) + Send + Sync + 'static) {
    let _ = SINK.set(Box::new(sink));
}
pub fn first_loss() -> Option<Report> {
    FIRST_LOSS.get()?.try_lock().ok().map(|report| report.clone())
}

pub(crate) struct Recorder {
    baseline: Report,
    report: Mutex<Report>,
    ring: Mutex<Ring>,
    #[cfg(not(target_os = "macos"))]
    native: vulkan::Native,
}
impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuRecorder").finish_non_exhaustive()
    }
}

/// A render-view submission owns its slot until the GPU completion callback.
/// Both XR eyes retain separate tickets even when they share a frame number.
/// Dropping an unsubmitted recording releases it without touching GPU memory.
pub(crate) struct Frame {
    recorder: Arc<Recorder>,
    ticket: Ticket,
    submitted: bool,
}
impl Recorder {
    #[cfg(not(target_os = "macos"))]
    pub(crate) fn new(device: &wgpu::Device, plan: &vulkan::Plan) -> Option<Arc<Self>> {
        let mut plan = *plan;
        let native = match vulkan::Native::new(device, &plan) {
            Ok(native) => native,
            Err(error) => {
                log::warn!("GPU breadcrumbs: recorder unavailable ({error:?}); retaining capability status with diagnostics disabled");
                plan.disable();
                vulkan::Native::new(device, &plan).ok()?
            }
        };
        let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
        let report = Report {
            offered: plan.offered,
            method: plan.method,
            fault_enabled: plan.enabled.fault,
            generation,
            fault: if plan.enabled.fault {
                FaultStatus::NotRequested
            } else if plan.offered.fault {
                FaultStatus::Disabled
            } else {
                FaultStatus::Unsupported
            },
            ..Report::default()
        };
        log::info!(
            "GPU breadcrumbs: {} ({} allocated bytes)",
            report.breadcrumbs(),
            native.allocation_bytes
        );
        log::info!("Driver fault report: {}", report.fault());
        if first_loss().is_none()
            && let Some(sink) = SINK.get()
        {
            sink(&report, &[]);
        }
        let _ = worker(); // Thread creation is done at startup, not in the loss callback.
        Some(Arc::new(Self {
            baseline: report.clone(),
            report: Mutex::new(report),
            ring: Mutex::new(Ring::new(generation)),
            native,
        }))
    }
    pub(crate) fn frame(self: &Arc<Self>, frame: u64) -> Option<Frame> {
        if self.baseline.method == Method::Unavailable {
            return None;
        }
        let ticket = self.ring.lock().ok()?.reserve(frame)?;
        Some(Frame {
            recorder: self.clone(),
            ticket,
            submitted: false,
        })
    }
    /// First loss wins across renderer recovery. Does not enter Vulkan/wgpu,
    /// wait for a worker, or perform filesystem I/O under the driver's callback.
    pub(crate) fn lost(self: &Arc<Self>, confirmed: bool) {
        if FIRST_LOSS.get().is_some() {
            return;
        }
        let mut snapshot = self
            .report
            .try_lock()
            .map_or_else(|_| self.baseline.clone(), |report| report.clone());
        snapshot.fault = if !confirmed {
            FaultStatus::Simulated
        } else if snapshot.fault_enabled {
            FaultStatus::Pending
        } else {
            snapshot.fault
        };
        if let Ok(ring) = self.ring.try_lock() {
            snapshot.saturated = ring.saturated;
        }
        if FIRST_LOSS.set(Mutex::new(snapshot.clone())).is_err() {
            return;
        }
        if let Some(worker) = worker() {
            let _ = worker.try_send(Job {
                recorder: self.clone(),
                confirmed,
                report: snapshot,
            });
        }
    }
    fn complete(&self, ticket: Ticket) {
        let Ok(mut ring) = self.ring.lock() else {
            return;
        };
        // Only this completed slot can be read: other slots may still be
        // written by the GPU. Never read the whole ring on normal completion.
        #[cfg(not(target_os = "macos"))]
        if let Some(words) = self.native.completed_words(ticket.slot) {
            let last = words
                .into_iter()
                .enumerate()
                .filter_map(|(i, token)| ring.decode(ticket.generation, ticket.slot * WORDS + i, token))
                .max_by_key(|o| o.token);
            if let Ok(mut report) = self.report.lock()
                && last.is_some_and(|last| report.observed.is_none_or(|old| old.token < last.token))
            {
                report.observed = last;
                report.stage = last.map_or(Stage::Unavailable, |last| {
                    if report.method == Method::Transfer {
                        Stage::Transfer
                    } else if last.boundary.begin() {
                        Stage::Top
                    } else {
                        Stage::Bottom
                    }
                });
            }
        }
        ring.complete(ticket);
    }
}
impl Frame {
    pub(crate) fn mark(&self, encoder: &mut wgpu::CommandEncoder, boundary: Boundary) {
        let Some((offset, token)) = self.recorder.ring.lock().ok().and_then(|mut r| r.record(self.ticket, boundary)) else {
            return;
        };
        let method = self.recorder.baseline.method;
        #[cfg(not(target_os = "macos"))]
        self.recorder.native.record(encoder, method, offset, token, boundary.begin());
        #[cfg(target_os = "macos")]
        let _ = (encoder, method, offset, token);
    }
    pub(crate) fn finish(&self, encoder: &mut wgpu::CommandEncoder) {
        #[cfg(not(target_os = "macos"))]
        self.recorder.native.host_barrier(encoder);
        #[cfg(target_os = "macos")]
        let _ = encoder;
    }
    /// Called before submit: the loss callback can occur inside queue.submit.
    /// After submission, move this guard into on_submitted_work_done.
    pub(crate) fn submitting(&mut self) {
        self.submitted = true;
    }
    pub(crate) fn completed(mut self) {
        self.recorder.complete(self.ticket);
        self.submitted = false;
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        if !self.submitted
            && let Ok(mut ring) = self.recorder.ring.lock()
        {
            ring.complete(self.ticket);
        }
    }
}

struct Job {
    recorder: Arc<Recorder>,
    confirmed: bool,
    report: Report,
}
pub(super) fn spawn_jobs<J: Send + 'static>(
    name: &str,
    mut process: impl FnMut(J) + Send + 'static,
) -> Option<std::sync::mpsc::SyncSender<J>> {
    let (send, receive) = std::sync::mpsc::sync_channel::<J>(1);
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            while let Ok(job) = receive.recv() {
                process(job);
            }
        })
        .ok()
        .map(|_| send)
}
fn worker() -> Option<&'static std::sync::mpsc::SyncSender<Job>> {
    static WORKER: OnceLock<Option<std::sync::mpsc::SyncSender<Job>>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            spawn_jobs("gpu-fault-details", |job: Job| {
                let mut report = job.report;
                let mut private = Vec::new();
                #[cfg(not(target_os = "macos"))]
                if job.confirmed {
                    job.recorder.native.collect(&job.recorder.ring, &mut report, &mut private);
                }
                if let Some(first) = FIRST_LOSS.get()
                    && let Ok(mut first) = first.lock()
                {
                    *first = report.clone();
                }
                if let Some(sink) = SINK.get() {
                    sink(&report, &private);
                }
            })
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires Vulkan AMD markers; records a small command buffer, never induces a loss"]
    fn breadcrumb_gpu_marker_and_transfer_round_trip_without_device_loss() {
        let _guard = crate::test_gpu::lock();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("Vulkan GPU");
        let auto = vulkan::Plan::query(&adapter);
        assert_eq!(auto.method, Method::Amd, "explicit hardware check needs AMD markers");
        let mut transfer = auto;
        transfer.method = Method::Transfer;
        transfer.enabled.amd = false;
        transfer.enabled.nv = false;
        for plan in [auto, transfer] {
            let (device, queue) = vulkan::open(&adapter, &wgpu::DeviceDescriptor::default(), &plan).expect("Vulkan device");
            let recorder = Recorder::new(&device, &plan).expect("diagnostic recorder");
            let mut frame = recorder.frame(77).expect("ring slot");
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            frame.mark(&mut encoder, Boundary::ContactBegin);
            frame.mark(&mut encoder, Boundary::ContactEnd);
            frame.finish(&mut encoder);
            frame.submitting();
            let submission = queue.submit([encoder.finish()]);
            queue.on_submitted_work_done(move || frame.completed());
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(5)),
                })
                .expect("marker submission completed");
            let report = recorder.report.lock().unwrap();
            assert_eq!(report.observed.unwrap().boundary, Boundary::ContactEnd);
            assert_eq!(report.observed.unwrap().frame, 77);
            assert!(!recorder.ring.lock().unwrap().slots.iter().any(|slot| slot.pending));
        }
    }
}
