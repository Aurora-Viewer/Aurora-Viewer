//! A modeless NSOpenPanel keeps the importer reachable. Only its retained
//! panel is cancelled, on AppKit's main thread; no application-wide abort.
use super::{Cancellation, Kind, PathBuf, Request};
use block2::RcBlock;
use dispatch2::{MainThreadBound, run_on_main};
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{NSArray, NSString};
use std::{cell::Cell, rc::Rc};

pub(super) fn select(request: Request, cancellation: Cancellation) -> Vec<PathBuf> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let opening = cancellation.clone();
    let panel = run_on_main(move |mtm| {
        // The importer may have closed while this block waited for AppKit.
        if opening.cancelled() {
            return None;
        }
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setMessage(Some(&NSString::from_str(&request.title)));
        panel.setCanChooseDirectories(request.kind == Kind::Folder);
        panel.setCanChooseFiles(request.kind != Kind::Folder);
        panel.setAllowsMultipleSelection(request.kind == Kind::Files);
        if !request.extensions.is_empty() {
            let extensions: Vec<_> = request.extensions.iter().map(|ext| NSString::from_str(ext)).collect();
            // Keep extension-based filtering identical to rfd, including j2c
            // and anim, which need not have a registered system content type.
            #[allow(deprecated)]
            panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&extensions)));
        }
        let finished = Rc::new(Cell::new(false));
        let (selected_panel, completed) = (panel.clone(), finished.clone());
        let completion = RcBlock::new(move |response| {
            completed.set(true);
            let paths = if response == NSModalResponseOK && !opening.cancelled() {
                selected_panel
                    .URLs()
                    .iter()
                    .filter_map(|url| url.path().map(|path| PathBuf::from(path.to_string())))
                    .collect()
            } else {
                Vec::new()
            };
            let _ = tx.send(paths);
        });
        // Unlike runModal(), this leaves Aurora's event loop and other windows
        // usable, so the user can close the import window that owns this panel.
        panel.beginWithCompletionHandler(&completion);
        Some(MainThreadBound::new((panel, finished), mtm))
    });
    let Some(panel) = panel else { return Vec::new() };
    super::wait_for_selection(&rx, &cancellation, || {
        panel.get_on_main(|(panel, finished)| {
            if !finished.get() {
                // SAFETY: the retained panel belongs to this request, AppKit
                // runs this closure on the main thread, and nil is a valid sender.
                unsafe { panel.cancel(None) };
            }
        });
    })
    .unwrap_or_default()
}
