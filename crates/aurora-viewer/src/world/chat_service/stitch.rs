//! The stitched read of a direct conversation: the resident's archive, its
//! plaintext transcripts older than the archive and the head page not
//! written yet, in one oldest-first history. Port of readStitched,
//! serviceMessage and legacyWallEpoch (indra/newview/llchatservicehistory.cpp,
//! LL viewer 26.5, LGPL 2.1). It runs on the transcript thread, under the
//! storage lock.

use super::ViewRead;
use super::storage::Storage;
use crate::world::chat_log::format::{self, Entry, SYSTEM_FROM};
use crate::world::chat_log::names::EXTENSION;
use crate::world::chat_log::service::csv::ArchiveState;
use crate::world::chat_log::service::merge::{Messages, merge_direct_history};
use crate::world::chat_log::service::message::{HistoryMessage, legacy_wall_may_precede_service};
use aurora_net::chat_service::{HistoryRow, UUID_EPOCH, created_at_seconds};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

/// Which local plaintext transcript family backs a direct conversation's
/// stitched read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptFamily {
    pub dir: PathBuf,
    pub peer: Option<Uuid>,
    pub stem: String,
    pub shared: bool,
}

/// The rows an IM session shows (LLIMModel's 200).
pub const SESSION_LIMIT: usize = 200;
/// The rows the History window reads (LLFloaterConversationPreview, 10000).
pub const PREVIEW_LIMIT: usize = 10_000;

/// A finished read (HistoryResult).
#[derive(Debug, Clone, Default)]
pub struct ReadResult {
    pub epoch: u32,
    pub archive_serial: u32,
    pub included_service: bool,
    /// The archive could not be read as valid: the manager rescans it.
    pub maintenance_needed: bool,
    /// Oldest first.
    pub messages: Messages,
}

/// An SLT wall clock of a transcript ("2026/10/12 14:05" or with AM/PM),
/// read as if it were UTC, in seconds; `None` for anything else. Port of
/// legacyWallEpoch: the whole text is consumed, 15 characters at least.
pub fn legacy_wall_epoch(text: &str) -> Option<f64> {
    if text.len() < 15 {
        return None;
    }
    let mut s = Scan(text.as_bytes(), 0);
    let year = s.int()?;
    s.lit(b'/')?;
    let month = s.int()?;
    s.lit(b'/')?;
    let day = s.int()?;
    s.space();
    let mut hour = s.int()?;
    s.lit(b':')?;
    let minute = s.int()?;
    let end = s.1;
    s.space();
    if s.1 < s.0.len() {
        // "%2s": two characters at most, then nothing
        let suffix = &text[s.1..];
        if s.1 == end || suffix.len() > 2 || !(1..=12).contains(&hour) {
            return None;
        }
        match suffix {
            "AM" => hour %= 12,
            "PM" if hour != 12 => hour += 12,
            "PM" => {}
            _ => return None,
        }
    } else if !(0..=23).contains(&hour) || end != text.len() {
        return None;
    }
    if !(0..=59).contains(&minute) {
        return None;
    }
    let date = chrono::NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, u32::try_from(month).ok()?, u32::try_from(day).ok()?)?;
    let time = date.and_hms_opt(u32::try_from(hour).ok()?, u32::try_from(minute).ok()?, 0)?;
    Some(time.and_utc().timestamp() as f64)
}

/// A tiny `sscanf` for `%d` and literals.
struct Scan<'a>(&'a [u8], usize);

impl Scan<'_> {
    fn space(&mut self) {
        while self.0.get(self.1).is_some_and(u8::is_ascii_whitespace) {
            self.1 += 1;
        }
    }

    fn int(&mut self) -> Option<i64> {
        self.space();
        let start = self.1;
        if matches!(self.0.get(self.1), Some(b'+' | b'-')) {
            self.1 += 1;
        }
        let digits = self.1;
        while self.0.get(self.1).is_some_and(u8::is_ascii_digit) {
            self.1 += 1;
        }
        if self.1 == digits {
            return None;
        }
        std::str::from_utf8(&self.0[start..self.1]).ok()?.parse().ok()
    }

    fn lit(&mut self, b: u8) -> Option<()> {
        (self.0.get(self.1) == Some(&b)).then(|| self.1 += 1)
    }
}

