//! The per-resident archive of the Chat Service's history: one RFC 4180
//! CSV file, the exact header then one row per message in strictly
//! ascending TimeUUID order. Port of the CSV half of LLChatServiceHistoryCore
//! (quoteCsv, writeCsvRow, readRecord, rowFromFields and the pure part of
//! scanArchive; indra/newview/llchatservicehistorycore.cpp, LL viewer 26.5,
//! LGPL 2.1). The file system stays out: the scan takes the file's bytes.

use aurora_net::chat_service::{
    HistoryRow, MAX_FROM_NAME_BYTES, MAX_MESSAGE_BYTES, TimeUuidKey, clean_text, direct_conversation_id, parse_canonical_uuid,
    parse_created_at, parse_time_uuid, persisted_direct_dialog,
};
use std::borrow::Cow;
use std::collections::VecDeque;
use uuid::Uuid;

/// The first record of every archive (CSV_HEADER).
pub const CSV_HEADER: &str = "conversation_id,msg_id,from_id,from_name,message,dialog,created_at\n";
/// The first record of the non-authoritative index of archives
/// (INDEX_HEADER); its `agent_id` column holds the resident's id.
pub const INDEX_HEADER: &str = "filename,agent_id,agent_name,display_name\n";

const COLUMN_COUNT: usize = 7;
const HEADER: [&str; COLUMN_COUNT] = [
    "conversation_id",
    "msg_id",
    "from_id",
    "from_name",
    "message",
    "dialog",
    "created_at",
];

/// A field as RFC 4180 writes it: quoted, with doubled quotes, only when it
/// holds a comma, a quote or a line break. Port of
/// LLChatServiceHistoryCore::quoteCsv.
pub fn quote_csv(value: &str) -> Cow<'_, str> {
    if !value.contains([',', '"', '\r', '\n']) {
        return Cow::Borrowed(value);
    }
    Cow::Owned(format!("\"{}\"", value.replace('"', "\"\"")))
}

/// One archive row, LF-terminated; the dialog is plain decimal. Port of
/// LLChatServiceHistoryCore::writeCsvRow.
pub fn write_csv_row(out: &mut String, row: &HistoryRow) {
    let fields = [
        quote_csv(&row.conversation_id),
        quote_csv(&row.msg_id),
        Cow::Owned(row.from_id.to_string()),
        quote_csv(&row.from_name),
        quote_csv(&row.message),
        Cow::Owned(row.dialog.to_string()),
        quote_csv(&row.created_at),
    ];
    out.push_str(&fields.join(","));
    out.push('\n');
}

/// One decoded record. `eof_torn` marks an end of file inside the record
/// (an open quote, or too few columns), the repairable torn tail; a record
/// with `had_record` false is the clean end of the file.
struct Record {
    fields: Vec<Vec<u8>>,
    eof_torn: bool,
    had_record: bool,
}

/// The next RFC 4180 record from `pos`, or `None` when its bytes can never
/// be valid (a quote inside an unquoted field, bytes after a closing quote).
/// LF, CR and CRLF all end a record; quoted fields keep their delimiters and
/// line breaks. Validation and torn-tail recovery share this one parser.
/// Port of LLChatServiceHistoryCore::readRecord.
fn read_record(input: &[u8], pos: &mut usize) -> Option<Record> {
    let mut fields = Vec::new();
    let mut field = Vec::new();
    let mut quoted = false;
    let mut quote_closed = false;
    let mut started = false;
    loop {
        let Some(&ch) = input.get(*pos) else {
            // A final record without its line break is complete when it has
            // every column and no open quote.
            let eof_torn = started && (quoted || fields.len() + 1 < COLUMN_COUNT);
            if started && !eof_torn {
                fields.push(field);
            }
            return Some(Record {
                fields,
                eof_torn,
                had_record: started,
            });
        };
        *pos += 1;
        started = true;
        if quoted {
            if ch != b'"' {
                field.push(ch);
            } else if input.get(*pos) == Some(&b'"') {
                *pos += 1;
                field.push(b'"');
            } else {
                quoted = false;
                quote_closed = true;
            }
            continue;
        }
        match ch {
            b'"' if !field.is_empty() || quote_closed => return None,
            b'"' => quoted = true,
            b',' => {
                fields.push(std::mem::take(&mut field));
                quote_closed = false;
            }
            b'\n' | b'\r' => {
                if ch == b'\r' && input.get(*pos) == Some(&b'\n') {
                    *pos += 1;
                }
                fields.push(field);
                return Some(Record {
                    fields,
                    eof_torn: false,
                    had_record: true,
                });
            }
            _ if quote_closed => return None,
            _ => field.push(ch),
        }
    }
}

