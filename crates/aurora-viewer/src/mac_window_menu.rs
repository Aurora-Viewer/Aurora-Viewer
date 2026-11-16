//! The macOS Window menu: Minimize (Command+M) and Enter Full Screen
//! (Control+Command+F), as the Legacy Viewer puts them in Firestorm.xib
//! (`performMiniaturize:` on "m", `toggleFullScreen:` on Control+Command
//! "f"). winit only installs the application menu, so without these items
//! the viewer's key handler sees the chords: Command counts as Control
//! (`keybinds::control_down`), and Command+M opens the World Map while
//! Control+Command+F opens Search.
//!
//! Enter Full Screen asks AppKit for the native full screen of the green
//! button, not the borderless one of the Fullscreen preference: Game Mode
//! turns on there (docs/MACOS.md), and `App::apply_fullscreen_setting`
//! leaves a full screen the system entered by itself alone, as it already
//! does for the green button.

use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::{NSString, ns_string};

/// Adds the Window menu once the window exists, on the main thread.
pub fn install() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("macOS menu: not on the main thread, Window menu left out");
        return;
    };
    let Some(bar) = NSApplication::sharedApplication(mtm).mainMenu() else {
        return;
    };
    // A recreated window (GPU recovery) finds the menu already there.
    for i in 0..bar.numberOfItems() {
        let Some(menu) = bar.itemAtIndex(i).and_then(|item| item.submenu()) else {
            continue;
        };
        for j in 0..menu.numberOfItems() {
            if menu.itemAtIndex(j).and_then(|item| item.action()) == Some(sel!(performMiniaturize:)) {
                return;
            }
        }
    }

    let title = NSString::from_str(aurora_i18n::tr!("app-mac-window-menu"));
    let window_item = NSMenuItem::new(mtm);
    window_item.setTitle(&title);
    let menu = NSMenu::new(mtm);
    menu.setTitle(&title);

    // SAFETY: AppKit objects built on the main thread (MainThreadMarker);
    // both selectors are NSWindow's own actions, reached through the
    // responder chain, so no target is set.
    let minimize = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(aurora_i18n::tr!("app-mac-minimize")),
            Some(sel!(performMiniaturize:)),
            ns_string!("m"),
        )
    };
    let full_screen = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(aurora_i18n::tr!("app-mac-enter-full-screen")),
            Some(sel!(toggleFullScreen:)),
            ns_string!("f"),
        )
    };
    full_screen.setKeyEquivalentModifierMask(NSEventModifierFlags::Control | NSEventModifierFlags::Command);
    menu.addItem(&minimize);
    menu.addItem(&full_screen);
    window_item.setSubmenu(Some(&menu));
    bar.addItem(&window_item);
}
