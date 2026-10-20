//! The Dev Toolbox (Advanced › Dev Toolbox, Ctrl+Alt+D): the viewer's debug
//! tools in one window, each with a stable identifier (J2, L1, R3…) that an
//! agent or a developer can ask a tester to press, so that a grid report
//! comes with the few lines that explain it instead of a whole log. This
//! module holds what does not depend on egui: the tools, the layout the
//! user arranges, the anonymisation of what is copied and the header of a
//! copy. The window is `ui/dev_toolbox.rs`, the tools run in
//! `app/dev_toolbox.rs`; the agents' guide is docs/DEV_TOOLBOX.md.

pub mod anonymize;
pub mod legend;
pub mod tools;

/// GitHub refuses a comment longer than this (characters): a longer copy
/// is offered as a .txt file instead.
pub const GITHUB_COMMENT_LIMIT: usize = 65_536;

/// Where the per-target tools look (the window's target bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Target {
    /// The nearest other avatar within 3 m, no need to aim at it.
    #[default]
    Nearest,
    /// The build tools' selection (right-click › Edit).
    Selection,
    /// The surface under the cursor, frozen by a click.
    UnderCursor,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Nearest, Target::Selection, Target::UnderCursor];

    /// Its name in the header of a copy (English, like the log).
    pub fn code(self) -> &'static str {
        match self {
            Target::Nearest => "nearest avatar",
            Target::Selection => "selection",
            Target::UnderCursor => "under cursor",
        }
    }
}

/// The first line of every copy: which build and which tool wrote it, so an
/// agent reading it knows which code to look at.
pub fn header(version: &str, tool: &str, target: Option<Target>, names_hidden: bool) -> String {
    let mut line = format!("Aurora {version} · {tool}");
    if let Some(target) = target {
        line.push_str(" · target: ");
        line.push_str(target.code());
    }
    if names_hidden {
        line.push_str(" · names hidden");
    }
    line
}

/// Whether a copy can be pasted in a GitHub comment as it is.
pub fn fits_comment(text: &str) -> bool {
    text.chars().count() <= GITHUB_COMMENT_LIMIT
}

/// The name proposed when a copy is saved as a file.
pub fn file_name(tool: &str, unix_seconds: u64) -> String {
    format!("aurora-{}-{unix_seconds}.txt", tool.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_names_build_tool_and_target() {
        assert_eq!(
            header("0.7.2+12.g1a2b3c", "J2", Some(Target::Nearest), true),
            "Aurora 0.7.2+12.g1a2b3c · J2 · target: nearest avatar · names hidden"
        );
        assert_eq!(header("0.7.2", "J4", None, false), "Aurora 0.7.2 · J4");
    }

    #[test]
    fn long_copies_do_not_fit_a_comment() {
        assert!(fits_comment("short"));
        assert!(!fits_comment(&"x".repeat(GITHUB_COMMENT_LIMIT + 1)));
        assert_eq!(file_name("J2", 42), "aurora-j2-42.txt");
    }
}