/// A service row as a history message (serviceMessage): shown at its SLT
/// time with the date, its id kept for the dedup.
pub fn service_message(row: &HistoryRow) -> HistoryMessage {
    // static_cast<U32> of the seconds: whole seconds, never negative
    let secs = created_at_seconds(&row.created_at).unwrap_or(0.0).max(0.0) as u32;
    let when = UNIX_EPOCH + Duration::from_secs(u64::from(secs));
    HistoryMessage {
        from: row.from_name.clone(),
        from_id: Some(row.from_id),
        message: row.message.clone(),
        time: format::stamp(when, true),
        datetime: row.created_at.clone(),
        timestamp: Some(secs as i32),
        is_history: true,
        msg_id: Some(row.msg_id.clone()),
        ..HistoryMessage::default()
    }
}

/// A transcript line as a history message (LLLogChat::loadChatHistory):
/// "You" is the agent, a system line nobody, any other sender has no id.
pub fn legacy_message(entry: &Entry, agent: Uuid) -> HistoryMessage {
    let from_id = if entry.from == "You" {
        Some(agent)
    } else if entry.from == SYSTEM_FROM || entry.from.is_empty() {
        Some(Uuid::nil())
    } else {
        None
    };
    HistoryMessage {
        from: entry.from.clone(),
        from_id,
        message: entry.text.clone(),
        time: entry.stamp.clone(),
        datetime: entry.stamp.clone(),
        is_history: true,
        ..HistoryMessage::default()
    }
}

/// A conversation's plaintext files in reading order
/// (LLLogChat::getTranscriptFamily): `<stem>.txt` when it exists, then
/// every monthly `<stem>-YYYY-MM.txt`, sorted. `stem` is cleaned already.
pub fn transcript_family(dir: &Path, stem: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let undated = dir.join(format!("{stem}.{EXTENSION}"));
    if undated.is_file() {
        out.push(undated);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut shards: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|f| {
            f.strip_suffix(&format!(".{EXTENSION}"))
                .and_then(|s| s.strip_prefix(stem))
                .is_some_and(|suffix| {
                    suffix.len() == "-0000-00".len()
                        && suffix
                            .bytes()
                            .enumerate()
                            .all(|(i, b)| if matches!(i, 0 | 5) { b == b'-' } else { b.is_ascii_digit() })
                })
        })
        .collect();
    shards.sort();
    out.extend(shards.into_iter().map(|f| dir.join(f)));
    out
}

/// The plaintext admitted beside the archive: a line without a wall time
/// stays, a dated one only when it may precede the archive's oldest row
/// (or when the archive has none), with its wall time for the sort.
pub fn admit_legacy(legacy: Vec<HistoryMessage>, archive_oldest_epoch: Option<f64>) -> Messages {
    let mut out = Vec::with_capacity(legacy.len());
    for mut message in legacy {
        match legacy_wall_epoch(&message.datetime) {
            None => out.push(message),
            Some(wall) if archive_oldest_epoch.is_none_or(|oldest| legacy_wall_may_precede_service(wall, oldest)) => {
                message.legacy_wall_time = Some(wall);
                out.push(message);
            }
            Some(_) => {}
        }
    }
    out
}

/// Merge the admitted plaintext with the archive's display rows and the
/// preview, and keep the newest `limit` (0: all).
pub fn stitch(legacy: Messages, archive_rows: &[HistoryRow], preview: &[HistoryRow], limit: usize) -> Messages {
    let service: Messages = archive_rows.iter().chain(preview).map(service_message).collect();
    let mut messages = merge_direct_history(&legacy, &service);
    if limit > 0 && messages.len() > limit {
        messages.drain(..messages.len() - limit);
    }
    messages
}

