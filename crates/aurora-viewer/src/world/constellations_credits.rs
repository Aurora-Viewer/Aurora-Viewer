//! The constellation data's credits: who each source is, what was taken
//! from it, under which licence, its site and when it was brought in, for
//! the window "Licences and attribution" (ui/constellations_window.rs).
//!
//! They come from the data, never from the code: a `credits.json` beside
//! the general data (the data's root), in a culture's folder
//! (`cultures/<id>/`) and in a set's (`sets/<id>/`), each taken with its
//! unit from the built-in data or the user's folder; the root's of both
//! are read, the built-in one first (constellations_builtin.rs):
//!
//! ```json
//! {"credits": [{"holder": "…", "work": "…", "licence": "…",
//!   "url": "https://…", "imported": "2026-11-24", "note": "…"}]}
//! ```
//!
//! Only `holder` is needed; a field missing (or unreadable) shows nothing.
//! The files are untrusted input like the rest of the folder: a size limit,
//! a bounded number of entries, texts cut and cleaned of control
//! characters, a date that must look like one; a bad file or entry is a
//! problem line (logged once), never a failure. A link is clickable only
//! when it is an `https://` address; anything else stays text. The scan of
//! the figures (constellations_catalogue.rs) never reads these files: in a
//! set's folder it reads a JSON file as a figure only when it has a `kind`
//! and the file's own id.
//!
//! Read off the frame's thread when the window opens, and again after a
//! Rescan or a new reading of the folder; nothing while it is closed.

use serde::Deserialize;
use std::sync::{Arc, Mutex, MutexGuard};

/// The file's name at each of its places.
pub const FILE: &str = "credits.json";
/// Its size limit (a set's own file's).
pub const MAX_FILE: u64 = super::catalogue::MAX_SET_FILE;
/// Entries read from one file; the rest are left out (said once).
pub const MAX_ENTRIES: usize = 32;
/// Characters kept of each text (a URL a little longer).
pub const MAX_TEXT: usize = 200;
pub const MAX_URL: usize = 300;
pub const MAX_NOTE: usize = 400;

/// One source's credit, its texts cleaned (empty: not given).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Credit {
    pub holder: String,
    pub work: String,
    pub licence: String,
    pub url: String,
    /// The date the data was brought in, `YYYY-MM-DD` as the file says it.
    pub imported: String,
    pub note: String,
}

impl Credit {
    /// The address to open when its link is clicked: an `https://` one
    /// only (any other text is shown, never opened).
    pub fn link(&self) -> Option<&str> {
        https_link(&self.url)
    }
}

/// `url` when it can be opened: `https://` and something after it, no
/// spaces.
pub fn https_link(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    (!rest.is_empty() && !url.chars().any(char::is_whitespace)).then_some(url)
}

/// Where a group of credits comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// The folder's root: the general data.
    General,
    /// A culture's folder, by its name in the interface's language.
    Culture(String),
    /// A set's folder, by its name in the interface's language.
    Set(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub source: Source,
    pub credits: Vec<Credit>,
}

