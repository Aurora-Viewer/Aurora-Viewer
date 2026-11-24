//! The dialogs on UIKit: the document picker, alerts, and the folder saved
//! files go to.
//!
//! UIKit presents from the main thread only, and a dialog answers through
//! the main run loop. A caller on a worker thread hands its dialog to the
//! main queue and waits on a channel. Every way a dialog can end without an
//! answer (no window to present from, a picker closed without telling its
//! delegate) drops the channel's senders, which ends the wait as "cancelled":
//! no caller stays blocked on a dialog that is gone.
//!
//! A caller on the main thread cannot wait, since its own run loop is what
//! answers the dialog, and running that loop from here is not safe under
//! winit 0.30 (`platform_impl/ios`):
//! - its run loop observers are in the default mode, and a nested turn of
//!   the loop in that mode calls them while the app is still inside an event
//!   callback, a state `app_state::handle_main_events_cleared` and
//!   `handle_wakeup_transition` answer with a panic;
//! - the interface runs from `drawRect:` (`RedrawRequested`), in the middle
//!   of a Core Animation commit, which the presentation needs to complete
//!   before anything shows;
//! - on Mac Catalyst the clicks come from AppKit's event queue, which a
//!   nested run loop does not read: the dialog would never be answered.
//!
//! So a picker asked from the main thread is not shown, and neither is an
//! alert with a choice; an alert with a single button, whose answer is known
//! beforehand, is shown without waiting. Each case is logged.

use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::mpsc::{self, Sender};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSArray, NSSearchPathDirectory, NSSearchPathDomainMask, NSSearchPathForDirectoriesInDomains, NSString, NSURL};
use objc2_ui_kit::{
    UIAlertAction, UIAlertActionStyle, UIAlertController, UIAlertControllerStyle, UIApplication, UIDocumentPickerDelegate,
    UIDocumentPickerViewController, UIModalPresentationStyle, UIViewController, UIWindow,
};
use objc2_uniform_type_identifiers::{UTType, UTTypeFolder, UTTypeItem};

use super::Pick;
use crate::{Button, FileDialog, MessageDialog, MessageDialogResult};

/// The folder of the app's Documents that saved files go to.
const EXPORTS: &str = "Exports";

/// A dialog asked while another one is opening or closing waits for that
/// animation: UIKit refuses a presentation during one. This long between two
/// tries, and this many tries (3 s in all) before giving up.
const RETRY_NANOS: i64 = 100_000_000;
const RETRIES: u32 = 30;

pub(crate) fn pick(dialog: &FileDialog, kind: Pick) -> Option<Vec<PathBuf>> {
    // the system's picker shows no title of the app's: it is only logged
    log::debug!("file picker ({kind:?}, {:?})", dialog.title);
    if MainThreadMarker::new().is_some() {
        log::warn!("file picker ({kind:?}) asked from the main thread, which cannot wait for it: not shown");
        return None;
    }
    let extensions = crate::accepted_extensions(&dialog.filters);
    let directory = dialog.directory.clone();
    let (answer, picked) = mpsc::channel();
    on_main("file picker", move |mtm, from| {
        present_picker(mtm, from, kind, &extensions, directory.as_deref(), answer)
    });
    // an error is a picker that went away without an answer (module comment)
    picked.recv().ok().filter(|paths| !paths.is_empty())
}

/// No dialog: the caller writes the file after this returns, and the
/// system's export picker only takes a file that exists. The file goes to
/// `<Documents>/Exports`, which the Files app shows (the app's Info.plist
/// sets `UIFileSharingEnabled` and `LSSupportsOpeningDocumentsInPlace`).
/// The folder `set_directory` names is not used: the app cannot write
/// outside its own container.
pub(crate) fn save(dialog: &FileDialog) -> Option<PathBuf> {
    let name = crate::proposed_name(dialog.file_name.as_deref(), &dialog.filters);
    let Some(documents) = documents() else {
        log::error!("save of {name}: the app has no Documents folder");
        return None;
    };
    let folder = documents.join(EXPORTS);
    if let Err(e) = std::fs::create_dir_all(&folder) {
        log::error!("save of {name}: cannot create {}: {e}", folder.display());
        return None;
    }
    let path = folder.join(crate::unique_name(&name, |candidate| folder.join(candidate).exists()));
    log::info!("save of {name}: no dialog on this system, the file goes to {}", path.display());
    Some(path)
}

pub(crate) fn alert(dialog: &MessageDialog) -> MessageDialogResult {
    let buttons = crate::buttons(&dialog.buttons);
    let unanswered = crate::dismissed(&dialog.buttons);
    let waits = MainThreadMarker::new().is_none();
    if !waits && buttons.len() > 1 {
        log::warn!(
            "{:?} alert \"{}\" asked from the main thread, which cannot wait for its answer: not shown",
            dialog.level,
            dialog.title
        );
        return unanswered;
    }
    log::info!("{:?} alert: {}", dialog.level, dialog.title);
    let (title, description) = (dialog.title.clone(), dialog.description.clone());
    let (answer, answered) = mpsc::channel();
    on_main("alert", move |mtm, from| {
        present_alert(mtm, from, &title, &description, &buttons, &answer)
    });
    if !waits {
        // a single button: what it answers is known, and the main thread
        // goes on while the alert is up
        return unanswered;
    }
    // an error is an alert that went away without a button (module comment)
    answered.recv().unwrap_or(unanswered)
}

