//! File and message dialogs on iPadOS.
//!
//! The desktop builds call the `rfd` crate, which has no iOS backend. The
//! viewer and the media host depend on this crate under `rfd`'s name on
//! iOS (their Cargo.toml), so that every call site stays as it is: this
//! crate gives the calls they make, with the same names and results, and
//! shows the system's document picker and alerts instead.
//!
//! The dialogs block their caller until the user answers, as `rfd`'s do.
//! UIKit only presents from the main thread, so a call made from a worker
//! thread hands the dialog to the main thread and waits for it. A call made
//! on the main thread itself cannot wait for its own run loop: a picker is
//! not shown and answers "cancelled", an alert is shown without waiting when
//! it has a single button and not shown otherwise (see `backend::uikit`).
//!
//! What differs from the desktop for the caller:
//! - a picked file is a copy the system made in the app's temporary folder,
//!   so its path is not where the user keeps the original;
//! - a picked folder stays readable and writable until the app quits, not
//!   after: a path kept in the settings needs picking again;
//! - a saved file has no dialog: it goes to the app's "Exports" folder, which
//!   the Files app shows.

use std::path::{Path, PathBuf};

mod backend;

/// One line of the "file type" choice: a name and its extensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Filter {
    pub name: String,
    pub extensions: Vec<String>,
}

/// A file picker, built like `rfd::FileDialog`.
#[derive(Debug, Clone, Default)]
pub struct FileDialog {
    pub(crate) filters: Vec<Filter>,
    pub(crate) directory: Option<PathBuf>,
    pub(crate) file_name: Option<String>,
    pub(crate) title: Option<String>,
}

impl FileDialog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Files offered: a name for the group and its extensions, without dot.
    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.filters.push(Filter {
            name: name.into(),
            extensions: extensions.iter().map(|e| e.to_string()).collect(),
        });
        self
    }

    /// Folder the picker opens in, when the system allows it.
    pub fn set_directory<P: AsRef<Path>>(mut self, path: P) -> Self {
        let path = path.as_ref();
        self.directory = (!path.as_os_str().is_empty()).then(|| path.to_path_buf());
        self
    }

    /// Name proposed for a saved file.
    pub fn set_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }

    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// One file, or `None` when the user cancels.
    pub fn pick_file(self) -> Option<PathBuf> {
        backend::pick(&self, backend::Pick::File).and_then(|mut paths| paths.pop())
    }

    /// Several files, or `None` when the user cancels.
    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        backend::pick(&self, backend::Pick::Files).filter(|paths| !paths.is_empty())
    }

    /// One folder, or `None` when the user cancels.
    pub fn pick_folder(self) -> Option<PathBuf> {
        backend::pick(&self, backend::Pick::Folder).and_then(|mut paths| paths.pop())
    }

    /// Where to write a file. No dialog is shown: the caller writes the file
    /// after this returns, so the system's export picker, which takes a file
    /// that already exists, cannot be offered here. The path is in the
    /// "Exports" folder of the app's Documents, which the Files app shows
    /// under "On My iPad"; a name already taken there gets a number
    /// (`unique_name`). `None` only when that folder cannot be made.
    pub fn save_file(self) -> Option<PathBuf> {
        backend::save(&self)
    }
}

/// The extensions a picker accepts, lower case and without duplicates; empty
/// when no filter was given or one of them takes every file (`*`).
pub(crate) fn accepted_extensions(filters: &[Filter]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for ext in filters.iter().flat_map(|f| &f.extensions) {
        let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        if ext == "*" || ext.is_empty() {
            return Vec::new();
        }
        if !out.contains(&ext) {
            out.push(ext);
        }
    }
    out
}

/// The name a saved file is proposed under: the caller's, with the first
/// filter's extension when it has none, and without anything that would
/// take it out of its folder.
pub(crate) fn proposed_name(file_name: Option<&str>, filters: &[Filter]) -> String {
    let base = file_name.unwrap_or("").rsplit(['/', '\\']).next().unwrap_or("").trim();
    let base = if base.is_empty() || base == "." || base == ".." {
        "Untitled"
    } else {
        base
    };
    let ext = accepted_extensions(filters).into_iter().next();
    match ext {
        Some(ext) if Path::new(base).extension().is_none() => format!("{base}.{ext}"),
        _ => base.to_string(),
    }
}

