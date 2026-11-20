use super::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, GetPropW, WS_OVERLAPPEDWINDOW};

struct TestWindow(HWND);

impl TestWindow {
    fn new() -> Self {
        // SAFETY: a built-in class, constant title, no parent, menu or user
        // data; the window stays hidden and is owned by this test thread.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!("Aurora SLurl unit test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                100,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null(), "{}", std::io::Error::last_os_error());
        Self(hwnd)
    }

    fn stamp(&self) -> usize {
        // SAFETY: this test's live window, querying a scalar property.
        (unsafe { GetPropW(self.0, FOCUS) }) as usize
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        // SAFETY: this thread owns the window and registrations drop first.
        unsafe { DestroyWindow(self.0) };
    }
}

fn scope() -> Scope {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    Scope {
        demo: true,
        title: Some(format!("native-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))),
    }
}

#[test]
fn the_last_focused_window_wins_and_changes_when_focus_returns() {
    let scope = scope();
    let a = TestWindow::new();
    let a_registration = Registration::new(a.0, &scope).unwrap();
    let b = TestWindow::new();
    let b_registration = Registration::new(b.0, &scope).unwrap();
    a_registration.focused();
    assert_eq!(candidates(&scope)[0].stamp, a.stamp());
    b_registration.focused();
    assert!(b.stamp() > a.stamp());
    assert_eq!(candidates(&scope)[0].stamp, b.stamp());
    a_registration.focused();
    assert!(a.stamp() > b.stamp());
    assert_eq!(candidates(&scope)[0].stamp, a.stamp());
}

#[test]
fn closing_the_last_focused_window_leaves_the_previous_receiver() {
    let scope = scope();
    let a = TestWindow::new();
    let a_registration = Registration::new(a.0, &scope).unwrap();
    let b = TestWindow::new();
    let b_registration = Registration::new(b.0, &scope).unwrap();
    a_registration.focused();
    b_registration.focused();
    let saved_focus = b_registration.stamp();
    drop(b_registration);
    assert_eq!(b.stamp(), 0, "remove our properties before destroying the HWND");
    // A GPU recovery replaces the HWND without a new focus event.
    let replacement = TestWindow::new();
    let replacement_registration = Registration::new(replacement.0, &scope).unwrap();
    replacement_registration.remember_focus(saved_focus);
    assert_eq!(candidates(&scope)[0].stamp, saved_focus);
    drop(replacement_registration);
    let remaining = candidates(&scope);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].stamp, a.stamp());
    drop(a_registration);
    assert!(candidates(&scope).is_empty());
}

#[test]
fn offline_scenarios_and_explicit_titles_do_not_receive_each_others_links() {
    let a_scope = scope();
    let b_scope = scope();
    let a = TestWindow::new();
    let _a_registration = Registration::new(a.0, &a_scope).unwrap();
    assert_eq!(candidates(&a_scope).len(), 1);
    assert!(candidates(&b_scope).is_empty());
    let online = Scope {
        demo: false,
        title: a_scope.title.clone(),
    };
    assert!(candidates(&online).is_empty());
    let b = TestWindow::new();
    let _b_registration = Registration::new(b.0, &online).unwrap();
    assert_eq!(candidates(&a_scope).len(), 1);
    assert_eq!(candidates(&online).len(), 1);
    assert!(
        candidates(&Scope { demo: false, title: None })
            .iter()
            .any(|c| c.pid == std::process::id())
    );
}

#[test]
fn a_window_without_focus_does_not_displace_a_focused_receiver() {
    let scope = scope();
    let a = TestWindow::new();
    let a_registration = Registration::new(a.0, &scope).unwrap();
    a_registration.focused();
    let b = TestWindow::new();
    let _b_registration = Registration::new(b.0, &scope).unwrap();
    assert_eq!(b.stamp(), 0);
    assert_eq!(candidates(&scope)[0].stamp, a.stamp());
}
