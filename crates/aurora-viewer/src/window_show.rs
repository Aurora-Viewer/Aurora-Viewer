//! Showing the startup window, maximized or not, with the keyboard focus.
//!
//! The window is created hidden and shown once its first frame is drawn
//! (`App::frame`). winit maximizes a window with `ShowWindow(SW_MAXIMIZE)`,
//! which also shows and activates it, and then hides it again while its own
//! flags still say hidden: the window ended "active" in its thread while it
//! was hidden, Windows sent no focus when it was shown, and the keyboard
//! went nowhere until the user clicked another window and came back.
//!
//! On Windows the window is now shown once, already maximized when it has
//! to be, then asked to come to the foreground and take the focus, as
//! LLWindowWin32::show does (indra/llwindow/llwindowwin32.cpp: `ShowWindow`,
//! `SetForegroundWindow`, `SetFocus`). `SetForegroundWindow` follows the
//! rules of Windows: a user who went to another application during the
//! start keeps it, and the taskbar button flashes instead. winit's flags
//! are set after, visible first, so they never hide it again.

use winit::window::Window;

/// Show the startup window, maximized when `maximized`.
pub fn show(window: &Window, maximized: bool) {
    #[cfg(windows)]
    imp::show_native(window, maximized);
    // winit's own state: visible first (a maximize on a window it thinks
    // hidden shows it, then hides it again)
    window.set_visible(true);
    if maximized {
        window.set_maximized(true);
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::{
            Input::KeyboardAndMouse::SetFocus,
            WindowsAndMessaging::{SHOW_WINDOW_CMD, SW_SHOW, SW_SHOWMAXIMIZED, SetForegroundWindow, ShowWindow},
        },
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// The `ShowWindow` command of the startup window: maximized in one
    /// step, never maximized while hidden.
    pub fn show_command(maximized: bool) -> SHOW_WINDOW_CMD {
        if maximized { SW_SHOWMAXIMIZED } else { SW_SHOW }
    }

    pub fn show_native(window: &winit::window::Window, maximized: bool) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        let hwnd = handle.hwnd.get() as HWND;
        // SAFETY: plain window manager calls on the window this thread
        // created; a refused foreground request returns false and nothing
        // else happens.
        unsafe {
            ShowWindow(hwnd, show_command(maximized));
            SetForegroundWindow(hwnd);
            SetFocus(hwnd);
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOW, SW_SHOWMAXIMIZED};

    #[test]
    fn the_startup_window_is_shown_maximized_in_one_step() {
        assert_eq!(super::imp::show_command(true), SW_SHOWMAXIMIZED);
        assert_eq!(super::imp::show_command(false), SW_SHOW);
    }
}