/// A name no file of the folder has: `name` itself, else `name (2).ext`,
/// `name (3).ext`… `taken` tells whether the folder has a file of that name.
#[cfg(any(target_os = "ios", test))]
pub(crate) fn unique_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = path.extension().and_then(|e| e.to_str());
    (2..=u32::MAX)
        .map(|n| match ext {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        })
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| name.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageLevel {
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MessageButtons {
    #[default]
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
    OkCustom(String),
    OkCancelCustom(String, String),
    YesNoCancelCustom(String, String, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageDialogResult {
    Yes,
    No,
    Ok,
    Cancel,
    Custom(String),
}

/// What a button of an alert answers, and its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Button {
    pub label: String,
    pub result: MessageDialogResult,
    /// The button that dismisses without acting (UIKit sets it apart).
    pub cancel: bool,
}

/// The buttons of an alert, in the order they are shown.
pub(crate) fn buttons(kind: &MessageButtons) -> Vec<Button> {
    use MessageDialogResult as R;
    let b = |label: &str, result: R, cancel: bool| Button {
        label: label.to_string(),
        result,
        cancel,
    };
    let custom = |label: &String, cancel: bool| Button {
        label: label.clone(),
        result: R::Custom(label.clone()),
        cancel,
    };
    match kind {
        // (the viewer only asks for the first, which the three languages of
        // the interface write alike; the others are rfd's own English)
        MessageButtons::Ok => vec![b("OK", R::Ok, false)],
        MessageButtons::OkCancel => vec![b("OK", R::Ok, false), b("Cancel", R::Cancel, true)],
        MessageButtons::YesNo => vec![b("Yes", R::Yes, false), b("No", R::No, true)],
        MessageButtons::YesNoCancel => vec![b("Yes", R::Yes, false), b("No", R::No, false), b("Cancel", R::Cancel, true)],
        MessageButtons::OkCustom(ok) => vec![custom(ok, false)],
        MessageButtons::OkCancelCustom(ok, cancel) => vec![custom(ok, false), custom(cancel, true)],
        MessageButtons::YesNoCancelCustom(yes, no, cancel) => vec![custom(yes, false), custom(no, false), custom(cancel, true)],
    }
}

/// What an alert answers when it cannot be shown or is dismissed without a
/// button: its cancel button, or its only button.
pub(crate) fn dismissed(kind: &MessageButtons) -> MessageDialogResult {
    let all = buttons(kind);
    all.iter()
        .find(|b| b.cancel)
        .or(all.last())
        .map(|b| b.result.clone())
        .unwrap_or(MessageDialogResult::Cancel)
}

/// An alert, built like `rfd::MessageDialog`.
#[derive(Debug, Clone, Default)]
pub struct MessageDialog {
    pub(crate) level: MessageLevel,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) buttons: MessageButtons,
}

impl MessageDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_level(mut self, level: MessageLevel) -> Self {
        self.level = level;
        self
    }

    pub fn set_title(mut self, text: impl Into<String>) -> Self {
        self.title = text.into();
        self
    }

    pub fn set_description(mut self, text: impl Into<String>) -> Self {
        self.description = text.into();
        self
    }

    pub fn set_buttons(mut self, btn: MessageButtons) -> Self {
        self.buttons = btn;
        self
    }

    pub fn show(self) -> MessageDialogResult {
        backend::alert(&self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(name: &str, ext: &[&str]) -> Filter {
        Filter {
            name: name.to_string(),
            extensions: ext.iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn the_accepted_extensions_are_cleaned_and_a_star_takes_everything() {
        let f = [filter("Images", &["PNG", ".jpg", "png"]), filter("Models", &["dae"])];
        assert_eq!(accepted_extensions(&f), ["png", "jpg", "dae"]);
        assert!(accepted_extensions(&[]).is_empty());
        assert!(accepted_extensions(&[filter("Images", &["png"]), filter("All", &["*"])]).is_empty());
    }

    #[test]
    fn a_saved_file_keeps_its_name_inside_its_folder() {
        let f = [filter("Images", &["png"])];
        assert_eq!(proposed_name(Some("photo"), &f), "photo.png");
        assert_eq!(proposed_name(Some("photo.jpg"), &f), "photo.jpg");
        assert_eq!(proposed_name(Some("../../etc/passwd"), &f), "passwd.png");
        assert_eq!(proposed_name(Some("a\\b\\c.txt"), &[]), "c.txt");
        assert_eq!(proposed_name(None, &f), "Untitled.png");
        assert_eq!(proposed_name(Some(".."), &[]), "Untitled");
    }

    #[test]
    fn a_saved_file_never_takes_the_name_of_another() {
        let files = ["photo.png", "photo (2).png", "notes", "a.tar.gz", ".hidden"];
        let taken = |name: &str| files.contains(&name);
        assert_eq!(unique_name("new.png", taken), "new.png");
        assert_eq!(unique_name("photo.png", taken), "photo (3).png");
        assert_eq!(unique_name("notes", taken), "notes (2)");
        assert_eq!(unique_name("a.tar.gz", taken), "a.tar (2).gz");
        assert_eq!(unique_name(".hidden", taken), ".hidden (2)");
    }

    #[test]
    fn an_alert_dismissed_without_a_button_answers_its_cancel() {
        assert_eq!(dismissed(&MessageButtons::Ok), MessageDialogResult::Ok);
        assert_eq!(dismissed(&MessageButtons::OkCancel), MessageDialogResult::Cancel);
        assert_eq!(dismissed(&MessageButtons::YesNo), MessageDialogResult::No);
        assert_eq!(dismissed(&MessageButtons::YesNoCancel), MessageDialogResult::Cancel);
        assert_eq!(
            dismissed(&MessageButtons::OkCancelCustom("Keep".into(), "Drop".into())),
            MessageDialogResult::Custom("Drop".into())
        );
        let labels: Vec<_> = buttons(&MessageButtons::YesNoCancel).into_iter().map(|b| b.label).collect();
        assert_eq!(labels, ["Yes", "No", "Cancel"]);
    }
}
