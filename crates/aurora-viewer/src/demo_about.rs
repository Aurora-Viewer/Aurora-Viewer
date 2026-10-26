//! AURORA_DEMO_ABOUT=info|licences[:end]: Help › About Aurora Viewer opens
//! on its Info tab (the demo region's simulator, the computer read in the
//! background) or on its Licences tab; `:end` scrolls the tab to its end.

use crate::ui::about::Tab;

/// The tab asked for, if any, and whether to scroll it to its end.
pub fn tab() -> Option<(Tab, bool)> {
    let mode = std::env::var("AURORA_DEMO_ABOUT").ok()?.trim().to_ascii_lowercase();
    let (tab, end) = match mode.split_once(':') {
        Some((tab, rest)) => (tab.to_owned(), rest == "end"),
        None => (mode, false),
    };
    let tab = match tab.as_str() {
        "licences" | "licenses" => Tab::Licences,
        _ => Tab::Info,
    };
    Some((tab, end))
}

/// The frame the window opens at.
pub const OPEN: u64 = 600;
