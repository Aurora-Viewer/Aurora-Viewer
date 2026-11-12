//! The list of the account's past conversations for the Conversation Log
//! window (LLConversationLog / LLFloaterConversationLog,
//! indra/newview/llconversationlog.cpp and llfloaterconversationlog.cpp,
//! originally LGPL 2.1): who, of which kind, last when, and the transcript
//! to open.
//!
//! The Legacy Viewer keeps its own `conversation.log` beside the
//! transcripts. Aurora reads the list from the transcripts themselves (the
//! files of `names.rs`, their dated parts gathered): the same conversations,
//! with no second file that could drift from them. The folder is read on a
//! thread of its own (`list`).

use super::names::{EXTENSION, GROUP_SUFFIX, NEARBY};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What a conversation was with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Nearby,
    Group,
    /// A resident, or a conference.
    Other,
}

/// One past conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    /// Its name as the file has it: a group's without " (group)".
    pub name: String,
    pub kind: Kind,
    /// When it was last written to.
    pub last: SystemTime,
    /// Its most recent file (the dated parts of a conversation are one
    /// conversation).
    pub file: PathBuf,
}

/// Reads the folder on a thread; the list comes back on the receiver, newest
/// first (nothing when the folder cannot be read).
pub fn list(dir: PathBuf) -> crossbeam_channel::Receiver<Vec<Conversation>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new().name("conversation-log".into()).spawn(move || {
        let files = match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .flatten()
                .filter_map(|e| {
                    let meta = e.metadata().ok().filter(std::fs::Metadata::is_file)?;
                    Some((e.file_name().into_string().ok()?, meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
                })
                .collect(),
            Err(e) => {
                log::warn!("conversation log: the transcripts' folder cannot be read: {e}");
                Vec::new()
            }
        };
        let _ = tx.send(gather(&dir, files));
    });
    if let Err(e) = spawned {
        log::warn!("conversation log: no thread to read the folder: {e}");
    }
    rx
}

/// The conversations of a folder's files (name, last modified), newest
/// first: the transcripts only, a conversation's dated parts as one.
pub fn gather(dir: &Path, files: Vec<(String, SystemTime)>) -> Vec<Conversation> {
    let mut by_stem: std::collections::HashMap<String, (SystemTime, String)> = std::collections::HashMap::new();
    for (file, modified) in files {
        let Some(stem) = file.strip_suffix(&format!(".{EXTENSION}")) else {
            continue;
        };
        let stem = undated(stem);
        if stem.is_empty() {
            continue;
        }
        let entry = by_stem.entry(stem.to_owned()).or_insert((modified, file.clone()));
        if modified > entry.0 || (modified == entry.0 && file > entry.1) {
            *entry = (modified, file);
        }
    }
    let mut out: Vec<Conversation> = by_stem
        .into_iter()
        .map(|(stem, (last, file))| {
            let (name, kind) = if stem == NEARBY {
                (stem, Kind::Nearby)
            } else if let Some(group) = stem.strip_suffix(GROUP_SUFFIX) {
                (group.to_owned(), Kind::Group)
            } else {
                (stem, Kind::Other)
            };
            Conversation {
                name,
                kind,
                last,
                file: dir.join(file),
            }
        })
        .collect();
    out.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.name.cmp(&b.name)));
    out
}

/// A file's stem without its date ("-2026-10" for a conversation,
/// "-2026-10-12" for the nearby chat, LogFileNamewithDate).
fn undated(stem: &str) -> &str {
    let dated = |suffix: &str| {
        suffix
            .bytes()
            .enumerate()
            .all(|(i, b)| if matches!(i, 0 | 5 | 8) { b == b'-' } else { b.is_ascii_digit() })
    };
    for len in ["-0000-00-00".len(), "-0000-00".len()] {
        if stem.len() > len
            && let Some(cut) = stem.len().checked_sub(len)
            && stem.is_char_boundary(cut)
            && dated(&stem[cut..])
        {
            return &stem[..cut];
        }
    }
    stem
}

/// The conversations whose name holds `filter`, case ignored.
pub fn filtered<'a>(list: &'a [Conversation], filter: &str) -> Vec<&'a Conversation> {
    let needle = filter.trim().to_lowercase();
    list.iter()
        .filter(|c| needle.is_empty() || c.name.to_lowercase().contains(&needle))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn dated_parts_are_one_conversation_newest_first() {
        let dir = Path::new("logs");
        let list = gather(
            dir,
            vec![
                ("Jane Doe-2026-09.txt".into(), at(100)),
                ("Jane Doe-2026-10.txt".into(), at(300)),
                ("Builders (group).txt".into(), at(200)),
                ("chat-2026-10-12.txt".into(), at(50)),
                ("chat.txt".into(), at(40)),
                ("settings.xml".into(), at(900)),
            ],
        );
        let names: Vec<_> = list.iter().map(|c| (c.name.as_str(), c.kind)).collect();
        assert_eq!(
            names,
            [("Jane Doe", Kind::Other), ("Builders", Kind::Group), ("chat", Kind::Nearby)]
        );
        assert_eq!(list[0].file, dir.join("Jane Doe-2026-10.txt"), "its latest part");
        assert_eq!(list[0].last, at(300));
        assert_eq!(list[2].file, dir.join("chat-2026-10-12.txt"));
    }

    #[test]
    fn a_name_that_ends_like_a_date_keeps_it() {
        assert_eq!(undated("Jane-2026-10"), "Jane");
        assert_eq!(undated("chat-2026-10-12"), "chat");
        assert_eq!(undated("Jane-20x6-10"), "Jane-20x6-10");
        assert_eq!(undated("2026-10"), "2026-10", "nothing before the date");
        assert_eq!(undated("Zoé"), "Zoé");
    }

    #[test]
    fn the_filter_ignores_case() {
        let list = gather(Path::new("."), vec![("Jane Doe.txt".into(), at(1)), ("Tom.txt".into(), at(2))]);
        assert_eq!(filtered(&list, "jane").len(), 1);
        assert_eq!(filtered(&list, " ").len(), 2);
    }
}
