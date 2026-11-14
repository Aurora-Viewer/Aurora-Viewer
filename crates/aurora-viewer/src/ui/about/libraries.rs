//! The Libraries tab reads THIRD-PARTY-LICENSES.md, built into the viewer:
//! the Rust crates it is made of, their licences and the notices their
//! authors ask to keep (MIT, BSD, Apache, Unicode… want them with every copy
//! of the program). The file is written by `scripts/third-party-licenses.ps1`
//! from Cargo.lock. Close to a megabyte of text: it is shown as lines, laid
//! out only where the scroll shows them.

/// THIRD-PARTY-LICENSES.md at the repository root.
const TEXT: &str = include_str!("../../../../../THIRD-PARTY-LICENSES.md");

/// The file's lines, those longer than `width` characters folded at a space
/// so that every row has the same height and the list can skip the hidden
/// ones. Folded again only when the width changes (the window resized).
pub fn lines(width: usize) -> std::sync::Arc<Vec<&'static str>> {
    type Folded = Option<(usize, std::sync::Arc<Vec<&'static str>>)>;
    static LINES: std::sync::Mutex<Folded> = std::sync::Mutex::new(None);
    let width = width.max(20);
    let mut cached = LINES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match &*cached {
        Some((w, lines)) if *w == width => lines.clone(),
        _ => {
            let lines = std::sync::Arc::new(fold(TEXT, width));
            *cached = Some((width, lines.clone()));
            lines
        }
    }
}

/// Split `text` in lines of at most `width` characters, cutting a longer
/// line after its last space that fits (a word longer than `width` stays
/// whole).
pub fn fold(text: &str, width: usize) -> Vec<&str> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim_end();
        while rest.chars().count() > width {
            // the first `width` characters and the one after them: a space
            // there ends a line of exactly `width`
            let limit = rest.char_indices().nth(width).map_or(rest.len(), |(i, c)| i + c.len_utf8());
            let cut = rest[..limit]
                .rfind(' ')
                .filter(|&i| i > 0)
                .or_else(|| rest[limit..].find(' ').map(|i| limit + i));
            let Some(cut) = cut else { break };
            out.push(&rest[..cut]);
            rest = rest[cut..].trim_start();
        }
        out.push(rest);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_fold_at_a_space() {
        assert_eq!(fold("one two three\n\nfour", 7), vec!["one two", "three", "", "four"]);
        // a word longer than the width stays whole
        assert_eq!(fold("abcdefghij kl", 4), vec!["abcdefghij", "kl"]);
        assert_eq!(fold("é é é", 3), vec!["é é", "é"]);
    }

    /// Cargo.lock's packages, as "name version".
    fn locked() -> std::collections::HashSet<String> {
        let lock = include_str!("../../../../../Cargo.lock");
        let mut out = std::collections::HashSet::new();
        let mut name = None;
        for line in lock.lines() {
            if let Some(n) = line.strip_prefix("name = ") {
                name = Some(n.trim_matches('"'));
            } else if let (Some(v), Some(n)) = (line.strip_prefix("version = "), name.take()) {
                out.insert(format!("{n} {}", v.trim_matches('"')));
            }
        }
        out
    }

    #[test]
    fn every_crate_listed_is_in_cargo_lock() {
        let locked = locked();
        let listed: Vec<&str> = TEXT
            .split("## Crates")
            .nth(1)
            .and_then(|s| s.split("## Licence texts").next())
            .expect("THIRD-PARTY-LICENSES.md has its Crates table")
            .lines()
            .filter_map(|l| l.strip_prefix("| "))
            .filter_map(|l| l.split(" | ").next())
            .filter(|c| *c != "Crate" && !c.starts_with("---"))
            .collect();
        assert!(listed.len() > 100, "the Crates table was not read");
        let stale: Vec<&&str> = listed.iter().filter(|c| !locked.contains(**c)).collect();
        assert!(
            stale.is_empty(),
            "THIRD-PARTY-LICENSES.md names crates Cargo.lock no longer has: {stale:?}. \
             Run ./scripts/third-party-licenses.ps1 and commit the file."
        );
    }
}
