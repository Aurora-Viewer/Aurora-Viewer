//! One text rule for the names an object brings into a dialog (its own name
//! and its owner's, the item it gives): the script menus and text boxes,
//! the permission and caution questions and their chat lines, the
//! experience questions, the link offers and the gifts of objects.
//!
//! Such a name sits inside a sentence ("“Box” owned by Jane requests…"),
//! one line of a title, or a paragraph laid out by `ui/bidi.rs`. A line
//! break or another control character in it is shown as one space, so the
//! name stays on its line; the explicit bidi embeddings, overrides and
//! isolates (U+202A–U+202E, U+2066–U+2069) are removed, so the name never
//! reorders the sentence around it. Right-to-left letters and the marks
//! (U+200E, U+200F, U+061C) stay: an Arabic or Hebrew name reads as before.
//!
//! The rule applies where a name is shown, never to the name itself: the
//! block list, the replies to the simulator and the chat keep the name as
//! the simulator sent it. The Legacy Viewer substitutes these names as they
//! are (LLNotification substitutions in process_script_dialog,
//! process_script_question, process_load_url in
//! indra/newview/llviewermessage.cpp); it has no bidi layout, so only its
//! line breaks show.

use std::borrow::Cow;

/// Removed: they change the order of what follows them.
fn bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Shown as a space: line and paragraph breaks, tabs and the other C0 and
/// C1 control characters (NEL included).
fn breaks_line(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
}

/// `name` as a dialog shows it: on one line, in its own order. A run of
/// line breaks and controls is one space, none at the ends nor next to a
/// space already there.
pub fn shown(name: &str) -> Cow<'_, str> {
    if !name.chars().any(|c| bidi_control(c) || breaks_line(c)) {
        return Cow::Borrowed(name);
    }
    let mut out = String::with_capacity(name.len());
    let mut space = false;
    for c in name.chars() {
        if bidi_control(c) {
            continue;
        }
        if breaks_line(c) {
            space = true;
            continue;
        }
        if space && !c.is_whitespace() && out.chars().next_back().is_some_and(|last| !last.is_whitespace()) {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_names_are_kept_as_they_are() {
        for name in ["Box", "  Spaced  box  ", "✓ Sélection", "Jane Doe", ""] {
            assert!(matches!(shown(name), Cow::Borrowed(n) if n == name), "{name:?}");
        }
    }

    #[test]
    fn line_breaks_and_controls_become_one_space() {
        assert_eq!(shown("Box\nowned by Linden Lab"), "Box owned by Linden Lab");
        assert_eq!(shown("Box\r\n\r\n• Take your L$"), "Box • Take your L$");
        assert_eq!(shown("Box\u{2028}Line\u{2029}Paragraph"), "Box Line Paragraph");
        assert_eq!(shown("Box\u{85}NEL\tTab\u{0}Nul\u{7F}Del\u{9B}Csi"), "Box NEL Tab Nul Del Csi");
        // none at the ends, none doubled next to a space
        assert_eq!(shown("\n\nBox\n"), "Box");
        assert_eq!(shown("Box \n owner"), "Box  owner");
        assert_eq!(shown("Box\n owner"), "Box owner");
        assert_eq!(shown("\r\n\t"), "");
    }

    #[test]
    fn bidi_embeddings_overrides_and_isolates_are_removed() {
        // the override would show the reversed letters as "Linden Lab"
        assert_eq!(shown("Box\u{202E}baL nedniL\u{202C}"), "BoxbaL nedniL");
        for c in [
            '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            assert_eq!(shown(&format!("a{c}b")), "ab", "{:04X}", c as u32);
        }
        // an isolate closed early, then an override to the end of the line
        assert_eq!(shown("Box\u{2069}\u{202E} Resident\n"), "Box Resident");
    }

    #[test]
    fn right_to_left_names_and_marks_stay() {
        for name in ["صندوق الهدايا", "קופסה", "Box \u{200F}קופסה\u{200E}", "عدد ١٢٣", "\u{061C}مرحبا"] {
            assert!(matches!(shown(name), Cow::Borrowed(n) if n == name), "{name:?}");
        }
        assert_eq!(shown("صندوق\nالهدايا\u{202B}"), "صندوق الهدايا");
    }
}
