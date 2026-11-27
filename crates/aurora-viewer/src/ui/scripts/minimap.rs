//! The minimap at the right of the source: one bar per line, from its
//! indentation to its last character, in the colour of what the line starts
//! with -- the script's shape and colours in a column, as an IDE's map.
//!
//! Aurora's own (the Legacy Viewer draws no minimap): it takes the place of
//! the thin overview strip when the setting is on (`ScriptSettings::minimap`). The clicks and the drag are the
//! window's (`view::minimap`).

use super::Lang;

#[cfg(test)]
use super::indent;
use super::settings::ColorRole;
use super::syntax;

/// The columns the map spans: a longer line stops at its right edge.
pub(super) const COLUMNS: f32 = 80.0;

/// One line of the map.
#[derive(Clone, Copy)]
pub(super) struct Line {
    /// Its number in the source, from 0.
    pub number: usize,
    /// The columns of its bar, from its indentation to its last character:
    /// none for a blank line, which the map leaves empty.
    pub bar: Option<(f32, f32)>,
    /// What its first character is coloured as.
    pub role: ColorRole,
}

/// A script's map.
pub(super) struct Map {
    pub total: usize,
    pub lines: Vec<Line>,
}

/// The map of `text`: every line of the source holds its place, blank lines
/// empty, so that the map's height is the script's.
pub(super) fn map(text: &str, lang: Lang, tab_size: usize) -> Map {
    let highlight = syntax::highlight(text, lang);
    let mut lines = Vec::new();
    let mut start = 0;
    // the highlighter's ranges come in the text's order, as the lines' first
    // characters do: one cursor walks them, where a search of the whole list
    // per line made the map cost the square of the script
    let mut range = 0;
    for (number, line) in text.split('\n').enumerate() {
        let line = line.trim_end_matches('\r');
        let trimmed = line.trim_end();
        let content = trimmed.trim_start();
        let bar = if content.is_empty() {
            None
        } else {
            let lead = &trimmed[..trimmed.len() - content.len()];
            let from = columns(lead, tab_size);
            Some((from, from + columns(content, tab_size)))
        };
        // the colour of the line's first character, from the highlighter
        let at = start + trimmed.len() - content.len();
        while range < highlight.len() && highlight[range].0.end <= at {
            range += 1;
        }
        let role = highlight
            .get(range)
            .filter(|(span, _)| span.contains(&at))
            .map_or(ColorRole::Text, |(_, role)| *role);
        lines.push(Line { number, bar, role });
        start += line.len() + 1;
    }
    Map { total: lines.len(), lines }
}

/// The columns of a run of text, a tab as wide as the editor draws it.
fn columns(text: &str, tab_size: usize) -> f32 {
    let tab = tab_size.max(1) as f32;
    text.chars().map(|ch| if ch == '\t' { tab } else { 1.0 }).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(map: &Map) -> Vec<(usize, Option<(f32, f32)>, ColorRole)> {
        map.lines.iter().map(|line| (line.number, line.bar, line.role)).collect()
    }

    #[test]
    fn a_bar_runs_from_the_indentation_to_the_last_character() {
        let map = map(
            "integer count = 0;\ndefault\n{\n    llSay(0, \"x\");\n}\n",
            Lang::Lsl,
            indent::TAB_SIZE,
        );
        assert_eq!(map.total, 6);
        assert_eq!(bars(&map)[0], (0, Some((0.0, 18.0)), ColorRole::Type));
        assert_eq!(bars(&map)[3].1, Some((4.0, 18.0)), "indented by four");
        assert_eq!(bars(&map)[3].2, ColorRole::Function, "what it starts with");
        // a blank line holds its place, empty
        assert_eq!(bars(&map)[5], (5, None, ColorRole::Text));
    }

    #[test]
    fn a_tab_counts_as_the_editors_columns_and_a_comment_keeps_its_colour() {
        let map = map("// a comment\n\tllSay(0, \"\");\n", Lang::Lsl, indent::TAB_SIZE);
        assert_eq!(map.lines[0].role, ColorRole::Comment);
        assert_eq!(map.lines[1].bar, Some((4.0, 17.0)), "a tab is four columns wide");
    }

    #[test]
    fn a_longer_line_runs_past_the_columns_the_map_spans() {
        let long = format!("{};", "x".repeat(120));
        let map = map(&long, Lang::Lsl, indent::TAB_SIZE);
        let (from, to) = map.lines[0].bar.expect("a bar");
        assert_eq!(from, 0.0);
        assert_eq!(to, 121.0, "the window cuts it at the map's edge");
        assert!(to > COLUMNS);
    }

    #[test]
    fn an_slua_comment_is_coloured_as_one() {
        let map = map("-- a comment\nlocal count = 0\n", Lang::Lua, indent::TAB_SIZE);
        assert_eq!(map.lines[0].role, ColorRole::Comment);
        assert_eq!(map.lines[1].role, ColorRole::Control, "local");
    }
}
