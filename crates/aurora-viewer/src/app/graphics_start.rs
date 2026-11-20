//! When the renderer cannot be created at startup (no graphics card can
//! draw the window, Vulkan missing), the viewer used to log it and exit
//! with its window still hidden: nothing on screen, no crash report, since
//! it is no crash (#894, a tester with the integrated GPU disabled in
//! Device Manager). It now says so in a native message box, with what to
//! try and where the log is. The Legacy Viewer does the same when its
//! renderer cannot start: `LLUserWarningMsg::show(MBVideoDrvErr)` in
//! LLViewerWindow's constructor (indra/newview/llviewerwindow.cpp), a native
//! box asking to update or reinstall the video driver.
//!
//! A device lost during the start is no missing card but a crash, which ended
//! in a Rust panic (#1892): the same box, and a crash report after it.
//!
//! A card whose video memory is full before the window's interface can be
//! drawn (another viewer or a game holding it, #3867) gets a box of its own:
//! no driver to update there, only programs to close. Before, egui-wgpu's
//! renderer panicked on its first refused buffer.

/// The message box's title and text for that failure; `memory_full`: the
/// card had no video memory left for the viewer.
pub(super) fn text(error: &str, memory_full: bool, log: &std::path::Path) -> (String, String) {
    let log = log.display().to_string();
    let body = if memory_full {
        aurora_i18n::tr!("app-graphics-start-memory-full", log = log)
    } else {
        aurora_i18n::tr!("app-graphics-start-failed", e = error.to_string(), log = log)
    };
    (aurora_i18n::tr!("app-graphics-start-failed-title").to_string(), body.to_string())
}

/// Tell the user the viewer cannot start, and wait for them to close the
/// message. Not in automated captures, which have no one to read it.
pub(super) fn report(error: &str, memory_full: bool, automated: bool) {
    let (title, body) = text(error, memory_full, &crate::logging::log_path());
    if automated {
        return;
    }
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(body)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_message_names_the_error_and_the_log() {
        let log = std::path::Path::new("C:/logs/aurora.log");
        let (title, body) = super::text("Vulkan lists no graphics card", false, log);
        assert!(!title.is_empty());
        assert!(body.contains("Vulkan lists no graphics card"));
        assert!(body.contains("aurora.log"));
        assert!(body.contains("AMD"));
        assert!(body.contains("NVIDIA"));
        // one paragraph per advice, not a single line
        assert!(body.lines().count() >= 5);
    }

    #[test]
    fn a_full_video_memory_says_what_to_close_not_which_driver_to_update() {
        let log = std::path::Path::new("C:/logs/aurora.log");
        let error = "the graphics card's video memory is full";
        let (_, body) = super::text(error, true, log);
        // in every language: checks on the text's own words would only
        // hold for one
        assert_ne!(body, super::text(error, false, log).1);
        assert!(!body.contains(error), "its own text, not the error's: {body}");
        assert!(body.contains("aurora.log"));
        assert!(!body.contains("NVIDIA"), "no driver advice: {body}");
        // one paragraph per advice, not a single line
        assert!(body.lines().count() >= 5);
    }
}
