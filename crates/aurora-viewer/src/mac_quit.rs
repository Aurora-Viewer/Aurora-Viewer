//! Cmd+Q on macOS asks before quitting, like the window's close button.
//!
//! winit's application menu sends `terminate:` for its Quit item (Cmd+Q):
//! AppKit then ends the process with `ApplicationHandler::exiting` as the
//! only notice, so the viewer neither logged out nor saved its settings and
//! layout. The Legacy Viewer answers `applicationShouldTerminate:` with its
//! own quit (handleQuit, indra/newview/llappdelegate-objc.mm), the same
//! confirmation as closing the window. Here the item sends `performClose:`
//! along the responder chain instead: the key window answers it, winit turns
//! it into `WindowEvent::CloseRequested`, and the exit prompt follows
//! (`App::request_exit`).

use objc2::{MainThreadMarker, sel};
use objc2_app_kit::NSApplication;

/// Points the application menu's Quit item at the window's close. Called
/// once the window exists, on the main thread.
pub fn route_quit_to_close() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("macOS menu: not on the main thread, Quit left as is");
        return;
    };
    let Some(bar) = NSApplication::sharedApplication(mtm).mainMenu() else {
        return;
    };
    for i in 0..bar.numberOfItems() {
        let Some(menu) = bar.itemAtIndex(i).and_then(|item| item.submenu()) else {
            continue;
        };
        for j in 0..menu.numberOfItems() {
            let Some(item) = menu.itemAtIndex(j) else {
                continue;
            };
            if item.action() == Some(sel!(terminate:)) {
                // SAFETY: on the main thread; no target sends the action
                // along the responder chain, where every NSWindow answers
                // performClose:.
                unsafe {
                    item.setTarget(None);
                    item.setAction(Some(sel!(performClose:)));
                }
                return;
            }
        }
    }
}
