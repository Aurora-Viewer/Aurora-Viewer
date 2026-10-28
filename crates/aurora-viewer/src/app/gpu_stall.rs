//! When the graphics card stops finishing frames: the renderer no longer
//! waits for it on the main thread (aurora-render, `Renderer::stalled_for`),
//! so the window keeps answering, the world and the sound go on. Nothing
//! can be drawn, though: after [`NOTICE_AFTER`] a native message box says
//! the card does not answer, with what to try, and asks whether to keep
//! waiting or to close the viewer (logged out properly). The render
//! thread's stack goes to the crash session, so a session closed now is
//! reported as "GPU not responding" and the next start offers the safe
//! graphics. When frames come again the box goes away by itself.
//! The Legacy Viewer has no such notice: its LLWatchdog
//! (indra/llcommon/llwatchdog.cpp) ends a viewer whose main loop stops,
//! which only happens here if the main thread itself is stuck.
//!
//! When the card is lost for good (`Renderer::device_lost`: a driver reset,
//! the card removed), nothing will ever be drawn again: a box says so at
//! once, with what to try, and its OK closes the viewer the same way. The
//! session is reported as "GPU device lost", and the next start offers the
//! safe graphics too. The Legacy Viewer has no notice either.

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// No frame for this long and the notice shows.
const NOTICE_AFTER: Duration = Duration::from_secs(6);
/// Kept waiting from the notice: shown again after this long if still stuck.
const NOTICE_AGAIN: Duration = Duration::from_secs(60);

/// What the user answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Wait,
    Close,
}

/// A message box shown on its own thread (it runs its own message loop).
struct Notice {
    answer: std::sync::mpsc::Receiver<Answer>,
    /// The box's thread, to close it when the GPU answers again (0 before
    /// it is up).
    thread: Arc<AtomicU32>,
    /// Set before the box is closed from here: its answer is not the user's.
    withdrawn: Arc<AtomicBool>,
}

#[derive(Default)]
pub(super) struct GpuStall {
    /// The stall has been noted (log, crash session).
    noted: bool,
    notice: Option<Notice>,
    /// When the user last chose to keep waiting.
    waited_at: Option<Instant>,
    /// The device loss has been noted and its box shown.
    lost: bool,
    /// Closing was asked from the notice.
    pub(super) close: bool,
}

/// Whether to show the notice now.
fn notice_due(stalled: Duration, shown: bool, waited_at: Option<Instant>, now: Instant) -> bool {
    !shown && stalled >= NOTICE_AFTER && waited_at.is_none_or(|at| now.duration_since(at) >= NOTICE_AGAIN)
}

impl App {
    /// Each frame, after the renderer was asked for one.
    pub(super) fn gpu_stall_frame(&mut self, renderer: &aurora_render::Renderer) {
        let now = Instant::now();
        let stall = &mut self.gpu_stall;
        if renderer.device_lost() {
            if !std::mem::replace(&mut stall.lost, true) {
                log::error!("the graphics card is lost: notice shown, the viewer closes after it");
                crate::crash::gpu_lost();
                if let Some(notice) = stall.notice.take() {
                    withdraw(&notice);
                }
                let (title, body) = lost_text();
                stall.notice = show(title, body, false);
            }
            // its one button closes; a box that could not be shown, at once
            if stall.notice.as_ref().is_none_or(|n| n.answer.try_recv().is_ok()) {
                stall.notice = None;
                stall.close = true;
            }
            return;
        }
        match renderer.stalled_for() {
            Some(stalled) => {
                if !stall.noted && stalled >= NOTICE_AFTER {
                    stall.noted = true;
                    log::warn!(
                        "the graphics card has not finished a frame for {} s: notice shown",
                        stalled.as_secs()
                    );
                    crate::crash::gpu_stalled(stalled.as_secs());
                }
                if notice_due(stalled, stall.notice.is_some(), stall.waited_at, now) {
                    let (title, body) = text(stalled.as_secs());
                    stall.notice = show(title, body, true);
                }
            }
            None => {
                if std::mem::take(&mut stall.noted) {
                    log::warn!("the graphics card answered again");
                    crate::crash::gpu_answered();
                }
                stall.waited_at = None;
                if let Some(notice) = stall.notice.take() {
                    withdraw(&notice);
                }
            }
        }
        if let Some(answer) = stall.notice.as_ref().and_then(|n| n.answer.try_recv().ok()) {
            let withdrawn = stall.notice.take().is_some_and(|n| n.withdrawn.load(Ordering::Acquire));
            match answer {
                _ if withdrawn => {}
                Answer::Wait => stall.waited_at = Some(now),
                Answer::Close => {
                    log::warn!("closing at the user's request while the graphics card does not answer");
                    stall.close = true;
                }
            }
        }
    }
}

impl App {
    /// Closing from the notice: logged out and saved as usual. The render
    /// thread may never give the GPU back, and whatever waits for it at
    /// exit would keep the process alive: it ends at the latest 10 s later.
    pub(super) fn close_while_gpu_stalled(&mut self, event_loop: &ActiveEventLoop) {
        let _ = std::thread::Builder::new().name("aurora-exit-guard".into()).spawn(|| {
            std::thread::sleep(Duration::from_secs(10));
            log::warn!("the graphics card kept the viewer from closing: ended");
            std::process::exit(0);
        });
        self.shutdown(event_loop);
    }
}