/// Runs `present` on the main thread, at a later turn of its run loop (also
/// for a caller that is on it: never from inside an event callback), with
/// the view controller to present from. When there is none, `present` is
/// dropped with the channel it holds.
fn on_main(what: &'static str, present: impl FnOnce(MainThreadMarker, &UIViewController) + Send + 'static) {
    DispatchQueue::main().exec_async(move || show(what, present, 0));
}

fn show(what: &'static str, present: impl FnOnce(MainThreadMarker, &UIViewController) + Send + 'static, attempt: u32) {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("{what}: the main queue did not run on the main thread");
        return;
    };
    match presenter(mtm) {
        Presenter::Ready(from) => present(mtm, &from),
        Presenter::Busy if attempt < RETRIES => {
            let later = DispatchTime::NOW.time(RETRY_NANOS);
            if let Err(e) = DispatchQueue::main().after(later, move || show(what, present, attempt + 1)) {
                log::warn!("{what}: cannot wait for the dialog before it ({e:?}): not shown");
            }
        }
        Presenter::Busy => log::warn!("{what}: the dialog before it is still opening or closing: not shown"),
        Presenter::Missing => log::warn!("{what}: no window to show it in"),
    }
}

enum Presenter {
    Ready(Retained<UIViewController>),
    /// A presentation or a dismissal is under way: try again shortly.
    Busy,
    /// No window, or a window without a view controller.
    Missing,
}

/// The view controller a dialog is presented from: the window's root, then
/// whatever is presented over it, up to the top. A dialog over another one
/// is fine: the one on top is answered first.
fn presenter(mtm: MainThreadMarker) -> Presenter {
    let Some(mut top) = window(mtm).and_then(|window| window.rootViewController()) else {
        return Presenter::Missing;
    };
    while let Some(next) = top.presentedViewController() {
        if next.isBeingPresented() || next.isBeingDismissed() {
            return Presenter::Busy;
        }
        top = next;
    }
    Presenter::Ready(top)
}

/// The window dialogs are shown in: the key one, else the first visible one
/// that has a view controller.
fn window(_mtm: MainThreadMarker) -> Option<Retained<UIWindow>> {
    // SAFETY: `sharedApplication` takes no argument and returns the
    // application object, or nil before `UIApplicationMain` made it. The
    // generated binding takes it as never nil and would panic there.
    let app: Option<Retained<UIApplication>> = unsafe { msg_send![UIApplication::class(), sharedApplication] };
    // winit makes its window without a scene delegate, and this list has the
    // windows of every scene, which is what is wanted here
    #[allow(deprecated)]
    let windows = app?.windows().to_vec();
    let key = windows.iter().find(|window| window.isKeyWindow());
    let visible = || {
        windows
            .iter()
            .find(|window| !window.isHidden() && window.rootViewController().is_some())
    };
    key.or_else(visible).cloned()
}

fn present_picker(
    mtm: MainThreadMarker,
    from: &UIViewController,
    kind: Pick,
    extensions: &[String],
    directory: Option<&Path>,
    answer: Sender<Vec<PathBuf>>,
) {
    let folder = kind == Pick::Folder;
    let types = if folder {
        // SAFETY: a constant of the framework, set when it is loaded and
        // never changed.
        NSArray::from_slice(&[unsafe { UTTypeFolder }])
    } else {
        content_types(extensions)
    };
    // Files come as copies the system makes in the app's temporary folder:
    // plain `std::fs` reads them, with no access to ask for or to give back.
    // A folder cannot be copied: it is opened in place (`did_pick`).
    let picker =
        UIDocumentPickerViewController::initForOpeningContentTypes_asCopy(UIDocumentPickerViewController::alloc(mtm), &types, !folder);
    picker.setAllowsMultipleSelection(kind == Pick::Files);
    if let Some(url) = directory.and_then(NSURL::from_directory_path) {
        picker.setDirectoryURL(Some(&url));
    }
    // (a sheet needs no anchor on an iPad, unlike a popover)
    picker.setModalPresentationStyle(UIModalPresentationStyle::FormSheet);
    let delegate = PickerDelegate::new(mtm, folder, answer);
    picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    keep_with(&picker, &delegate);
    from.presentViewController_animated_completion(&picker, true, None);
}

/// One type per extension; every file when there is no extension to go by,
/// or none the system can make a type of.
fn content_types(extensions: &[String]) -> Retained<NSArray<UTType>> {
    let types: Vec<Retained<UTType>> = extensions
        .iter()
        .filter_map(|extension| UTType::typeWithFilenameExtension(&NSString::from_str(extension)))
        .collect();
    if types.is_empty() {
        // SAFETY: a constant of the framework, set when it is loaded and
        // never changed.
        return NSArray::from_slice(&[unsafe { UTTypeItem }]);
    }
    NSArray::from_retained_slice(&types)
}

