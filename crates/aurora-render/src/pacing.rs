//! Frame pacing: a frame reads its input only once the GPU is at most one
//! frame behind, so that what it shows is the input of the moment it is
//! drawn and not the input of a frame that waited in a queue (#1484).
//!
//! A frame goes through three stages: the main thread (input, interface,
//! packet), the render thread (encoding, `submit`, `present`) and the GPU
//! behind the swapchain. When the GPU sets the pace, the stages ahead of it
//! fill up: the main thread waits for the render thread after it has read
//! the input, the render thread waits for a swapchain image, the swapchain
//! holds the frames the GPU has not drawn yet. The frame rate is the same,
//! but a mouse move takes three or four frames to reach the screen, about
//! 60 ms at 60 frames per second, where the frame limiter, which leaves the
//! stages empty, shows it in one or two.
//!
//! So once it has handed frame N over, the main thread waits, at the place
//! of the frame limiter and before the events of the next frame are read,
//! until the GPU has finished frame N−1 ([`GpuFrames`]). The news comes from
//! the render thread: a poll at the start of each frame's job when the
//! frame before is already done, else a wait on the device once it has
//! presented frame N (never the main thread's: it would stall the render
//! thread's `submit` and `present`). Frame N+1 then reads its input while the GPU
//! draws frame N, and is ready by the time it is done: the GPU keeps a
//! frame to draw, and nothing else waits in front of it. When the GPU is not
//! the limit, frame N−1 is long finished and the wait costs nothing.
//!
//! The Legacy Viewer reads the input, draws and swaps on one thread
//! (`LLAppViewer::mainLoop`, indra/newview/llappviewer.cpp): only the
//! driver's queue lies between its input and the screen. Aurora's render
//! thread adds a stage, which this wait keeps from adding latency.
//!
//! AURORA_FRAME_PACING=0 turns it off (comparison).

use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::time::Instant;

/// Submissions kept: the frames the main thread can still wait for.
const KEPT: usize = 4;

/// The GPU submission of each frame, newest last, and the frames the GPU
/// is known to have finished. The thread that submits writes both (the
/// render thread, which waits on the device itself: a wait on the device
/// from the main thread holds the device's locks and stalls the render
/// thread's `submit` and `present`); the main thread only waits for the
/// news, before it reads the input.
pub(crate) struct GpuFrames<T> {
    pub enabled: bool,
    submitted: Mutex<VecDeque<(u64, T)>>,
    /// Every frame up to this one is finished on the GPU.
    finished: Mutex<Option<u64>>,
    news: Condvar,
}

impl<T: Clone> GpuFrames<T> {
    pub fn new(enabled: bool) -> GpuFrames<T> {
        GpuFrames {
            enabled,
            submitted: Mutex::new(VecDeque::with_capacity(KEPT)),
            finished: Mutex::new(None),
            news: Condvar::new(),
        }
    }

    /// Every frame up to `frame` is finished on the GPU (the newest one
    /// submitted, and those that submitted nothing).
    pub fn finished_through(&self, frame: u64) {
        let mut f = self.finished.lock();
        if f.is_none_or(|done| done < frame) {
            *f = Some(frame);
            self.news.notify_all();
        }
    }

    /// Wait until `frame` is finished on the GPU, not past `deadline`:
    /// false when it is not by then.
    pub fn wait_finished(&self, frame: u64, deadline: Instant) -> bool {
        let mut f = self.finished.lock();
        while f.is_none_or(|done| done < frame) {
            if self.news.wait_until(&mut f, deadline).timed_out() {
                return f.is_some_and(|done| done >= frame);
            }
        }
        true
    }

    /// Frame `frame` submitted its work as `index`.
    pub fn submitted(&self, frame: u64, index: T) {
        let mut s = self.submitted.lock();
        if s.len() == KEPT {
            s.pop_front();
        }
        s.push_back((frame, index));
    }

    /// The submission of the newest frame older than `frame`: what has to
    /// be finished before the frame after `frame` reads its input. None
    /// when no such frame was submitted (start, frames not drawn).
    pub fn before(&self, frame: u64) -> Option<T> {
        let s = self.submitted.lock();
        s.iter().rev().find(|(f, _)| *f < frame).map(|(_, index)| index.clone())
    }
}

