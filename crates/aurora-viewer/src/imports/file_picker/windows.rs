//! rfd has no cancellation handle. Dispatch a normal close to the dialogs of
//! its dedicated thread, from their own modal loop, without retaining HWNDs
//! or touching windows belonging to another picker or to the main viewer.
use super::Cancellation;
use std::{cell::RefCell, marker::PhantomData, ptr::null_mut, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{EnumThreadWindows, GetClassNameW, KillTimer, PostMessageW, SetTimer, WM_CLOSE},
};

thread_local! {
    static CANCELLATION: RefCell<Option<Cancellation>> = const { RefCell::new(None) };
}

pub(super) struct Guard {
    timer: usize,
    // The timer and thread-local state must be released on their own thread.
    _thread: PhantomData<Rc<()>>,
}

impl Guard {
    pub fn new(cancellation: Cancellation) -> Option<Self> {
        // SAFETY: a thread timer, with a static callback of the required ABI.
        // The native dialog dispatches messages on this same thread.
        let timer = unsafe { SetTimer(null_mut(), 0, 50, Some(tick)) };
        if timer == 0 {
            log::warn!("import picker cancellation timer: {}", std::io::Error::last_os_error());
            return None;
        }
        CANCELLATION.with(|slot| *slot.borrow_mut() = Some(cancellation));
        Some(Self {
            timer,
            _thread: PhantomData,
        })
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        CANCELLATION.with(|slot| *slot.borrow_mut() = None);
        // SAFETY: this timer was created on this thread and the guard is !Send.
        unsafe { KillTimer(null_mut(), self.timer) };
    }
}

unsafe extern "system" fn tick(_: HWND, _: u32, _: usize, _: u32) {
    let cancelled = CANCELLATION.with(|slot| slot.borrow().as_ref().is_some_and(Cancellation::cancelled));
    if cancelled {
        // SAFETY: enumeration is restricted to the dedicated picker's thread;
        // the callback posts a message and never holds a borrowed window handle.
        unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(close_dialog), 0) };
    }
}

unsafe extern "system" fn close_dialog(window: HWND, _: LPARAM) -> i32 {
    let mut class = [0u16; 32];
    // SAFETY: Windows supplies a live enumerated HWND; the buffer is writable
    // for exactly the capacity passed. Only native dialog windows are closed.
    let len = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
    // #32770 is the standard Windows dialog class, including IFileDialog.
    if len == 6 && class[..6] == [35, 51, 50, 55, 55, 48] {
        // SAFETY: WM_CLOSE carries no pointers and requests the normal cancel
        // path (like the title-bar X), rather than destroying a COM-owned HWND.
        unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
    }
    1
}
