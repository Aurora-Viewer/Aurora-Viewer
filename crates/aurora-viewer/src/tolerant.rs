//! Forward-compatible reading of the files the viewer saves (settings,
//! account catalogue, HUD sizes, skin): a value an older build does not
//! understand — a shortcut added later, a new enum variant, a field whose type
//! changed — costs that value only, never the rest of the file.
//!
//! The strict read is tried first; on failure `serde_path_to_error` names the
//! value that failed, which is removed (an object entry, a list element) so
//! that serde fills it with its default, and the read is tried again. A value
//! the containing type cannot do without (a required field, a fixed-size
//! tuple) makes the failure climb to its container, so a list keeps its valid
//! elements and a struct its valid fields.

use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Upper bound on the values dropped from one file: past it the file is not
/// a newer version of ours but something else, and is refused as a whole.
const MAX_DROPPED: usize = 4096;

/// A file read with what had to be left out of it.
#[derive(Debug)]
pub struct Read<T> {
    pub value: T,
    /// Paths (`keybinds.map[12]`, `colors.chat_slurl`) of the values that
    /// were not understood and fell back to their default.
    pub dropped: Vec<String>,
}

impl<T> Read<T> {
    pub fn repaired(&self) -> bool {
        !self.dropped.is_empty()
    }
}

/// Reads `bytes` as `T`, keeping everything that is understood. `None` when
/// the file is not JSON or its top level itself cannot be read; `file` names
/// it in the log, one line per value left out (the path, never the value).
pub fn from_slice<T: DeserializeOwned>(bytes: &[u8], file: &str) -> Option<Read<T>> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(value) => from_value(value, file),
        Err(error) => {
            log::warn!("{file}: not readable JSON ({}:{})", error.line(), error.column());
            None
        }
    }
}

/// [`from_slice`] for a document already parsed.
pub fn from_value<T: DeserializeOwned>(mut value: Value, file: &str) -> Option<Read<T>> {
    let mut dropped = Vec::new();
    loop {
        let error = match serde_path_to_error::deserialize::<_, T>(value.clone()) {
            Ok(read) => return Some(Read { value: read, dropped }),
            Err(error) => error,
        };
        let path = error.path().to_string();
        if dropped.len() >= MAX_DROPPED || !remove(&mut value, error.path()) {
            log::warn!("{file}: unreadable at `{path}`, file ignored");
            return None;
        }
        // a tuple fails element by element at the same path: one line
        if dropped.last() != Some(&path) {
            log::warn!("{file}: value at `{path}` not understood, default used");
        }
        dropped.push(path);
    }
}

/// Removes the deepest value of `path` present in `root`; false when that
/// would be the root itself.
fn remove(root: &mut Value, path: &serde_path_to_error::Path) -> bool {
    use serde_path_to_error::Segment;
    // Only object keys and list indices address the document: an enum or an
    // unknown segment stops the walk, and the value holding it is dropped.
    let mut steps = Vec::new();
    let mut at: &Value = root;
    for segment in path.iter() {
        let next = match (segment, at) {
            (Segment::Map { key }, Value::Object(map)) => map.get(key).map(|v| (Step::Key(key.clone()), v)),
            (Segment::Seq { index }, Value::Array(list)) => list.get(*index).map(|v| (Step::Index(*index), v)),
            _ => None,
        };
        let Some((step, value)) = next else { break };
        steps.push(step);
        at = value;
    }
    let Some(last) = steps.pop() else { return false };
    let mut parent = root;
    for step in &steps {
        parent = match (step, parent) {
            (Step::Key(key), Value::Object(map)) => &mut map[key],
            (Step::Index(index), Value::Array(list)) => &mut list[*index],
            _ => return false,
        };
    }
    match (last, parent) {
        (Step::Key(key), Value::Object(map)) => map.remove(&key).is_some(),
        (Step::Index(index), Value::Array(list)) => {
            list.remove(index);
            true
        }
        _ => false,
    }
}

enum Step {
    Key(String),
    Index(usize),
}

/// Keeps a copy of a file before it is rewritten without what this build
/// could not read (`settings.json` → `settings.newer-backup.json`): a newer
/// build's values stay on disk. Replaced at each repair, so it holds the
/// latest file that needed one.
pub fn backup(path: &Path) {
    let target = backup_path(path);
    match std::fs::read(path).and_then(|bytes| crate::atomic_file::write(&target, &bytes)) {
        Ok(()) => log::info!("kept a copy of {} as {}", file_name(path), file_name(&target)),
        Err(error) => log::warn!("cannot keep a copy of {}: {error}", file_name(path)),
    }
}