/// On unless AURORA_FRAME_PACING says `0`, `off`, `false` or `no`.
pub fn wanted(var: Option<&str>) -> bool {
    !matches!(
        var.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0" | "off" | "false" | "no")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three stages, one frame at a time: the main thread builds for
    /// `build` ms, the render thread holds a frame `render` ms after its
    /// swapchain image, the GPU draws one in `gpu` ms, the swapchain lets
    /// two frames wait for it. Times in ms.
    struct Pipeline {
        build: f32,
        render: f32,
        gpu: f32,
        render_free: f32,
        /// When the GPU finishes each frame, in order.
        done: Vec<f32>,
        now: f32,
    }

    impl Pipeline {
        fn new(build: f32, render: f32, gpu: f32) -> Pipeline {
            Pipeline {
                build,
                render,
                gpu,
                render_free: 0.0,
                done: Vec::new(),
                now: 0.0,
            }
        }

        /// One frame; with `paced`, the wait of this module after the
        /// hand-over. Returns the time from its input to the end of its GPU
        /// work, and when it was handed over.
        fn frame(&mut self, paced: bool) -> (f32, f32) {
            let input = self.now;
            self.now += self.build;
            self.now = self.now.max(self.render_free);
            let handed = self.now;
            // the swapchain: an image once fewer than two frames still wait
            // for the GPU
            let n = self.done.len();
            let image = if n >= 2 { self.done[n - 2] } else { 0.0 };
            let present = handed.max(image) + self.render;
            let start = self.done.last().copied().unwrap_or(0.0).max(present);
            self.done.push(start + self.gpu);
            self.render_free = present;
            if paced && n >= 1 {
                self.now = self.now.max(self.done[n - 1]);
            }
            (start + self.gpu - input, handed)
        }
    }

    /// Mean input-to-GPU-done latency and frame period, once settled.
    fn run(build: f32, render: f32, gpu: f32, paced: bool) -> (f32, f32) {
        let mut p = Pipeline::new(build, render, gpu);
        let frames = 400;
        let (mut latency, mut first, mut last) = (0.0, 0.0, 0.0);
        for i in 0..frames {
            let (l, handed) = p.frame(paced);
            if i == frames / 2 {
                first = handed;
            }
            if i >= frames / 2 {
                latency += l;
                last = handed;
            }
        }
        let n = (frames - frames / 2) as f32;
        (latency / n, (last - first) / (n - 1.0))
    }

    #[test]
    fn a_gpu_bound_frame_keeps_its_rate_with_fresher_input() {
        let (late, period_off) = run(4.0, 3.0, 16.0, false);
        let (fresh, period_on) = run(4.0, 3.0, 16.0, true);
        // the GPU sets the pace either way
        assert!((period_off - 16.0).abs() < 0.01, "{period_off}");
        assert!((period_on - 16.0).abs() < 0.01, "{period_on}");
        // three frames and more in front of the GPU, then about two: the
        // frame drawn, and the one that reads its input meanwhile
        assert!(late > 55.0, "without pacing: {late}");
        assert!(fresh < 33.0, "with pacing: {fresh} (without: {late})");
    }

    #[test]
    fn a_main_thread_bound_frame_does_not_wait() {
        let (_, free) = run(20.0, 3.0, 5.0, false);
        let (_, paced) = run(20.0, 3.0, 5.0, true);
        assert!((free - 20.0).abs() < 0.01, "{free}");
        assert!((paced - 20.0).abs() < 0.01, "{paced}");
    }

    #[test]
    fn a_render_thread_bound_frame_keeps_its_rate() {
        let (_, free) = run(4.0, 12.0, 8.0, false);
        let (_, paced) = run(4.0, 12.0, 8.0, true);
        assert!((free - 12.0).abs() < 0.01, "{free}");
        assert!((paced - 12.0).abs() < 0.01, "{paced}");
    }

    #[test]
    fn the_main_thread_waits_for_the_news_of_the_frame_before() {
        let frames = std::sync::Arc::new(GpuFrames::<()>::new(true));
        let soon = || Instant::now() + std::time::Duration::from_millis(20);
        assert!(!frames.wait_finished(0, soon()), "nothing finished yet");
        frames.finished_through(3);
        assert!(frames.wait_finished(2, soon()));
        assert!(frames.wait_finished(3, soon()));
        assert!(!frames.wait_finished(4, soon()));
        // older news does not move it back
        frames.finished_through(1);
        assert!(frames.wait_finished(3, soon()));
        // news from the render thread wakes the main thread
        let f = frames.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            f.finished_through(5);
        });
        assert!(frames.wait_finished(5, Instant::now() + std::time::Duration::from_secs(10)));
        t.join().expect("render thread");
    }

    #[test]
    fn the_frame_before_is_the_one_waited_for() {
        let frames = GpuFrames::new(true);
        assert_eq!(frames.before(0), None);
        frames.submitted(0, 'a');
        assert_eq!(frames.before(0), None);
        assert_eq!(frames.before(1), Some('a'));
        frames.submitted(1, 'b');
        frames.submitted(2, 'c');
        // after handing frame 2 over, the main thread waits for frame 1
        assert_eq!(frames.before(2), Some('b'));
        // a frame not drawn (no swapchain image): the one before it
        frames.submitted(4, 'e');
        assert_eq!(frames.before(4), Some('c'));
        for (f, c) in (5..10).zip('f'..) {
            frames.submitted(f, c);
        }
        assert_eq!(frames.before(2), None, "older than what is kept");
        assert_eq!(frames.before(9), Some('i'));
    }

    #[test]
    fn the_switch_only_turns_pacing_off_explicitly() {
        assert!(wanted(None));
        assert!(wanted(Some("1")));
        assert!(!wanted(Some("0")));
        assert!(!wanted(Some(" Off ")));
        assert!(!wanted(Some("false")));
        assert!(!wanted(Some("no")));
    }
}