/// A stored dialog: plain decimal, no sign, no leading zero, in the
/// allowlist (decimalDialog).
fn decimal_dialog(text: &str) -> Option<i32> {
    if text.is_empty() || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<i32>().ok().filter(|d| persisted_direct_dialog(*d))
}

/// A stored row, trusted only once every field and the cross-field identity
/// match this account and this resident. Port of
/// LLChatServiceHistoryCore::rowFromFields.
fn row_from_fields(fields: &[Vec<u8>], agent_id: Uuid, resident_id: Uuid) -> Option<HistoryRow> {
    let [conversation_id, msg_id, from_id, from_name, message, dialog, created_at] = fields else {
        return None;
    };
    let text = |bytes: &[u8]| std::str::from_utf8(bytes).ok().map(str::to_owned);
    let conversation_id = text(conversation_id)?;
    let msg_id = text(msg_id)?;
    let from_name = text(from_name)?;
    let message = text(message)?;
    if conversation_id != direct_conversation_id(agent_id, resident_id) {
        return None;
    }
    let key = parse_time_uuid(&msg_id)?;
    let from_id = parse_canonical_uuid(&text(from_id)?)?;
    if (from_id != agent_id && from_id != resident_id)
        || !clean_text(&from_name, MAX_FROM_NAME_BYTES)
        || !clean_text(&message, MAX_MESSAGE_BYTES)
    {
        return None;
    }
    let dialog = decimal_dialog(&text(dialog)?)?;
    let created_at = parse_created_at(&text(created_at)?)?;
    Some(HistoryRow {
        conversation_id,
        msg_id,
        key,
        from_id,
        from_name,
        message,
        dialog,
        created_at,
    })
}

/// What a scan found (EArchiveState). The pure scan gives `Valid`, `Torn`
/// or `Corrupt`; `Absent` and `Failed` are the file system's answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ArchiveState {
    /// No file.
    #[default]
    Absent,
    /// The exact header and every record valid.
    Valid,
    /// Only the last record is incomplete: repairable by truncating to
    /// `valid_prefix_bytes`.
    Torn,
    /// A bad header, a bad complete record or a key out of order.
    Corrupt,
    /// Not a regular file, or it could not be read.
    Failed,
}

/// An archive folded into a summary and a bounded display window, without
/// keeping the whole file (LLChatServiceHistoryCore::ArchiveScan, less its
/// file stamps, which belong to the file system side).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveScan {
    pub state: ArchiveState,
    /// Rows newer than the deletion cutoff.
    pub row_count: u32,
    /// The oldest and newest key newer than the cutoff, `None` when no row
    /// is: the oldest bounds the legacy transcript, even when the display
    /// window does not reach back to it.
    pub oldest: Option<TimeUuidKey>,
    pub newest: Option<TimeUuidKey>,
    /// The offset just after the last valid record (after the header when
    /// no row is valid): where a torn tail is cut.
    pub valid_prefix_bytes: u64,
    /// The newest `display_cap` rows newer than the cutoff, oldest first.
    pub display_rows: Vec<HistoryRow>,
}