pub fn backup_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("json");
    path.with_file_name(format!("{stem}.newer-backup.{extension}"))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The document to write over a file from a newer build: `ours` (what this
/// build saves) laid over `theirs` (the file as read), so that keys this
/// build does not know survive, objects merging key by key. A top-level key
/// in `kept` keeps their value as long as ours still equals what was loaded
/// in its place (`loaded`), i.e. the user did not change it.
pub fn merge_over_newer(theirs: &Value, ours: Value, loaded: &Value, kept: &[String]) -> Value {
    let Value::Object(theirs) = theirs else { return ours };
    let Value::Object(ours) = ours else { return ours };
    let mut out = theirs.clone();
    for (key, value) in ours {
        let untouched = kept.contains(&key) && loaded.get(&key) == Some(&value);
        if untouched && out.contains_key(&key) {
            continue;
        }
        let merged = match out.remove(&key) {
            Some(previous @ Value::Object(_)) if value.is_object() && !kept.contains(&key) => {
                merge_over_newer(&previous, value, &Value::Null, &[])
            }
            _ => value,
        };
        out.insert(key, merged);
    }
    Value::Object(out)
}

/// The top-level key of a dropped path (`keybinds.map[3]` → `keybinds`).
pub fn top_key(path: &str) -> &str {
    path.split(['.', '[']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Default, Deserialize, PartialEq, Clone, Copy)]
    enum Kind {
        #[default]
        A,
        B,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Pair(Kind, u8);

    #[derive(Debug, Default, Deserialize, PartialEq)]
    #[serde(default)]
    struct Inner {
        kind: Kind,
        n: u32,
    }

    #[derive(Debug, Default, Deserialize, PartialEq)]
    #[serde(default)]
    struct File {
        kind: Kind,
        name: String,
        inner: Inner,
        list: Vec<Pair>,
        version: u32,
    }

    #[test]
    fn a_value_from_the_future_costs_only_that_value() {
        let file = json!({
            "kind": "C",
            "name": "kept",
            "inner": { "kind": "Z", "n": 7 },
            "list": [["A", 1], ["Future", 2], ["B", 3], ["B", "x"]],
            "unknown_field": { "anything": true },
            "version": 99,
        });
        let read = from_value::<File>(file, "test.json").expect("readable");
        assert_eq!(read.value.kind, Kind::A);
        assert_eq!(read.value.name, "kept");
        assert_eq!(read.value.inner, Inner { kind: Kind::A, n: 7 });
        assert_eq!(read.value.list, vec![Pair(Kind::A, 1), Pair(Kind::B, 3)]);
        assert_eq!(read.value.version, 99);
        // a tuple element fails value by value, then as a whole
        assert!(read.dropped.iter().any(|p| p == "list[1]"), "{:?}", read.dropped);
        assert!(read.dropped.iter().any(|p| p == "kind"));
        assert!(read.dropped.iter().any(|p| p == "inner.kind"));
    }

    #[test]
    fn a_readable_file_is_read_as_is() {
        let read = from_value::<File>(json!({ "kind": "B", "list": [["B", 2]] }), "test.json").unwrap();
        assert!(!read.repaired());
        assert_eq!(read.value.list, vec![Pair(Kind::B, 2)]);
    }

    #[test]
    fn a_top_level_that_cannot_be_read_refuses_the_file() {
        assert!(from_value::<File>(json!("text"), "test.json").is_none());
        assert!(from_slice::<File>(b"{ not json", "test.json").is_none());
        // a list whose elements all fail keeps none of them, not the defaults
        let read = from_value::<Vec<Pair>>(json!([["X", 1], 5]), "test.json").unwrap();
        assert!(read.value.is_empty());
        assert_eq!(read.dropped.iter().filter(|p| *p == "[0]").count(), 2, "{:?}", read.dropped);
    }

    #[test]
    fn newer_keys_and_untouched_values_survive_a_rewrite() {
        let theirs = json!({
            "kind": "C",
            "name": "old",
            "inner": { "n": 1, "future": "yes" },
            "future_top": [1, 2],
            "version": 99,
        });
        // what this build loaded in place of the value it could not read
        let loaded = json!({ "kind": "A", "name": "old", "inner": { "n": 1 }, "version": 1 });
        let ours = json!({ "kind": "A", "name": "renamed", "inner": { "n": 2 }, "version": 1 });
        let kept = vec!["kind".to_owned(), "version".to_owned()];
        let merged = merge_over_newer(&theirs, ours, &loaded, &kept);
        assert_eq!(merged["kind"], "C", "untouched: the newer value stays");
        assert_eq!(merged["name"], "renamed", "changed here: ours is written");
        assert_eq!(merged["inner"], json!({ "n": 2, "future": "yes" }));
        assert_eq!(merged["future_top"], json!([1, 2]));
        assert_eq!(merged["version"], 99);
        // a kept value the user changed is written
        let ours = json!({ "kind": "B", "version": 1 });
        let merged = merge_over_newer(&theirs, ours, &loaded, &kept);
        assert_eq!(merged["kind"], "B");
    }

    #[test]
    fn backup_names_and_top_keys() {
        assert_eq!(backup_path(Path::new("a/settings.json")), Path::new("a/settings.newer-backup.json"));
        assert_eq!(top_key("keybinds.map[3][0]"), "keybinds");
        assert_eq!(top_key("grid"), "grid");
        assert_eq!(top_key("[2].grid"), "");
    }
}
