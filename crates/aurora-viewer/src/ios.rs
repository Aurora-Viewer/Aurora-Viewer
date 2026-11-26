//! iPadOS: what the viewer asks of UIKit itself, where winit and egui-winit
//! stop. The window's view is kept for the calls that need one; the
//! on-screen keyboard's height is followed, since winit 0.30 does not
//! report it; the system's pasteboard is read and written, since
//! egui-winit's clipboard (arboard) has no iOS backend and keeps its text
//! inside the app.
//!
//! Everything here runs on the main thread, as UIKit wants: a call from
//! another thread does nothing.

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, Message};
use objc2_avf_audio::{AVAudioSession, AVAudioSessionCategoryOptions, AVAudioSessionCategoryPlayAndRecord};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue, NSString, NSValue};
use objc2_ui_kit::{
    NSValueUIGeometryExtensions, UIApplicationDidEnterBackgroundNotification, UIApplicationWillEnterForegroundNotification, UIDevice,
    UIImpactFeedbackGenerator, UIImpactFeedbackStyle, UIKeyboardFrameEndUserInfoKey, UIKeyboardWillChangeFrameNotification,
    UIKeyboardWillHideNotification, UIPasteboard, UIUserInterfaceIdiom, UIView,
};
use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use winit::platform::ios::{ScreenEdge, WindowExtIOS};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

thread_local! {
    /// The main window's view.
    static VIEW: RefCell<Option<Retained<UIView>>> = const { RefCell::new(None) };
}

/// Points of the view's bottom the on-screen keyboard covers (f32 bits).
static KEYBOARD: AtomicU32 = AtomicU32::new(0);

/// The main window exists: keep its view, set what an iPad app sets on its
/// window, and follow the keyboard and the app's way to the background. A
/// window made again (a graphics device lost) only changes the view.
pub fn attach(window: &winit::window::Window) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // the first swipe from the bottom edge shows the system's grabber and
    // the second one leaves the app: the dock's buttons sit along that edge
    window.set_preferred_screen_edges_deferring_system_gestures(ScreenEdge::BOTTOM);
    let view = match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::UiKit(handle)) => handle.ui_view.as_ptr().cast::<UIView>(),
        _ => {
            log::warn!("iPadOS: the window has no UIKit view");
            return;
        }
    };
    // SAFETY: winit hands out the pointer of the live UIView it draws into,
    // and we are on the main thread; `retain` keeps it for as long as we do.
    let view = unsafe { (*view).retain() };
    VIEW.with(|v| *v.borrow_mut() = Some(view));
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        follow_keyboard(mtm);
        follow_background(mtm);
    });
}

/// The audio session plays and records: without a category that records,
/// iPadOS names no microphone and the voice has no input. The speaker
/// stays the output (a recording session answers in the earpiece of a
/// phone otherwise), and a Bluetooth headset is used for listening only
/// (A2DP): taking its microphone too would put the whole viewer's sound in
/// the telephone quality of its hands-free profile. The system asks for the
/// microphone when the voice first opens it, not here.
///
/// Called at the start, before the sound output opens: a category that
/// changes under a running output restarts it. Not where a Mac is the host
/// (Mac Catalyst, the simulator): a session that records brings the Mac's
/// default input device into the output's own start, and one that does not
/// answer (a virtual device) hangs the viewer there, or ends it.
pub fn audio_session() {
    if cfg!(any(target_abi = "macabi", target_abi = "sim")) {
        return;
    }
    // SAFETY: the shared session is a singleton, its category constant a
    // static string of AVFAudio (null on no system this builds for).
    let done = unsafe {
        let session = AVAudioSession::sharedInstance();
        let Some(category) = AVAudioSessionCategoryPlayAndRecord else {
            log::warn!("iPadOS: no recording category, the voice will have no microphone");
            return;
        };
        let options = AVAudioSessionCategoryOptions::DefaultToSpeaker | AVAudioSessionCategoryOptions::AllowBluetoothA2DP;
        session
            .setCategory_withOptions_error(category, options)
            .and_then(|()| session.setActive_error(true))
    };
    if let Err(error) = done {
        log::warn!("iPadOS: the audio session could not be set for the voice: {error}");
    }
}

/// The app goes behind another one, or comes back. winit's `suspended` is
/// not that: it is the app no longer active, which the Control Center, a
/// system question ("Allow Paste") or the app switcher also make it, with
/// the app still on screen and still to be drawn.
fn follow_background(_mtm: MainThreadMarker) {
    let center = NSNotificationCenter::defaultCenter();
    let queue = NSOperationQueue::mainQueue();
    let behind = RcBlock::new(|_: NonNull<NSNotification>| {
        log::info!("iPadOS: behind another app, the frames wait");
        BACKGROUND.store(true, Ordering::Relaxed);
    });
    let back = RcBlock::new(|_: NonNull<NSNotification>| {
        BACKGROUND.store(false, Ordering::Relaxed);
    });
    // SAFETY: both blocks run on the main queue and only touch an atomic;
    // the names are UIKit's constants. The observers live as long as the
    // app: their tokens are leaked on purpose.
    unsafe {
        let a = center.addObserverForName_object_queue_usingBlock(
            Some(UIApplicationDidEnterBackgroundNotification),
            None,
            Some(&queue),
            &behind,
        );
        let b = center.addObserverForName_object_queue_usingBlock(
            Some(UIApplicationWillEnterForegroundNotification),
            None,
            Some(&queue),
            &back,
        );
        std::mem::forget((a, b));
    }
}

