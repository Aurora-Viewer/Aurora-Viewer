//! When the renderer cannot be created at startup (no graphics card can
//! draw the window, Vulkan missing), the viewer used to log it and exit
//! with its window still hidden: nothing on screen, no crash report, since
//! it is no crash (#894, a tester with the integrated GPU disabled in
//! Device Manager). It now says so in a native message box, with what to
//! try and where the log is. The Legacy Viewer does the same when its
//! renderer cannot start: `LLUserWarningMsg::show(MBVideoDrvErr)` in
//! LLViewerWindow's constructor (indra/newview/llviewerwindow.cpp), a native
//! box asking to update or reinstall the video driver.

/// The message box's title and text for that failure.
pub(super) fn text(error: &str, log: &std::path::Path) -> (String, String) {
    (
        aurora_i18n::tr!("app-graphics-start-failed-title").to_string(),
        aurora_i18n::tr!("app-graphics-start-failed", e = error.to_string(), log = log.display().to_string()).to_string(),
    )
}

/// Tell the user the viewer cannot start, and wait for them to close the
/// message. Not in automated captures, which have no one to read it.
pub(super) fn report(error: &str, automated: bool) {
    let (title, body) = text(error, &crate::logging::log_path());
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
        let (title, body) = super::text("no Vulkan adapter can present to the window", log);
        assert!(!title.is_empty());
        assert!(body.contains("no Vulkan adapter can present to the window"));
        assert!(body.contains("aurora.log"));
        // one paragraph per advice, not a single line
        assert!(body.lines().count() >= 5);
    }
}
