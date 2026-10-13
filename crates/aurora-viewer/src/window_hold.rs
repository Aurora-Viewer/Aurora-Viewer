//! Frames while Windows holds the window.
//!
//! A mouse button pressed on the window frame (title bar, border, close /
//! maximize / minimize button) and the system menu are followed by Windows
//! in a message loop of its own, inside the handling of the message that
//! started it. winit's loop does not turn for as long as it lasts:
//! `about_to_wait` is not called, the redraw it asks for each frame never
//! comes, and the viewer showed a still image until the button was released
//! or the menu closed.
//!
//! Those loops do not all deliver paint messages (the one of the title bar
//! buttons reads mouse messages only), but all of them deliver the messages
//! *sent* from another thread. For as long as the hold lasts, a helper
//! thread asks for an immediate repaint of the window, which winit turns
//! into a `RedrawRequested` on the main thread, as for any other frame.
//! Outside a hold the helper sleeps and nothing changes.
//!
//! The main thread must stay free for that loop: Windows handles one mouse
//! message between two frames, and a window dragged with a fast mouse trails
//! far behind the cursor if frames follow one another. So during a hold the
//! frame limiter does not wait on the main thread ([`paces`]): the helper
//! waits instead, between two requests, and it spaces the frames out while
//! the cursor moves ([`pause`]).
//!
//! `AURORA_WINDOW_HOLD=0` turns it off, to compare.

use std::time::Duration;

use winit::window::Window;

/// While the cursor moves, the main thread draws at most one part of the
/// time in this many, and not more often than [`MOVING_MIN`].
const MOVING_SHARE: u32 = 5;
const MOVING_MIN: Duration = Duration::from_millis(16);
/// A request answered faster than this drew nothing (window gone, frame
/// refused by winit).
const NOTHING_DRAWN: Duration = Duration::from_micros(500);

/// Wait of the helper after a frame that kept the main thread for `work`:
/// the rest of the limiter's `period` (zero: no limit) while the cursor is
/// still, enough to leave the main thread to Windows while it moves.
#[cfg_attr(not(windows), allow(dead_code))]
fn pause(work: Duration, period: Duration, moving: bool) -> Duration {
    let pause = if moving {
        (work * (MOVING_SHARE - 1)).max(MOVING_MIN)
    } else {
        period.saturating_sub(work)
    };
    if work < NOTHING_DRAWN {
        pause.max(Duration::from_millis(2))
    } else {
        pause
    }
}

/// Frame limiter, on the main thread: `true` while Windows holds the window.
/// The helper then waits the `period` of the limiter (`None`: no limit)
/// between two frames, and the caller must not wait.
pub fn paces(period: Option<Duration>) -> bool {
    #[cfg(windows)]
    return imp::paces(period.unwrap_or_default());
    #[cfg(not(windows))]
    {
        let _ = period;
        false
    }
}

/// Called once, on the thread that created the window.
pub fn install(window: &Window) {
    #[cfg(windows)]
    if std::env::var("AURORA_WINDOW_HOLD").as_deref() != Ok("0") {
        imp::install(window);
    }
    #[cfg(not(windows))]
    let _ = window;
}

#[cfg(windows)]
mod imp {
    use std::{
        sync::{
            OnceLock,
            atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        },
        thread::{self, Thread},
        time::{Duration, Instant},
    };

    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        Graphics::Gdi::{RDW_INTERNALPAINT, RDW_UPDATENOW, RedrawWindow},
        UI::WindowsAndMessaging::{
            CallWindowProcW, GWLP_WNDPROC, GetCursorPos, SC_MOVE, SC_SIZE, SetWindowLongPtrW, WM_ENTERMENULOOP, WM_EXITMENULOOP,
            WM_NCLBUTTONDOWN, WM_SYSCOMMAND, WNDPROC,
        },
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// Window procedure in place before ours (winit's).
    static PREVIOUS: AtomicIsize = AtomicIsize::new(0);
    /// Messages being handled that Windows may follow in a loop of its own
    /// (one inside the other: a press on the title bar starts a move).
    static HOLDS: AtomicU32 = AtomicU32::new(0);
    /// The system menu is open.
    static MENU: AtomicBool = AtomicBool::new(false);
    static HELPER: OnceLock<Thread> = OnceLock::new();
    /// Period of the frame limiter, in microseconds (0: no limit).
    static PERIOD: AtomicU32 = AtomicU32::new(0);
    /// The cursor counts as moving for this long after its last move.
    const MOVING_FOR: Duration = Duration::from_millis(150);

    fn held() -> bool {
        HOLDS.load(Ordering::Acquire) > 0 || MENU.load(Ordering::Acquire)
    }

