//! Finding a word in the scripts of one window, and where a word is declared:
//! the dock's Search tab, and Ctrl+click's go-to-definition across the scripts
//! of a linkset. Both scan the text as it is edited (not the processed view),
//! through the same matching and outline passes as the rest of the editor.

use super::Lang;
use super::outline::{self, Kind};

/// Where `word` is declared among the window's scripts (`(document, source,
/// lua)`): the document's place in the list, the line, and the column of its
/// name. The script the caret is in is searched first, then the others in the
/// window's order; only the outlines' states, events, functions and globals
/// count, so a keyword or a built-in name has no declaration.
pub(super) fn definition(scripts: &[(usize, &str, Lang)], word: &str, current: usize) -> Option<(usize, usize, usize)> {
    if word.is_empty() {
        return None;
    }
    let mut order: Vec<&(usize, &str, Lang)> = scripts.iter().collect();
    order.sort_by_key(|(document, ..)| *document != current);
    for (document, source, lang) in order {
        for item in outline::outline(source, *lang) {
            if item.name != word || !matches!(item.kind, Kind::State | Kind::Event | Kind::Function | Kind::Global) {
                continue;
            }
            let column = source
                .lines()
                .nth(item.line)
                .and_then(|line| line.find(word).map(|byte| line[..byte].chars().count()))
                .unwrap_or(0);
            return Some((*document, item.line, column));
        }
    }
    None
}