/// Classify an archive's bytes and summarise it. Rows at or below
/// `deleted_before_ticks` are still checked (structure and order) but
/// neither counted nor shown; `display_cap` 0 shows none. The scan never
/// repairs anything: the caller decides to show, cut or quarantine. Port
/// of the reading part of LLChatServiceHistoryCore::scanArchive.
pub fn scan_archive(bytes: &[u8], agent_id: Uuid, resident_id: Uuid, deleted_before_ticks: u64, display_cap: usize) -> ArchiveScan {
    let mut scan = ArchiveScan::default();
    let mut pos = 0;

    // The exact header is the first committed record, never repairable data.
    match read_record(bytes, &mut pos) {
        Some(r) if r.had_record && !r.eof_torn && r.fields.iter().map(Vec::as_slice).eq(HEADER.map(str::as_bytes)) => {}
        _ => {
            scan.state = ArchiveState::Corrupt;
            return scan;
        }
    }
    scan.valid_prefix_bytes = pos as u64;

    let mut previous: Option<TimeUuidKey> = None;
    let mut newest = VecDeque::new();
    loop {
        let Some(record) = read_record(bytes, &mut pos) else {
            scan.state = ArchiveState::Corrupt;
            return scan;
        };
        if !record.had_record || record.eof_torn {
            scan.state = if record.eof_torn { ArchiveState::Torn } else { ArchiveState::Valid };
            break;
        }
        let row = match row_from_fields(&record.fields, agent_id, resident_id) {
            Some(row) if previous.is_none_or(|p| p < row.key) => row,
            _ => {
                scan.state = ArchiveState::Corrupt;
                return scan;
            }
        };
        previous = Some(row.key);
        scan.valid_prefix_bytes = pos as u64;

        if row.key.ticks <= deleted_before_ticks {
            continue;
        }
        scan.oldest.get_or_insert(row.key);
        scan.newest = Some(row.key);
        scan.row_count += 1;
        if display_cap > 0 {
            newest.push_back(row);
            if newest.len() > display_cap {
                newest.pop_front();
            }
        }
    }
    scan.display_rows = newest.into();
    scan
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: Uuid = Uuid::from_u128(1);
    const RESIDENT: Uuid = Uuid::from_u128(2);
    const FIRST: &str = "00000001-0000-1000-8000-000000000000";
    const SECOND: &str = "00000002-0000-1000-8000-000000000000";

    fn row(msg_id: &str, from_id: Uuid, message: &str) -> HistoryRow {
        HistoryRow {
            conversation_id: direct_conversation_id(AGENT, RESIDENT),
            msg_id: msg_id.to_owned(),
            key: parse_time_uuid(msg_id).expect("fixture id parses"),
            from_id,
            from_name: "Resident".to_owned(),
            message: message.to_owned(),
            dialog: 0,
            created_at: "2026-08-12T12:34:56Z".to_owned(),
        }
    }

    /// The minimal ascending archive of the valid, torn and corrupt cases.
    fn valid_archive() -> String {
        let mut output = CSV_HEADER.to_owned();
        write_csv_row(&mut output, &row(FIRST, AGENT, "first"));
        write_csv_row(&mut output, &row(SECOND, RESIDENT, "second, quoted"));
        output
    }

    #[test]
    fn case_08_rfc4180_quoting() {
        assert_eq!(quote_csv("comma, quote \" and\nnewline"), "\"comma, quote \"\" and\nnewline\"");
        assert!(matches!(quote_csv("plain"), Cow::Borrowed("plain")));
    }

    #[test]
    fn case_09_valid_archive_keeps_the_durable_oldest_under_the_display_cap() {
        let scan = scan_archive(valid_archive().as_bytes(), AGENT, RESIDENT, 0, 1);
        assert_eq!(scan.state, ArchiveState::Valid, "valid archive");
        assert_eq!(scan.row_count, 2, "two summarized");
        assert_eq!(scan.oldest, parse_time_uuid(FIRST), "display cap preserves durable oldest");
        assert_eq!(scan.display_rows.len(), 1, "newest display cap");
        assert_eq!(scan.display_rows[0].msg_id, SECOND, "newest retained");
        assert_eq!(scan.display_rows[0].message, "second, quoted");
    }

    #[test]
    fn case_10_unterminated_quoted_final_record_is_torn() {
        let valid = valid_archive();
        let content = format!("{valid}\"unterminated");
        let scan = scan_archive(content.as_bytes(), AGENT, RESIDENT, 0, 0);
        assert_eq!(scan.state, ArchiveState::Torn, "torn distinguished");
        assert_eq!(scan.valid_prefix_bytes, valid.len() as u64, "the cut keeps every complete row");
    }

    #[test]
    fn case_11_complete_row_with_an_unknown_dialog_is_corrupt() {
        let mut content = valid_archive();
        let needle = ",0,2026-08-12T12:34:56Z\n";
        let last = content.rfind(needle).expect("fixture has the needle");
        content.replace_range(last..last + needle.len(), ",99,2026-08-12T12:34:56Z\n");
        let scan = scan_archive(content.as_bytes(), AGENT, RESIDENT, 0, 0);
        assert_eq!(scan.state, ArchiveState::Corrupt, "corrupt distinguished");
    }

    #[test]
    fn cutoff_rows_are_checked_but_not_counted() {
        let scan = scan_archive(valid_archive().as_bytes(), AGENT, RESIDENT, 1, 10);
        assert_eq!(scan.state, ArchiveState::Valid);
        assert_eq!(scan.row_count, 1);
        assert_eq!(scan.oldest, parse_time_uuid(SECOND));
        assert_eq!(scan.display_rows.len(), 1);
        // A final row without its line break is complete; CRLF ends a record too.
        let crlf = valid_archive().replace('\n', "\r\n");
        let crlf = crlf.trim_end();
        assert_eq!(scan_archive(crlf.as_bytes(), AGENT, RESIDENT, 0, 0).state, ArchiveState::Valid);
        let bad_header = valid_archive().replacen("created_at", "created", 1);
        assert_eq!(
            scan_archive(bad_header.as_bytes(), AGENT, RESIDENT, 0, 0).state,
            ArchiveState::Corrupt
        );
    }
}
