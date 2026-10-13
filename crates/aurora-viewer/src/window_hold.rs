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
//! `AURORA_WINDOW_HOLD=0` turns it off, to compare.

use winit::window::Window;

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
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{RDW_INTERNALPAINT, RDW_UPDATENOW, RedrawWindow},
        UI::WindowsAndMessaging::{
            CallWindowProcW, GWLP_WNDPROC, SC_MOVE, SC_SIZE, SetWindowLongPtrW, WM_ENTERMENULOOP, WM_EXITMENULOOP, WM_NCLBUTTONDOWN,
            WM_SYSCOMMAND, WNDPROC,
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

    fn held() -> bool {
        HOLDS.load(Ordering::Acquire) > 0 || MENU.load(Ordering::Acquire)
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
            loop {
                while !held() {
                    thread::park();
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
                if start.elapsed() < Duration::from_micros(500) {
                    // nothing was drawn (window gone, frame refused): do not spin
                    thread::sleep(Duration::from_millis(2));
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