/// The whole read of one resident (readStitched). `family`: the transcript
/// folder and stem inputs, `None` when the account keeps none or the folder
/// or name is not known yet. Shared Legacy Viewer folders resolve all Legacy,
/// username, dated, and `conversation.log`-mapped variants and deduplicate
/// overlapping lines across split files.
pub fn read_stitched(
    storage: &Storage,
    agent: Uuid,
    resident: Uuid,
    family: Option<TranscriptFamily>,
    read: &ViewRead,
    limit: usize,
) -> ReadResult {
    let mut result = ReadResult {
        epoch: read.epoch,
        archive_serial: read.archive_serial,
        included_service: read.include_service,
        ..ReadResult::default()
    };
    let mut archive_rows = Vec::new();
    let mut oldest_epoch = None;
    if read.include_service {
        let scan = storage.read_archive(resident, read.deleted_before_ticks, limit);
        result.maintenance_needed = !matches!(scan.state, ArchiveState::Valid | ArchiveState::Absent);
        if scan.state == ArchiveState::Valid {
            oldest_epoch = scan.oldest.map(|k| k.ticks.saturating_sub(UUID_EPOCH) as f64 / 10_000_000.0);
            archive_rows = scan.display_rows;
        }
    }
    let legacy = match family {
        None => Vec::new(),
        // every spelling, whole; a line without a date still goes after the
        // dated ones (mergeDirectHistory), not by its file's modification
        // time as in the other conversations' reads (human decision,
        // 2026-11-18)
        Some(family) if family.shared => {
            let paths = crate::world::chat_log::names::all_candidate_files(&family.dir, family.peer, &family.stem);
            crate::world::chat_log::store::read_combined_files(&paths)
                .iter()
                .map(|e| legacy_message(e, agent))
                .collect()
        }
        // Aurora's own folder holds Aurora's spelling only: the LL viewer's
        // family, not every candidate (human decision, 2026-11-18)
        Some(family) => {
            let mut out = Vec::new();
            for path in transcript_family(&family.dir, &family.stem) {
                if let Ok(bytes) = std::fs::read(&path) {
                    crate::disk_io::read(crate::disk_io::Origin::Logs, bytes.len());
                    out.extend(
                        crate::world::chat_log::store::parse(&bytes, false)
                            .iter()
                            .map(|e| legacy_message(e, agent)),
                    );
                }
            }
            out
        }
    };
    let legacy = admit_legacy(legacy, oldest_epoch);
    // the preview is newest first, as the snapshot holds it
    let preview: Vec<HistoryRow> = if read.include_service {
        read.preview.iter().rev().cloned().collect()
    } else {
        Vec::new()
    };
    result.messages = stitch(legacy, &archive_rows, &preview, limit);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::chat_service::{TimeUuidKey, format_time_uuid};

    fn row(n: u64, text: &str) -> HistoryRow {
        let key = TimeUuidKey {
            ticks: UUID_EPOCH + n * 10_000_000,
            tail: [i8::MIN, 0, 0, 0, 0, 0, 0, 0],
        };
        HistoryRow {
            conversation_id: String::new(),
            msg_id: format_time_uuid(&key),
            key,
            from_id: Uuid::from_u128(7),
            from_name: "Some One".into(),
            message: text.into(),
            dialog: 0,
            created_at: "2026-08-12T12:34:56Z".into(),
        }
    }

    fn entry(stamp: &str, from: &str, text: &str) -> Entry {
        Entry {
            stamp: stamp.into(),
            time: format::parse_stamp(stamp),
            from: from.into(),
            text: text.into(),
        }
    }

    #[test]
    fn wall_clocks_read_like_the_ll_viewer() {
        assert_eq!(legacy_wall_epoch("1970/01/01 00:01"), Some(60.0));
        assert_eq!(legacy_wall_epoch("1970/01/01 12:01 AM"), Some(60.0));
        assert_eq!(legacy_wall_epoch("1970/01/01 12:00 PM"), Some(43_200.0));
        assert_eq!(legacy_wall_epoch("1970/01/01 01:00 PM"), Some(46_800.0));
        for bad in [
            "14:05",
            "1970/01/01 24:00",
            "1970/01/01 13:00 PM",
            "1970/01/01 10:60",
            "1970/02/30 10:00",
            "1970/01/01 10:00 XM",
            "1970/01/01 10:00 extra",
        ] {
            assert_eq!(legacy_wall_epoch(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_plaintext_older_than_the_archive_is_admitted() {
        let agent = Uuid::from_u128(1);
        // the archive's oldest row: 2026/08/12 12:00 UTC
        let oldest = chrono::NaiveDate::from_ymd_opt(2026, 8, 12)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp() as f64;
        let lines = [
            entry("", "Some One", "undated"),
            entry("2026/08/12 04:59", "Some One", "before"),
            entry("2026/08/12 05:00", "Some One", "too close"),
            entry("2026/08/12 09:00", "You", "after"),
        ];
        let legacy: Vec<_> = lines.iter().map(|e| legacy_message(e, agent)).collect();
        assert_eq!(legacy[3].from_id, Some(agent));
        let admitted = admit_legacy(legacy.clone(), Some(oldest));
        let texts: Vec<&str> = admitted.iter().map(|m| m.message.as_str()).collect();
        assert_eq!(texts, ["undated", "before"]);
        assert!(admitted[1].legacy_wall_time.is_some());
        // no archive: everything stays
        assert_eq!(admit_legacy(legacy, None).len(), 4);
    }

    #[test]
    fn the_seam_keeps_the_newest_rows_and_each_service_row_once() {
        let legacy = admit_legacy(
            vec![legacy_message(&entry("2020/01/01 10:00", "Some One", "old"), Uuid::nil())],
            None,
        );
        let archive = [row(1, "a"), row(2, "b")];
        // the preview repeats the archive's newest row
        let preview = [row(2, "b"), row(3, "c")];
        let all = stitch(legacy, &archive, &preview, 0);
        let texts: Vec<&str> = all.iter().map(|m| m.message.as_str()).collect();
        assert_eq!(texts, ["old", "a", "b", "c"]);
        assert!(all[1].is_service() && all[1].timestamp.is_some());
        let newest = stitch(Vec::new(), &archive, &preview, 2);
        assert_eq!(newest.iter().map(|m| m.message.as_str()).collect::<Vec<_>>(), ["b", "c"]);
    }

    #[test]
    fn the_family_is_the_undated_file_then_the_months() {
        let dir = std::env::temp_dir().join(format!("aurora-stitch-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "Some One-2026-02.txt",
            "Some One.txt",
            "Some One-2025-12.txt",
            "Some One-x.txt",
            "Some Oneself.txt",
        ] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let names: Vec<String> = transcript_family(&dir, "Some One")
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Some One.txt", "Some One-2025-12.txt", "Some One-2026-02.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_family_combines_username_legacy_and_conversation_log_without_duplicates() {
        let dir = std::env::temp_dir().join(format!("aurora-stitch-shared-{}", Uuid::new_v4()));
        let transcripts = dir.join("transcripts");
        let service_dir = dir.join("service");
        std::fs::create_dir_all(&transcripts).unwrap();
        std::fs::create_dir_all(&service_dir).unwrap();

        let agent = Uuid::from_u128(1);
        let peer = Uuid::from_u128(2);
        std::fs::write(
            transcripts.join("conversation.log"),
            format!("[1767261600] 0 0 0 Some One| {peer} {peer} custom_alias|\n"),
        )
        .unwrap();
        std::fs::write(
            transcripts.join("Some One.txt"),
            "[2020/01/01 10:00]  Some One: from legacy\n[2020/01/01 10:01]  You: shared copy\n",
        )
        .unwrap();
        std::fs::write(
            transcripts.join("some_one.txt"),
            "[2020/01/01 10:01]  You: shared copy\n[2020/01/01 10:02]  Some One: from username\n",
        )
        .unwrap();
        std::fs::write(
            transcripts.join("custom_alias.txt"),
            "[2020/01/01 10:03]  Some One: from conversation log alias\n",
        )
        .unwrap();

        let storage = Storage::new(service_dir, agent);
        let read = ViewRead {
            epoch: 1,
            archive_serial: 0,
            deleted_before_ticks: 0,
            include_service: true,
            preview: vec![row(1, "from the service")],
        };
        let result = read_stitched(
            &storage,
            agent,
            peer,
            Some(TranscriptFamily {
                dir: transcripts,
                peer: Some(peer),
                stem: "Some One".into(),
                shared: true,
            }),
            &read,
            0,
        );
        let texts: Vec<&str> = result.messages.iter().map(|m| m.message.as_str()).collect();
        assert_eq!(
            texts,
            [
                "from legacy",
                "shared copy",
                "from username",
                "from conversation log alias",
                "from the service",
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
