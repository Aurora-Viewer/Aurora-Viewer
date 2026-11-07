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
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

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
    /// A recovery on a new window and device is to be tried (gpu_recovery.rs),
    /// with the safe graphics level when true.
    pub(super) recover: Option<bool>,
}

/// Whether to show the notice now.
fn notice_due(stalled: Duration, shown: bool, waited_at: Option<Instant>, now: Instant) -> bool {
    !shown && stalled >= NOTICE_AFTER && waited_at.is_none_or(|at| now.duration_since(at) >= NOTICE_AGAIN)
}

impl App {
    /// Each frame, after the renderer was asked for one.
    pub(super) fn gpu_stall_frame(&mut self, renderer: &aurora_render::Renderer) {
        let now = Instant::now();
        // a VR session is not recovered yet (TASKS.md): it closes as before
        let recovers = super::gpu_recovery::enabled(self.settings.gpu_recovery) && !renderer.vr_engaged();
        if renderer.device_lost() && recovers && !self.gpu_stall.lost {
            use super::gpu_recovery::LossStep;
            match super::gpu_recovery::loss_step(&mut self.gpu_losses, now) {
                step @ (LossStep::Recover | LossStep::RecoverSafe) => {
                    let safe = step == LossStep::RecoverSafe;
                    log::error!(
                        "the graphics card is lost: recovering on a new window and device{}",
                        if safe {
                            " with the safe graphics (a second loss within minutes)"
                        } else {
                            ""
                        }
                    );
                    self.gpu_stall.lost = true;
                    self.gpu_stall.recover = Some(safe);
                    return;
                }
                LossStep::Close => log::error!("the graphics card is lost a third time within minutes: no recovery"),
            }
        }
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
            withdraw_all(Duration::from_secs(1));
            crate::logging::drain(crate::logging::DRAIN_WAIT);
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

/// Set on the way out: the boxes still shown go away (`withdraw_all`).
static CLOSING: AtomicBool = AtomicBool::new(false);
/// The boxes shown right now, counted by their threads.
static SHOWN: AtomicUsize = AtomicUsize::new(0);

/// Opens the box on its own thread: nothing here waits for the answer. A
/// box that does not `ask` has OK alone, answered as Close.
fn show(title: String, body: String, ask: bool) -> Option<Notice> {
    let (tx, answer) = std::sync::mpsc::channel();
    let thread = Arc::new(AtomicU32::new(0));
    let withdrawn = Arc::new(AtomicBool::new(false));
    let notice = Notice {
        answer,
        thread: thread.clone(),
        withdrawn: withdrawn.clone(),
    };
    // counted before the thread runs: a viewer ending meanwhile waits for it
    SHOWN.fetch_add(1, Ordering::AcqRel);
    let spawned = std::thread::Builder::new().name("aurora-gpu-notice".into()).spawn(move || {
        thread.store(platform::thread_id(), Ordering::Release);
        let gone = || withdrawn.load(Ordering::Acquire) || CLOSING.load(Ordering::Acquire);
        let wait = if ask {
            platform::ask(&title, &body, &gone)
        } else {
            platform::tell(&title, &body, &gone);
            false
        };
        let _ = tx.send(if wait { Answer::Wait } else { Answer::Close });
        SHOWN.fetch_sub(1, Ordering::AcqRel);
    });
    match spawned {
        Ok(_) => Some(notice),
        Err(e) => {
            SHOWN.fetch_sub(1, Ordering::AcqRel);
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

/// The viewer ends: the boxes still shown go away with it, waiting at most
/// `within` for their threads to take them down. On macOS and Linux the box
/// belongs to another process (UserNotificationCenter, zenity), which keeps
/// it on the screen after the viewer is gone, its buttons answering no one;
/// a box of Windows goes with the process.
pub(crate) fn withdraw_all(within: Duration) {
    CLOSING.store(true, Ordering::Release);
    if cfg!(windows) {
        return;
    }
    let until = Instant::now() + within;
    while SHOWN.load(Ordering::Acquire) > 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
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
    /// any other way than by an answer. Withdrawn by `close`, not `gone`.
    pub fn ask(title: &str, body: &str, _gone: &dyn Fn() -> bool) -> bool {
        message_box(title, body, MB_YESNO | MB_ICONWARNING) == IDYES
    }

    /// OK alone: returns when it is pressed.
    pub fn tell(title: &str, body: &str, _gone: &dyn Fn() -> bool) {
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

/// How often a box's thread on macOS and Linux looks whether it is to go
/// away (the GPU answered, the viewer ends).
#[cfg(not(windows))]
const POLL: Duration = Duration::from_millis(100);

/// A CFUserNotification, the alert rfd showed here before: the
/// UserNotificationCenter process draws it, so it neither stops the main
/// thread (an NSAlert would, and no frame could withdraw it: a demo frame of
/// the macOS merge queue hung an hour on one) nor goes away with the viewer.
/// Unlike rfd's `CFUserNotificationDisplayAlert`, the box is kept, so that
/// its thread can cancel it.
#[cfg(target_os = "macos")]
mod platform {
    use super::POLL;
    use objc2_core_foundation::{
        CFDictionary, CFOptionFlags, CFString, CFUserNotification, kCFUserNotificationAlertHeaderKey, kCFUserNotificationAlertMessageKey,
        kCFUserNotificationAlternateButtonTitleKey, kCFUserNotificationCautionAlertLevel, kCFUserNotificationDefaultButtonTitleKey,
        kCFUserNotificationDefaultResponse, kCFUserNotificationStopAlertLevel,
    };

    /// `CFUserNotificationReceiveResponse` when no answer came within its
    /// timeout (`MACH_RCV_TIMED_OUT`, mach/message.h): the box is still up.
    const TIMED_OUT: i32 = 0x1000_4003;
    /// The button pressed, in the response flags.
    const BUTTON: CFOptionFlags = 0x3;

    pub fn thread_id() -> u32 {
        0
    }

    /// Yes: keep waiting; No, and a box that could not be shown, close.
    pub fn ask(title: &str, body: &str, gone: &dyn Fn() -> bool) -> bool {
        show(title, body, kCFUserNotificationCautionAlertLevel, Some(("Yes", "No")), gone) == Some(kCFUserNotificationDefaultResponse)
    }

    /// OK alone.
    pub fn tell(title: &str, body: &str, gone: &dyn Fn() -> bool) {
        show(title, body, kCFUserNotificationStopAlertLevel, None, gone);
    }

    /// The button pressed; None when the box went away or was not shown.
    fn show(
        title: &str,
        body: &str,
        level: CFOptionFlags,
        buttons: Option<(&str, &str)>,
        gone: &dyn Fn() -> bool,
    ) -> Option<CFOptionFlags> {
        let (header, message) = (CFString::from_str(title), CFString::from_str(body));
        let (yes, no) = buttons.map(|(y, n)| (CFString::from_str(y), CFString::from_str(n))).unzip();
        // SAFETY: CoreFoundation's constant keys, only read.
        let fields = unsafe {
            [
                (kCFUserNotificationAlertHeaderKey, Some(&*header)),
                (kCFUserNotificationAlertMessageKey, Some(&*message)),
                (kCFUserNotificationDefaultButtonTitleKey, yes.as_deref()),
                (kCFUserNotificationAlternateButtonTitleKey, no.as_deref()),
            ]
        };
        let (keys, values): (Vec<&CFString>, Vec<&CFString>) = fields.into_iter().filter_map(|(key, value)| Some((key?, value?))).unzip();
        let fields = CFDictionary::from_slices(&keys, &values);
        let mut error = 0;
        // SAFETY: `error` is a valid out pointer; the dictionary maps the
        // documented keys to strings. No timeout: the box stays until it is
        // answered or cancelled.
        let Some(notice) = (unsafe { CFUserNotification::new(None, 0.0, level, &mut error, Some(fields.as_opaque())) }) else {
            log::warn!("graphics card notice not shown: CFUserNotificationCreate failed ({error})");
            return None;
        };
        loop {
            if gone() {
                notice.cancel();
                return None;
            }
            let mut response: CFOptionFlags = 0;
            // SAFETY: `response` is a valid out pointer.
            match unsafe { notice.receive_response(POLL.as_secs_f64(), &mut response) } {
                0 => return Some(response & BUTTON),
                TIMED_OUT => {}
                e => {
                    log::warn!("graphics card notice: no answer ({e:#x})");
                    notice.cancel();
                    return None;
                }
            }
        }
    }

    /// The box's thread sees `withdrawn` and cancels it.
    pub fn close(_thread: u32) {}
}

/// zenity, the program rfd ran here before: a process of its own, which
/// keeps its box on the screen after the viewer is gone. Run here, it is
/// ended when the box is to go away.
#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::POLL;
    use std::process::{Command, Stdio};

    pub fn thread_id() -> u32 {
        0
    }

    /// Yes: keep waiting; No, closing the box, and a box that could not be
    /// shown (no zenity) close.
    pub fn ask(title: &str, body: &str, gone: &dyn Fn() -> bool) -> bool {
        zenity("--question", title, body, gone) == Some(true)
    }

    /// OK alone.
    pub fn tell(title: &str, body: &str, gone: &dyn Fn() -> bool) {
        zenity("--error", title, body, gone);
    }

    /// Whether the box was answered with its first button; None when it
    /// went away or was not shown.
    fn zenity(kind: &str, title: &str, body: &str, gone: &dyn Fn() -> bool) -> Option<bool> {
        let mut child = match Command::new("zenity")
            .args(["--no-markup", kind, "--title", title, "--text", body])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                log::warn!("graphics card notice not shown (zenity): {e}");
                return None;
            }
        };
        loop {
            if gone() {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            match child.try_wait() {
                Ok(Some(status)) => return Some(status.success()),
                Ok(None) => std::thread::sleep(POLL),
                Err(e) => {
                    log::warn!("graphics card notice: no answer (zenity): {e}");
                    let _ = child.kill();
                    return None;
                }
            }
        }
    }

    /// The box's thread sees `withdrawn` and ends zenity.
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
