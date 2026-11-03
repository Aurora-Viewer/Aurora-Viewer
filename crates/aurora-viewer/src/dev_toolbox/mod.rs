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

use uuid::Uuid;

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
    /// Our own avatar and what it wears: a tester wearing the item that
    /// renders wrong runs the tools on themselves.
    MyAvatar,
}

impl Target {
    /// In the order of `AURORA_DEMO_TOOLBOX`'s target numbers (0 to 3).
    pub const ALL: [Target; 4] = [Target::Nearest, Target::Selection, Target::UnderCursor, Target::MyAvatar];
    /// The target bar's order: the two avatars, then the objects.
    pub const BAR: [Target; 4] = [Target::Nearest, Target::MyAvatar, Target::Selection, Target::UnderCursor];

    /// Its name in the header of a copy (English, like the log).
    pub fn code(self) -> &'static str {
        match self {
            Target::Nearest => "nearest avatar",
            Target::Selection => "selection",
            Target::UnderCursor => "under cursor",
            Target::MyAvatar => "my avatar",
        }
    }
}

/// The avatar a per-target tool describes: `me` for my avatar, the nearest
/// other one within 3 m, or the wearer of the selected or pointed-at object
/// (`wearer`, that avatar itself when an avatar is the object).
pub fn target_avatar(target: Target, me: Uuid, nearest: Option<Uuid>, wearer: Option<Uuid>) -> Option<Uuid> {
    match target {
        Target::MyAvatar => (!me.is_nil()).then_some(me),
        Target::Nearest => nearest,
        Target::Selection | Target::UnderCursor => wearer,
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
    fn my_avatar_is_the_target_without_anything_else() {
        let me = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        assert_eq!(target_avatar(Target::MyAvatar, me, None, None), Some(me));
        assert_eq!(target_avatar(Target::MyAvatar, me, Some(other), Some(other)), Some(me));
        assert_eq!(target_avatar(Target::MyAvatar, Uuid::nil(), None, None), None, "before login");
        assert_eq!(target_avatar(Target::Nearest, me, Some(other), None), Some(other));
        assert_eq!(target_avatar(Target::Nearest, me, None, Some(other)), None);
        assert_eq!(target_avatar(Target::Selection, me, Some(other), None), None);
        assert_eq!(
            header("0.8.3", "J9", Some(Target::MyAvatar), true),
            "Aurora 0.8.3 · J9 · target: my avatar · names hidden"
        );
    }

    #[test]
    fn the_demo_target_numbers_keep_their_meaning() {
        assert_eq!(Target::ALL[0], Target::Nearest);
        assert_eq!(Target::ALL[1], Target::Selection);
        assert_eq!(Target::ALL[2], Target::UnderCursor);
        assert_eq!(Target::ALL[3], Target::MyAvatar);
        assert_eq!(Target::default(), Target::Nearest);
        assert!(Target::ALL.iter().all(|t| Target::BAR.contains(t)));
    }

    #[test]
    fn long_copies_do_not_fit_a_comment() {
        assert!(fits_comment("short"));
        assert!(!fits_comment(&"x".repeat(GITHUB_COMMENT_LIMIT + 1)));
        assert_eq!(file_name("J2", 42), "aurora-j2-42.txt");
    }
}