    pub fn paces(period: Duration) -> bool {
        if !held() {
            return false;
        }
        PERIOD.store(period.as_micros().min(u32::MAX as u128) as u32, Ordering::Release);
        true
    }

    fn cursor() -> (i32, i32) {
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: `GetCursorPos` writes the point it is given.
        unsafe {
            GetCursorPos(&mut point);
        }
        (point.x, point.y)
    }

    pub fn install(window: &winit::window::Window) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        let hwnd = handle.hwnd.get();
        if HELPER.get().is_some() {
            return;
        }
        let helper = thread::Builder::new().name("window-hold".into()).spawn(move || {
            let mut at = cursor();
            let mut moved = None;
            loop {
                while !held() {
                    thread::park();
                    moved = None;
                }
                let start = Instant::now();
                // SAFETY: `RedrawWindow` takes a window handle and flags; the
                // handle of a destroyed window makes it fail, nothing more.
                // The call returns once the window's thread has painted.
                unsafe {
                    RedrawWindow(
                        hwnd as HWND,
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        RDW_INTERNALPAINT | RDW_UPDATENOW,
                    );
                }
                let work = start.elapsed();
                let now = cursor();
                if now != at {
                    at = now;
                    moved = Some(Instant::now());
                }
                let moving = moved.is_some_and(|t: Instant| t.elapsed() < MOVING_FOR);
                let period = Duration::from_micros(PERIOD.load(Ordering::Acquire).into());
                let pause = super::pause(work, period, moving);
                if !pause.is_zero() {
                    thread::sleep(pause);
                }
            }
        });
        let Ok(helper) = helper else {
            return;
        };
        let _ = HELPER.set(helper.thread().clone());
        // SAFETY: `hwnd` is the live window of this thread, and `hook` has
        // the signature of a window procedure. The previous procedure is
        // stored before any message can reach `hook`: the messages of this
        // window are only delivered on this thread.
        unsafe {
            let hook: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT = hook;
            let previous = SetWindowLongPtrW(hwnd as HWND, GWLP_WNDPROC, hook as usize as isize);
            PREVIOUS.store(previous, Ordering::Release);
        }
    }

    fn wake() {
        if let Some(helper) = HELPER.get() {
            helper.unpark();
        }
    }

    unsafe extern "system" fn hook(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: `PREVIOUS` holds the value `SetWindowLongPtrW` returned for
        // this window: a window procedure (or a handle to one, which
        // `CallWindowProcW` accepts), never 0 once the hook is installed.
        let previous: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(PREVIOUS.load(Ordering::Acquire)) };
        // a press on the frame, or a move / resize started from the keyboard
        // (the low four bits of a system command are not part of it)
        let hold = match msg {
            WM_NCLBUTTONDOWN => true,
            WM_SYSCOMMAND => matches!(wparam as u32 & 0xFFF0, SC_MOVE | SC_SIZE),
            _ => false,
        };
        match msg {
            WM_ENTERMENULOOP => {
                MENU.store(true, Ordering::Release);
                wake();
            }
            WM_EXITMENULOOP => MENU.store(false, Ordering::Release),
            _ => {}
        }
        if hold {
            HOLDS.fetch_add(1, Ordering::AcqRel);
            wake();
        }
        // SAFETY: the arguments are those Windows gave for this message.
        // For a hold, the call returns when Windows lets go of the window.
        let result = unsafe { CallWindowProcW(previous, hwnd, msg, wparam, lparam) };
        if hold {
            HOLDS.fetch_sub(1, Ordering::AcqRel);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_still_cursor_keeps_the_pace_of_the_limiter() {
        // 120 fps: a frame of 1 ms leaves the rest of the period
        let period = Duration::from_micros(8_333);
        assert_eq!(pause(MS, period, false), period - MS);
        // a frame longer than the period, or no limit: the next one at once
        assert_eq!(pause(20 * MS, period, false), Duration::ZERO);
        assert_eq!(pause(MS, Duration::ZERO, false), Duration::ZERO);
    }

    #[test]
    fn a_moving_cursor_leaves_the_main_thread_to_windows() {
        // short frames: not more often than every 16 ms, whatever the limit
        assert_eq!(pause(MS, Duration::from_micros(6_060), true), MOVING_MIN);
        assert_eq!(pause(MS, Duration::ZERO, true), MOVING_MIN);
        // long frames: one part of the time in five
        assert_eq!(pause(20 * MS, Duration::from_micros(8_333), true), 80 * MS);
    }

    #[test]
    fn a_request_that_drew_nothing_does_not_spin() {
        assert_eq!(pause(Duration::from_micros(50), Duration::ZERO, false), 2 * MS);
        assert_eq!(pause(Duration::from_micros(50), 8 * MS, false), 8 * MS - Duration::from_micros(50));
    }
}
