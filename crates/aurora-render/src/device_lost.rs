//! The GPU device lost for good: a driver reset (TDR), a card removed or
//! taken by another program, its video memory exhausted. wgpu treats a loss
//! found while waiting on the device as fatal: `Device::poll` panics
//! (`handle_error_fatal`, wgpu 30) instead of returning an error. Once the
//! device-lost callback said so, nothing waits on the device any more and no
//! frame is built (main_thread.rs): a photo asked a minute after a driver
//! reset ended the session with a Rust panic (#1237). The viewer then closes
//! properly (app/gpu_stall.rs).
//!
//! A loss found by the very wait that hits it still panics in wgpu, on the
//! render thread: the session ends through `Renderer::failed`, as before.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
        !self.get() && device.poll(wgpu::PollType::wait_indefinitely()).is_ok()
    }

    /// Runs the callbacks of the work already finished, without waiting.
    pub fn poll(&self, device: &wgpu::Device) {
        if !self.get() {
            let _ = device.poll(wgpu::PollType::Poll);
        }
    }
}

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

    #[test]
    fn the_demo_loss_needs_the_demo_and_a_frame() {
        assert_eq!(demo_frame(true, Some("600")), Some(600));
        assert_eq!(demo_frame(true, Some(" 12 ")), Some(12));
        assert_eq!(demo_frame(false, Some("600")), None);
        assert_eq!(demo_frame(true, None), None);
        assert_eq!(demo_frame(true, Some("soon")), None);
    }
}