/// The box's title and text.
pub(super) fn text(seconds: u64) -> (String, String) {
    (
        aurora_i18n::tr!("app-gpu-stall-title").to_string(),
        aurora_i18n::tr!("app-gpu-stall", seconds = seconds.to_string()).to_string(),
    )
}

/// The device loss box's title and text.
pub(super) fn lost_text() -> (String, String) {
    (
        aurora_i18n::tr!("app-gpu-lost-title").to_string(),
        aurora_i18n::tr!("app-gpu-lost").to_string(),
    )
}

/// Opens the box on its own thread: nothing here waits for the answer. A
/// box that does not `ask` has OK alone, answered as Close.
fn show(title: String, body: String, ask: bool) -> Option<Notice> {
    let (tx, answer) = std::sync::mpsc::channel();
    let thread = Arc::new(AtomicU32::new(0));
    let withdrawn = Arc::new(AtomicBool::new(false));
    let notice = Notice {
        answer,
        thread: thread.clone(),
        withdrawn,
    };
    let spawned = std::thread::Builder::new().name("aurora-gpu-notice".into()).spawn(move || {
        thread.store(platform::thread_id(), Ordering::Release);
        let wait = if ask {
            platform::ask(&title, &body)
        } else {
            platform::tell(&title, &body);
            false
        };
        let _ = tx.send(if wait { Answer::Wait } else { Answer::Close });
    });
    match spawned {
        Ok(_) => Some(notice),
        Err(e) => {
            log::warn!("graphics card notice not shown: {e}");
            None
        }
    }
}

/// The GPU answered again: the box closes as if "keep waiting" was chosen.
fn withdraw(notice: &Notice) {
    notice.withdrawn.store(true, Ordering::Release);
    platform::close(notice.thread.load(Ordering::Acquire));
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, IDYES, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MESSAGEBOX_RESULT,
        MESSAGEBOX_STYLE, MessageBoxW, PostMessageW, WM_COMMAND,
    };

    pub fn thread_id() -> u32 {
        // SAFETY: no argument, no state changed.
        unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
    }

    fn message_box(title: &str, body: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (title, body) = (wide(title), wide(body));
        // SAFETY: both strings are zero-terminated and outlive the call; no
        // owner window (the viewer's belongs to the main thread).
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                body.as_ptr(),
                title.as_ptr(),
                style | MB_TOPMOST | MB_SETFOREGROUND,
            )
        }
    }

    /// Yes: keep waiting. A box with no Cancel button cannot be dismissed
    /// any other way than by an answer.
    pub fn ask(title: &str, body: &str) -> bool {
        message_box(title, body, MB_YESNO | MB_ICONWARNING) == IDYES
    }

    /// OK alone: returns when it is pressed.
    pub fn tell(title: &str, body: &str) {
        message_box(title, body, MB_OK | MB_ICONERROR);
    }

    /// The box's own answer to its Yes button: unlike a click on the button
    /// (BM_CLICK), it works while the box is not the active window.
    unsafe extern "system" fn press_yes(window: HWND, _: LPARAM) -> windows_sys::core::BOOL {
        // SAFETY: a window of the box's thread, handed by EnumThreadWindows;
        // posting a message to it reads nothing of ours.
        unsafe { PostMessageW(window, WM_COMMAND, IDYES as usize, 0) };
        1
    }

    /// Answers the box of that thread with Yes.
    pub fn close(thread: u32) {
        if thread != 0 {
            // SAFETY: the callback only uses the handles it is given.
            unsafe { EnumThreadWindows(thread, Some(press_yes), 0) };
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn thread_id() -> u32 {
        0
    }

    pub fn ask(title: &str, body: &str) -> bool {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(title)
            .set_description(body)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes
    }

    pub fn tell(title: &str, body: &str) {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title(title)
            .set_description(body)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }

    pub fn close(_thread: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_waits_for_a_real_stall_and_for_the_user_s_patience() {
        let now = Instant::now();
        assert!(!notice_due(Duration::from_secs(3), false, None, now));
        assert!(notice_due(NOTICE_AFTER, false, None, now));
        assert!(!notice_due(Duration::from_secs(30), true, None, now), "already shown");
        // kept waiting a moment ago: not again at once, but later
        assert!(!notice_due(Duration::from_secs(30), false, Some(now), now));
        assert!(notice_due(Duration::from_secs(90), false, Some(now), now + NOTICE_AGAIN));
    }

    #[test]
    fn the_text_says_how_long_and_what_to_try() {
        let (title, body) = text(12);
        assert!(!title.is_empty());
        assert!(body.contains("12"));
        assert!(body.lines().count() >= 5, "{body}");
    }

    #[test]
    fn the_loss_text_says_what_to_try() {
        let (title, body) = lost_text();
        assert!(!title.is_empty());
        assert!(body.lines().count() >= 5, "{body}");
    }
}