/// A picker holds its delegate weakly. This makes the picker own it, so that
/// the delegate lives exactly as long as the picker: until it answers, and
/// no longer than a picker that goes away without answering, whose end drops
/// the delegate's sender and frees the waiting caller.
fn keep_with(picker: &UIDocumentPickerViewController, delegate: &PickerDelegate) {
    static KEY: u8 = 0;
    let picker: *const UIDocumentPickerViewController = picker;
    let delegate: *const PickerDelegate = delegate;
    // SAFETY: both pointers come from live references to Objective-C
    // objects, on the main thread both belong to. The key is the address of
    // a static, which nothing else uses on this object. With this policy the
    // runtime retains the delegate and releases it when the picker is freed.
    unsafe {
        objc2::ffi::objc_setAssociatedObject(
            picker.cast_mut().cast::<AnyObject>(),
            (&raw const KEY).cast(),
            delegate.cast_mut().cast::<AnyObject>(),
            objc2::ffi::OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );
    }
}

struct PickerState {
    folder: bool,
    answer: Sender<Vec<PathBuf>>,
}

define_class!(
    // SAFETY:
    // - `NSObject` has no subclassing requirements.
    // - `PickerDelegate` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AuroraDocumentPickerDelegate"]
    #[ivars = PickerState]
    struct PickerDelegate;

    unsafe impl NSObjectProtocol for PickerDelegate {}

    unsafe impl UIDocumentPickerDelegate for PickerDelegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn did_pick(&self, _picker: &UIDocumentPickerViewController, urls: &NSArray<NSURL>) {
            let state = self.ivars();
            let mut paths = Vec::new();
            for url in urls.to_vec() {
                if state.folder {
                    // The folder is the user's own, outside the app's
                    // container: this opens it to the app, with what it
                    // holds. The access is never given back: the caller
                    // reads and writes the folder with `std::fs` after the
                    // dialog, and has no way to say when it is done. It
                    // lasts until the app quits, at the cost of one kernel
                    // resource per folder picked.
                    // SAFETY: the method takes no argument and returns a
                    // BOOL; the URL is the one the picker gave, which is the
                    // object that carries the access.
                    let opened = unsafe { url.startAccessingSecurityScopedResource() };
                    if !opened {
                        // (a folder of the app's own container needs none)
                        log::debug!("file picker: the folder came without an access to open");
                    }
                }
                match url.to_file_path() {
                    // (without the `/` a folder's URL ends with)
                    Some(path) => paths.push(path.components().collect()),
                    None => log::warn!("file picker: a choice has no path on the disk, left out"),
                }
            }
            log::debug!("file picker: {} chosen", paths.len());
            let _ = state.answer.send(paths);
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn was_cancelled(&self, _picker: &UIDocumentPickerViewController) {
            let _ = self.ivars().answer.send(Vec::new());
        }
    }
);

impl PickerDelegate {
    fn new(mtm: MainThreadMarker, folder: bool, answer: Sender<Vec<PathBuf>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PickerState { folder, answer });
        // SAFETY: `NSObject`'s `init` takes no argument and returns the
        // object it is given, initialised.
        unsafe { msg_send![super(this), init] }
    }
}

/// The alert owns its buttons' handlers, and each handler a sender: when the
/// alert goes away, answered or not, the last sender goes with it.
fn present_alert(
    mtm: MainThreadMarker,
    from: &UIViewController,
    title: &str,
    description: &str,
    buttons: &[Button],
    answer: &Sender<MessageDialogResult>,
) {
    let text = |text: &str| (!text.is_empty()).then(|| NSString::from_str(text));
    let (title, description) = (text(title), text(description));
    let alert = UIAlertController::alertControllerWithTitle_message_preferredStyle(
        title.as_deref(),
        description.as_deref(),
        UIAlertControllerStyle::Alert,
        mtm,
    );
    // UIKit throws on a second cancel button
    let mut cancel_used = false;
    for button in buttons {
        let style = if button.cancel && !cancel_used {
            cancel_used = true;
            UIAlertActionStyle::Cancel
        } else {
            UIAlertActionStyle::Default
        };
        let (result, answer) = (button.result.clone(), answer.clone());
        let handler = RcBlock::new(move |_: NonNull<UIAlertAction>| {
            let _ = answer.send(result.clone());
        });
        let label = NSString::from_str(&button.label);
        alert.addAction(&UIAlertAction::actionWithTitle_style_handler(
            Some(&label),
            style,
            Some(&handler),
            mtm,
        ));
    }
    from.presentViewController_animated_completion(&alert, true, None);
}

/// The app's Documents folder: its own on an iPad, the user's on a Mac
/// (Catalyst, outside the sandbox).
fn documents() -> Option<PathBuf> {
    // (worker threads have no pool of their own for what Foundation returns)
    autoreleasepool(|_| {
        let found = NSSearchPathForDirectoriesInDomains(
            NSSearchPathDirectory::DocumentDirectory,
            NSSearchPathDomainMask::UserDomainMask,
            true,
        );
        found.firstObject().map(|path| PathBuf::from(path.to_string()))
    })
}