#[derive(Deserialize)]
struct File {
    credits: Vec<serde_json::Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Entry {
    holder: Option<String>,
    work: Option<String>,
    licence: Option<String>,
    url: Option<String>,
    imported: Option<String>,
    note: Option<String>,
}

/// A text of the file as shown: no control characters, trimmed, at most
/// `max` characters (the names' rule, constellations_catalogue.rs).
fn clean(s: Option<&str>, max: usize) -> String {
    s.unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// An ISO date (`2026-11-24`), as stored in the file.
fn iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter()
            .enumerate()
            .all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
        && matches!(s.get(5..7).and_then(|m| m.parse::<u8>().ok()), Some(1..=12))
        && matches!(s.get(8..10).and_then(|d| d.parse::<u8>().ok()), Some(1..=31))
}

/// One file's credits, and what was left out of it (`problems`, without
/// the file's name). An error: nothing of the file could be read.
pub fn parse(bytes: &[u8], problems: &mut Vec<String>) -> Result<Vec<Credit>, String> {
    let file: File = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if file.credits.len() > MAX_ENTRIES {
        problems.push(format!("more than {MAX_ENTRIES} entries; the rest left out"));
    }
    let mut out = Vec::new();
    for (n, value) in file.credits.into_iter().take(MAX_ENTRIES).enumerate() {
        let Ok(e) = serde_json::from_value::<Entry>(value) else {
            problems.push(format!("entry {}: not readable", n + 1));
            continue;
        };
        let holder = clean(e.holder.as_deref(), MAX_TEXT);
        if holder.is_empty() {
            problems.push(format!("entry {}: no holder", n + 1));
            continue;
        }
        let mut imported = clean(e.imported.as_deref(), 16);
        if !imported.is_empty() && !iso_date(&imported) {
            problems.push(format!("entry {}: the import date is not a date (YYYY-MM-DD)", n + 1));
            imported.clear();
        }
        out.push(Credit {
            holder,
            work: clean(e.work.as_deref(), MAX_TEXT),
            licence: clean(e.licence.as_deref(), MAX_TEXT),
            url: clean(e.url.as_deref(), MAX_URL),
            imported,
            note: clean(e.note.as_deref(), MAX_NOTE),
        });
    }
    Ok(out)
}

/// One place's file (its bytes, `None`: not there); what does not read is
/// said in `problems` with the file's place (`label`).
fn read_one(bytes: Option<Result<Vec<u8>, String>>, label: &str, problems: &mut Vec<String>) -> Vec<Credit> {
    let Some(bytes) = bytes else { return Vec::new() };
    let mut found = Vec::new();
    let credits = bytes.and_then(|b| parse(&b, &mut found));
    problems.extend(found.into_iter().map(|p| format!("{label}: {p}")));
    credits.unwrap_or_else(|e| {
        problems.push(format!("{label}: {e}"));
        Vec::new()
    })
}

/// Every credit of the data, grouped: the general data's (the built-in
/// root's, then the user's), then each culture's and each set's in the
/// catalogue's order (`cultures`, `sets`: their ids, safe as folder names,
/// and their shown names). A place without a file has no group.
pub fn read(
    data: &super::builtin::Data,
    cultures: &[(String, String)],
    sets: &[(String, String)],
    problems: &mut Vec<String>,
) -> Vec<Group> {
    let mut out = Vec::new();
    let mut push = |source: Source, credits: Vec<Credit>| {
        if !credits.is_empty() {
            out.push(Group { source, credits });
        }
    };
    for root in data.root_credits(MAX_FILE) {
        push(Source::General, read_one(Some(root), FILE, problems));
    }
    for (id, name) in cultures {
        let label = format!("cultures/{id}/{FILE}");
        push(
            Source::Culture(name.clone()),
            read_one(data.read(&label, MAX_FILE), &label, problems),
        );
    }
    for (id, name) in sets {
        let label = format!("sets/{id}/{FILE}");
        push(Source::Set(name.clone()), read_one(data.read(&label, MAX_FILE), &label, problems));
    }
    out
}

/// What the window shows: whether the files are being read, and the groups
/// as last read (none before the first reading ends).
#[derive(Clone, Debug, Default)]
pub struct View {
    pub reading: bool,
    pub groups: Option<Arc<Vec<Group>>>,
}

struct State {
    reading: bool,
    /// The folder's reading (its generation) the groups were read for, and
    /// a Rescan asked since.
    read_for: Option<u64>,
    stale: bool,
    groups: Option<Arc<Vec<Group>>>,
}

static STATE: Mutex<State> = Mutex::new(State {
    reading: false,
    read_for: None,
    stale: false,
    groups: None,
});

fn lock() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Read the files again the next time the window asks (Rescan).
pub fn invalidate() {
    lock().stale = true;
}

/// The window's view; asks for a reading on a thread of its own when there
/// is none for the folder's reading `generation` (or a Rescan came since).
/// `read`: the reading itself, run on that thread.
pub fn view(generation: u64, read: impl FnOnce() -> Vec<Group> + Send + 'static) -> View {
    let mut st = lock();
    if !st.reading && (st.stale || st.read_for != Some(generation)) {
        st.reading = true;
        st.stale = false;
        let spawned = std::thread::Builder::new().name("constellation-credits".into()).spawn(move || {
            let groups = read();
            let mut st = lock();
            st.reading = false;
            st.read_for = Some(generation);
            st.groups = Some(Arc::new(groups));
        });
        if let Err(e) = spawned {
            log::error!("constellations: no thread to read the credits: {e}");
            st.reading = false;
            st.read_for = Some(generation);
            st.groups = Some(Arc::new(Vec::new()));
        }
    }
    View {
        reading: st.reading,
        groups: st.groups.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::constellations::catalogue::tests::{drawn, folder, set, write};

    const GOOD: &str = r#"{"credits":[
        {"holder":"Example Library","work":"Star chart scans","licence":"Example Licence 1.0",
         "url":"https://example.org/charts","imported":"2026-11-24","note":"Courtesy of Example Library","extra":1},
        {"holder":"Sample Observatory"}]}"#;

