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

/// True while [`poll_guarded`] waits on this thread: a panic now is wgpu's
/// answer to a device lost during the wait, caught there.
pub fn in_guarded_poll() -> bool {
    GUARDED.with(Cell::get)
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

/// Runs `wait`, its panic taken as a lost device (`None`), the flag the
/// panic hook reads set meanwhile.
fn guarded<T>(wait: impl FnOnce() -> T) -> Option<T> {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            GUARDED.with(|g| g.set(false));
        }
    }
    GUARDED.with(|g| g.set(true));
    let _reset = Reset;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(wait)).ok()
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

/// The demo's loss was given to a renderer already.
pub(crate) static DEMO_LOSS_TAKEN: AtomicBool = AtomicBool::new(false);

/// `AURORA_DEMO_GPU_LOST=<frame>`, with `AURORA_DEMO` only: the device is
/// taken as lost at that frame, as the device-lost callback would after a
/// driver reset.
pub(crate) fn demo_frame(demo: bool, var: Option<&str>) -> Option<u64> {
    if !demo {
        return None;
    }
    var?.trim().parse().ok()
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