/// The keyboard's frame, each time it moves, shows or hides.
fn follow_keyboard(_mtm: MainThreadMarker) {
    let center = NSNotificationCenter::defaultCenter();
    let queue = NSOperationQueue::mainQueue();
    let moved = RcBlock::new(|note: NonNull<NSNotification>| {
        // SAFETY: the notification is alive for the length of the call.
        let note = unsafe { note.as_ref() };
        KEYBOARD.store(covered(note).to_bits(), Ordering::Relaxed);
    });
    let hidden = RcBlock::new(|_: NonNull<NSNotification>| {
        KEYBOARD.store(0, Ordering::Relaxed);
    });
    // SAFETY: both blocks run on the main queue and only touch an atomic
    // and the main thread's own view; the names are UIKit's constants. The
    // observers live as long as the app: their tokens are leaked on purpose.
    unsafe {
        let a = center.addObserverForName_object_queue_usingBlock(Some(UIKeyboardWillChangeFrameNotification), None, Some(&queue), &moved);
        let b = center.addObserverForName_object_queue_usingBlock(Some(UIKeyboardWillHideNotification), None, Some(&queue), &hidden);
        std::mem::forget((a, b));
    }
}

/// The height of the view a keyboard frame covers, in points.
fn covered(note: &NSNotification) -> f32 {
    let Some(info) = note.userInfo() else {
        return 0.0;
    };
    // SAFETY: the key is UIKit's constant string.
    let key: &NSString = unsafe { UIKeyboardFrameEndUserInfoKey };
    let key: &AnyObject = key;
    let Some(value) = info.objectForKey(key) else {
        return 0.0;
    };
    let Ok(value) = value.downcast::<NSValue>() else {
        return 0.0;
    };
    // SAFETY: UIKit stores the keyboard's end frame as a CGRect value.
    let frame = unsafe { value.CGRectValue() };
    VIEW.with(|v| {
        let v = v.borrow();
        let Some(view) = v.as_ref() else {
            return 0.0;
        };
        // screen coordinates, to the window's, to the view's: the window is
        // not the whole screen in Stage Manager or Split View
        let frame = match view.window() {
            Some(window) => view.convertRect_fromView(window.convertRect_fromWindow(frame, None), None),
            None => frame,
        };
        let bounds = view.bounds();
        let bottom = bounds.origin.y + bounds.size.height;
        crate::touch::keyboard_cover(
            bottom as f32,
            bounds.size.height as f32,
            frame.origin.y as f32,
            frame.size.height as f32,
        )
    })
}

/// An iPhone, not an iPad (nor a Mac running the iPad app).
pub fn phone() -> bool {
    MainThreadMarker::new().is_some_and(|mtm| UIDevice::currentDevice(mtm).userInterfaceIdiom() == UIUserInterfaceIdiom::Phone)
}

/// The app is behind another one (`follow_background`): nothing may be
/// drawn there.
static BACKGROUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn in_background() -> bool {
    BACKGROUND.load(Ordering::Relaxed)
}

/// The keychain as the store of the remembered passwords and of the cache
/// key (credentials.rs, cache_key.rs), before anything asks for an entry.
pub fn credential_store() {
    match apple_native_keyring_store::protected::Store::new() {
        Ok(store) => keyring_core::set_default_store(store),
        // (no log yet at this point of the start: said on the error output;
        // every entry then answers "no store", as on a PC without one)
        Err(e) => eprintln!("the keychain is not available: {e}"),
    }
}

/// Points of the window's bottom under the on-screen keyboard.
pub fn keyboard_inset() -> f32 {
    f32::from_bits(KEYBOARD.load(Ordering::Relaxed))
}

/// A tick under the finger (crate::haptics).
pub fn haptic(strong: bool) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    VIEW.with(|v| {
        if let Some(view) = v.borrow().as_ref() {
            let style = if strong {
                UIImpactFeedbackStyle::Medium
            } else {
                UIImpactFeedbackStyle::Light
            };
            UIImpactFeedbackGenerator::feedbackGeneratorWithStyle_forView(style, view).impactOccurred();
        }
    });
}

/// The pasteboard's change count at our last read or write.
static PASTEBOARD_SEEN: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(-1);

/// The system pasteboard's text when it changed since the viewer last read
/// or wrote it: what another app copied. The system asks the user before
/// handing that over ("Allow Paste"), so this is only called once they have
/// chosen Paste, never to fill a menu nor every frame; the change count
/// itself is free to read. A read the user refuses gives nothing and is not
/// counted as read: the next Paste asks again.
pub fn pasteboard_news() -> Option<String> {
    MainThreadMarker::new()?;
    let board = UIPasteboard::generalPasteboard();
    // SAFETY: plain property reads on the main thread.
    let count = unsafe { board.changeCount() };
    if PASTEBOARD_SEEN.load(Ordering::Relaxed) == count {
        return None;
    }
    // SAFETY: as above.
    let text = unsafe { board.string() }?;
    PASTEBOARD_SEEN.store(count, Ordering::Relaxed);
    Some(text.to_string())
}

/// Whether the system pasteboard holds text, without reading it: the
/// system does not ask the user for that.
pub fn pasteboard_has_text() -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    // SAFETY: a plain property read on the main thread.
    unsafe { UIPasteboard::generalPasteboard().hasStrings() }
}

/// Text copied in the viewer, given to the system's pasteboard.
pub fn set_pasteboard_text(text: &str) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let board = UIPasteboard::generalPasteboard();
    // SAFETY: plain property write and read on the main thread.
    unsafe {
        board.setString(Some(&NSString::from_str(text)));
        PASTEBOARD_SEEN.store(board.changeCount(), Ordering::Relaxed);
    }
}