    #[test]
    fn a_good_file_and_missing_fields() {
        let mut problems = Vec::new();
        let c = parse(GOOD.as_bytes(), &mut problems).expect("readable");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.len(), 2);
        assert_eq!(
            c[0],
            Credit {
                holder: "Example Library".into(),
                work: "Star chart scans".into(),
                licence: "Example Licence 1.0".into(),
                url: "https://example.org/charts".into(),
                imported: "2026-11-24".into(),
                note: "Courtesy of Example Library".into(),
            }
        );
        assert_eq!(c[0].link(), Some("https://example.org/charts"));
        // only the holder: every other line empty, no link
        assert_eq!(
            c[1],
            Credit {
                holder: "Sample Observatory".into(),
                ..Credit::default()
            }
        );
        assert_eq!(c[1].link(), None);
    }

    #[test]
    fn bad_files_and_entries_are_problems_never_a_failure() {
        let mut problems = Vec::new();
        assert!(parse(b"{not json", &mut problems).is_err());
        assert!(parse(br#"{"credits": 3}"#, &mut problems).is_err());
        assert!(parse(br#"[]"#, &mut problems).is_err());
        // an entry without a holder, a bad one, a bad date: left out or emptied
        let c = parse(
            br#"{"credits":[{"work":"x"},{"holder":5},"text",{"holder":"Ok","imported":"yesterday"},{"holder":"  "}]}"#,
            &mut problems,
        )
        .expect("the file reads");
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].holder.as_str(), c[0].imported.as_str()), ("Ok", ""));
        assert_eq!(problems.len(), 5, "{problems:?}");
        for bad in ["2026-13-01", "2026-1-01", "26-11-24", "2026-11-32", "2026/11/24"] {
            assert!(!iso_date(bad), "{bad}");
        }
        assert!(iso_date("2026-02-01"));
    }

    #[test]
    fn long_texts_are_cut_and_cleaned_and_entries_capped() {
        let long = "a".repeat(5000);
        let json = format!(
            r#"{{"credits":[{{"holder":"Ex\u0007ample\nLibrary","work":"{long}","url":"https://example.org/{long}","note":"{long}"}}]}}"#
        );
        let mut problems = Vec::new();
        let c = parse(json.as_bytes(), &mut problems).expect("readable");
        assert_eq!(c[0].holder, "ExampleLibrary", "control characters left out");
        assert_eq!(c[0].work.chars().count(), MAX_TEXT);
        assert_eq!(c[0].url.chars().count(), MAX_URL);
        assert_eq!(c[0].note.chars().count(), MAX_NOTE);
        let many = format!(r#"{{"credits":[{}]}}"#, vec![r#"{"holder":"Example"}"#; MAX_ENTRIES + 5].join(","));
        let mut problems = Vec::new();
        assert_eq!(parse(many.as_bytes(), &mut problems).expect("readable").len(), MAX_ENTRIES);
        assert_eq!(problems.len(), 1);
        // a file over the limit is not read
        let root = folder("credits-big");
        write(&root, FILE, &vec![b' '; MAX_FILE as usize + 10]);
        let mut problems = Vec::new();
        assert!(read(&super::super::builtin::Data::folder(&root), &[], &[], &mut problems).is_empty());
        assert_eq!(problems.len(), 1, "{problems:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_an_https_link_opens() {
        assert_eq!(https_link("https://example.org/a"), Some("https://example.org/a"));
        for not in [
            "http://example.org",
            "HTTPS://example.org",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://",
            "https://example.org/a b",
            "example.org",
            "",
        ] {
            assert_eq!(https_link(not), None, "{not}");
        }
    }

    #[test]
    fn the_three_places_in_the_catalogues_order_and_the_scan_ignores_them() {
        let root = folder("credits-places");
        write(
            &root,
            "lines.json",
            br#"{"Ori":{"name":"Orion","label":[88.0,22.8],"lines":[{"a":[78.6,-8.2],"b":[81.1,-2.4]}]}}"#,
        );
        set(&root, "b-set", 2);
        set(&root, "a-set", 1);
        write(&root, "sets/a-set/Ori.json", drawn("Ori", "").as_bytes());
        write(&root, "sets/a-set/Ori.webp", b"RIFF");
        write(&root, "cultures/other/culture.json", br#"{"name":"Other"}"#);
        write(&root, FILE, br#"{"credits":[{"holder":"Example Library"}]}"#);
        write(
            &root,
            "cultures/other/credits.json",
            br#"{"credits":[{"holder":"Sample Society"}]}"#,
        );
        write(&root, "sets/a-set/credits.json", br#"{"credits":[{"holder":"Example Atlas"}]}"#);
        write(&root, "sets/b-set/credits.json", br#"{"credits":[{"holder":"Sample Museum"}]}"#);
        // the scan of the figures: no problem, no figure named credits, the
        // same catalogue as without the files
        let mut problems = Vec::new();
        let with = crate::world::constellations::catalogue::scan(&root, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(with.figures.iter().all(|f| !f.id.eq_ignore_ascii_case("credits")));
        assert_eq!(with.figures.len(), 1);
        let sets: Vec<(String, String)> = with.sets.iter().map(|s| (s.id.clone(), s.display_name("en").to_owned())).collect();
        let cultures: Vec<(String, String)> = with.cultures.iter().map(|c| (c.id.clone(), c.id.clone())).collect();
        let groups = read(&super::super::builtin::Data::folder(&root), &cultures, &sets, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        let holders: Vec<(&Source, &str)> = groups.iter().map(|g| (&g.source, g.credits[0].holder.as_str())).collect();
        assert_eq!(
            holders,
            [
                (&Source::General, "Example Library"),
                (&Source::Culture("other".into()), "Sample Society"),
                (&Source::Set("Set a-set".into()), "Example Atlas"),
                (&Source::Set("Set b-set".into()), "Sample Museum"),
            ]
        );
        for file in [
            FILE.to_owned(),
            "cultures/other/credits.json".into(),
            "sets/a-set/credits.json".into(),
        ] {
            let _ = std::fs::remove_file(root.join(file));
        }
        let _ = std::fs::remove_file(root.join("sets/b-set/credits.json"));
        let mut problems = Vec::new();
        let without = crate::world::constellations::catalogue::scan(&root, &mut problems);
        assert_eq!(
            (without.figures.len(), without.sets.len(), without.cultures.len()),
            (with.figures.len(), with.sets.len(), with.cultures.len())
        );
        // none: no group
        assert!(read(&super::super::builtin::Data::folder(&root), &cultures, &sets, &mut problems).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
