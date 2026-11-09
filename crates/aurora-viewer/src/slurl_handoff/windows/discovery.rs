//! Discover receivers by window properties, never their translated titles.
//! LLAppViewerWin32::sendURLToOtherInstance (indra/newview/llappviewerwin32.cpp)
//! finds a window by class. Record actual focus events as well: Z order alone
//! also changes when a window is made topmost, without receiving focus.

use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetPropW, GetWindowThreadProcessId, RemovePropW, SetPropW};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

const FOCUS: *const u16 = windows_sys::w!("Aurora.SLurl.Focus.2");

static PUBLISHED: Mutex<Option<Published>> = Mutex::new(None);

struct Published {
    registration: Registration,
    // Keep the HWND alive until its properties have been removed.
    _window: Arc<Window>,
}

#[derive(Clone)]
pub(super) struct Scope {
    pub demo: bool,
    pub title: Option<String>,
}

impl Scope {
    pub fn current() -> Self {
        Self {
            demo: std::env::var_os("AURORA_DEMO").is_some(),
            title: crate::cli::title_slug(),
        }
    }

    pub fn property(&self) -> Vec<u16> {
        let mut name = format!("Aurora.SLurl.Receiver.2.{}", if self.demo { "Demo" } else { "Online" });
        if let Some(title) = &self.title {
            name.push('.');
            name.push_str(title);
        }
        name.encode_utf16().chain([0]).collect()
    }
}

/// Only this process writes its own properties. The sender reads them even
/// across UAC, then the pipe's ACL checks the client's logon and integrity.
pub(super) struct Registration {
    hwnd: usize,
    properties: Vec<Vec<u16>>,
}

impl Registration {
    /// The caller keeps its own window alive until this registration drops.
    pub fn new(hwnd: HWND, scope: &Scope) -> std::io::Result<Self> {
        let mut registration = Self {
            hwnd: hwnd as usize,
            properties: vec![scope.property()],
        };
        // An ordinary browser launch has no --title. Online test windows
        // participate too; explicit titles and all offline demos stay scoped.
        if !scope.demo && scope.title.is_some() {
            registration.properties.push(Scope { demo: false, title: None }.property());
        }
        for property in &registration.properties {
            // SAFETY: this is our live HWND and a terminated property name.
            // The value is a marker, never dereferenced as a pointer.
            if unsafe { SetPropW(hwnd, property.as_ptr(), 1usize as _) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(registration)
    }

    pub fn focused(&self) {
        use windows_sys::Win32::System::Performance::QueryPerformanceCounter;
        // Performance-counter ticks are shared by all processes and are not
        // affected by changes of the wall clock. Aurora's Windows targets
        // are 64-bit, so the entire timestamp fits in a property value.
        let mut stamp = 0;
        // SAFETY: a valid output pointer for a monotonic clock reading.
        if unsafe { QueryPerformanceCounter(&mut stamp) } == 0 {
            return;
        }
        self.remember_focus(stamp.max(1) as usize);
    }

    fn stamp(&self) -> usize {
        // SAFETY: query a scalar property of our own live HWND.
        (unsafe { GetPropW(self.hwnd as HWND, FOCUS) }) as usize
    }

    fn remember_focus(&self, stamp: usize) {
        // SAFETY: the property is on our live window; this integer is never
        // dereferenced as a pointer.
        if unsafe { SetPropW(self.hwnd as HWND, FOCUS, stamp as _) } == 0 {
            log::warn!("startup links: focus not published ({})", std::io::Error::last_os_error());
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // SAFETY: the owner retains the window through this drop. These are
        // scalar markers, with no allocation for RemovePropW to release.
        unsafe {
            for property in &self.properties {
                RemovePropW(self.hwnd as HWND, property.as_ptr());
            }
            RemovePropW(self.hwnd as HWND, FOCUS);
        }
    }
}

pub(super) fn publish(window: &Arc<Window>) {
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
    let mut published = PUBLISHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // A recovered GPU replaces the HWND, not the instance's focus history.
    let previous_stamp = published.as_ref().map_or(0, |p| p.registration.stamp());
    drop(published.take());
    match Registration::new(handle.hwnd.get() as HWND, &Scope::current()) {
        Ok(registration) => {
            if window.has_focus() {
                registration.focused();
            } else if previous_stamp != 0 {
                registration.remember_focus(previous_stamp);
            }
            *published = Some(Published {
                registration,
                _window: window.clone(),
            });
        }
        Err(error) => log::warn!("startup links: receiver not published ({error})"),
    }
}

pub(super) fn focused() {
    if let Some(published) = &*PUBLISHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
        published.registration.focused();
    }
}

pub(super) fn release() {
    drop(PUBLISHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take());
}

#[derive(Debug)]
pub(super) struct Candidate {
    pub pid: u32,
    stamp: usize,
}

pub(super) fn candidates(scope: &Scope) -> Vec<Candidate> {
    struct Search {
        property: Vec<u16>,
        found: Vec<Candidate>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: isize) -> i32 {
        // SAFETY: EnumWindows is synchronous; data is its caller's live
        // Search, uniquely borrowed by this callback until it returns.
        let search = unsafe { &mut *(data as *mut Search) };
        // SAFETY: the enumerator supplied the HWND, and the terminated name
        // belongs to Search; this reads no memory of the other process.
        if unsafe { GetPropW(hwnd, search.property.as_ptr()) }.is_null() {
            return 1;
        }
        let mut pid = 0;
        // SAFETY: queries a top-level HWND supplied by EnumWindows and a
        // valid output pointer. No message is sent to the other process.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid != 0 {
            // SAFETY: another scalar property query, with a constant name.
            let stamp = unsafe { GetPropW(hwnd, FOCUS) } as usize;
            search.found.push(Candidate { pid, stamp });
        }
        1
    }
    let mut search = Search {
        property: scope.property(),
        found: Vec::new(),
    };
    // SAFETY: visit receives this live Search only during EnumWindows.
    if unsafe { EnumWindows(Some(visit), (&mut search as *mut Search) as isize) } == 0 {
        log::warn!("startup links: windows not enumerated ({})", std::io::Error::last_os_error());
    }
    // Stable sorting leaves the enumeration order as the tie-breaker for
    // focus events within the same clock tick. Never-focused windows last.
    search.found.sort_by_key(|candidate| std::cmp::Reverse(candidate.stamp));
    search.found
}

#[cfg(test)]
mod tests;
