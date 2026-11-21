//! The Chat Service's wire format: ids, field checks and the two response
//! validators. Port of the wire primitives of LLChatServiceHistoryCore
//! (indra/newview/llchatservicehistorycore.cpp, LL viewer 26.5, LGPL 2.1).

use super::{
    HISTORY_LIMIT, HistoryRow, ListEntry, MAX_FROM_NAME_BYTES, MAX_LIST_CONVERSATION_ID_BYTES, MAX_LIST_ENTRIES, MAX_MESSAGE_BYTES, Page,
    TimeUuidKey,
};
use aurora_llsd::Llsd;
use std::collections::HashSet;
use uuid::Uuid;

/// The id of the direct conversation between two residents: both canonical
/// ids, the byte-wise smaller first, joined by `_`, the same whichever side
/// asks. Port of LLChatServiceHistoryCore::directConversationId.
pub fn direct_conversation_id(agent_id: Uuid, resident_id: Uuid) -> String {
    if agent_id < resident_id {
        format!("{agent_id}_{resident_id}")
    } else {
        format!("{resident_id}_{agent_id}")
    }
}

/// A UUID written exactly as the service and the archive write one: 36
/// characters, lowercase hex, hyphens in place. Port of
/// LLChatServiceHistoryCore::parseCanonicalUuid.
pub fn parse_canonical_uuid(text: &str) -> Option<Uuid> {
    let bytes = text.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    let mut id = [0u8; 16];
    let mut nibbles = 0;
    for (pos, &b) in bytes.iter().enumerate() {
        if matches!(pos, 8 | 13 | 18 | 23) {
            if b != b'-' {
                return None;
            }
            continue;
        }
        let value = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            _ => return None,
        };
        id[nibbles / 2] |= value << if nibbles % 2 == 0 { 4 } else { 0 };
        nibbles += 1;
    }
    Some(Uuid::from_bytes(id))
}

/// The ordering key of a canonical version-1 (RFC 4122 variant) message id.
/// Port of LLChatServiceHistoryCore::parseTimeUuid.
pub fn parse_time_uuid(text: &str) -> Option<TimeUuidKey> {
    let id = parse_canonical_uuid(text)?;
    let b = id.as_bytes();
    if b[6] >> 4 != 1 || b[8] & 0xc0 != 0x80 {
        return None;
    }
    // UUIDv1 stores the 60-bit timestamp across three network-order fields.
    let time_low = u64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let time_mid = u64::from(u16::from_be_bytes([b[4], b[5]]));
    let time_high = (u64::from(b[6] & 0x0f) << 8) | u64::from(b[7]);
    let mut tail = [0i8; 8];
    for (t, &byte) in tail.iter_mut().zip(&b[8..]) {
        *t = byte as i8;
    }
    Some(TimeUuidKey {
        ticks: (time_high << 48) | (time_mid << 32) | time_low,
        tail,
    })
}

/// The message id of a key, so that an archive boundary can travel with a
/// live message; the default (absent) key gives an empty string. Port of
/// LLChatServiceHistoryCore::formatTimeUuid.
pub fn format_time_uuid(key: &TimeUuidKey) -> String {
    if *key == TimeUuidKey::default() {
        return String::new();
    }
    let t = key.ticks;
    let mut b = [0u8; 16];
    b[..4].copy_from_slice(&(t as u32).to_be_bytes());
    b[4] = (t >> 40) as u8;
    b[5] = (t >> 32) as u8;
    b[6] = ((t >> 56) & 0x0f) as u8 | 0x10;
    b[7] = (t >> 48) as u8;
    for (byte, &t) in b[8..].iter_mut().zip(&key.tail) {
        *byte = t as u8;
    }
    Uuid::from_bytes(b).to_string()
}

/// The IM dialogs a direct history keeps: IM_NOTHING_SPECIAL,
/// IM_GROUP_INVITATION, IM_INVENTORY_OFFERED, IM_DO_NOT_DISTURB_AUTO_RESPONSE,
/// IM_LURE_USER, IM_TELEPORT_REQUEST and IM_FRIENDSHIP_OFFERED. Port of
/// LLChatServiceHistoryCore::persistedDirectDialog.
pub fn persisted_direct_dialog(dialog: i32) -> bool {
    matches!(dialog, 0 | 3 | 4 | 20 | 22 | 26 | 38)
}

/// Wire and archive text stays bounded and NUL-free (cleanText). The LL
/// viewer also round-trips it through UTF-32 to refuse non-canonical UTF-8:
/// a `str` already is canonical, and the archive reader refuses invalid
/// bytes before calling this.
pub fn clean_text(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.contains('\0')
}

/// The fields of a `created_at` that matched the shape of
/// LLChatServiceHistoryCore::parseCreatedAt's regular expression:
/// `YYYY-MM-DDTHH:MM:SS`, an optional non-empty fraction, an optional `Z`
/// or `±HH:MM` zone, and nothing else.
struct CreatedAt<'a> {
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    /// The digits after the dot, empty when there is no fraction.
    fraction: &'a str,
    /// The zone's offset east of UTC in seconds; `None` without a zone.
    offset: Option<i64>,
}

fn created_at_shape(text: &str) -> Option<CreatedAt<'_>> {
    let b = text.as_bytes();
    let digits = |from: usize, len: usize| -> Option<u32> {
        let field = b.get(from..from + len)?;
        field
            .iter()
            .try_fold(0u32, |n, &d| d.is_ascii_digit().then(|| n * 10 + u32::from(d - b'0')))
    };
    let at = |pos: usize, ch: u8| b.get(pos) == Some(&ch);
    if !(at(4, b'-') && at(7, b'-') && at(10, b'T') && at(13, b':') && at(16, b':')) {
        return None;
    }
    let (year, month, day) = (digits(0, 4)?, digits(5, 2)?, digits(8, 2)?);
    let (hour, minute, second) = (digits(11, 2)?, digits(14, 2)?, digits(17, 2)?);
    // [01][0-9]|2[0-3] and [0-5][0-9]: the shape, not the calendar.
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut pos = 19;
    let mut fraction = "";
    if at(pos, b'.') {
        let len = b[pos + 1..].iter().take_while(|d| d.is_ascii_digit()).count();
        if len == 0 {
            return None;
        }
        fraction = &text[pos + 1..pos + 1 + len];
        pos += 1 + len;
    }
    let offset = match b.get(pos) {
        None => None,
        Some(b'Z') => {
            pos += 1;
            Some(0)
        }
        Some(&sign @ (b'+' | b'-')) => {
            let (hours, minutes) = (digits(pos + 1, 2)?, digits(pos + 4, 2)?);
            if !at(pos + 3, b':') || hours > 23 || minutes > 59 {
                return None;
            }
            pos += 6;
            let seconds = i64::from(hours * 3600 + minutes * 60);
            Some(if sign == b'+' { seconds } else { -seconds })
        }
        Some(_) => return None,
    };
    (pos == b.len()).then_some(CreatedAt {
        year,
        month,
        day,
        hour,
        minute,
        second,
        fraction,
        offset,
    })
}

/// Days from 1970-01-01 to a proleptic Gregorian date (month 1-12).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl CreatedAt<'_> {
    /// The calendar check of parseCreatedAt, then LLDate::fromString's own
    /// refusal of a wall time before 1970 (apr_time_exp_gmt_get fails on a
    /// negative time, before the zone offset applies).
    fn wall_seconds(&self) -> Option<i64> {
        const DAYS_PER_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        if self.year == 0 || !(1..=12).contains(&self.month) {
            return None;
        }
        let leap = self.year.is_multiple_of(400) || (self.year.is_multiple_of(4) && !self.year.is_multiple_of(100));
        let month_days = DAYS_PER_MONTH[self.month as usize - 1] + u32::from(self.month == 2 && leap);
        if self.day < 1 || self.day > month_days {
            return None;
        }
        let days = days_from_civil(i64::from(self.year), self.month, self.day);
        let seconds = days * 86_400 + i64::from(self.hour * 3600 + self.minute * 60 + self.second);
        (seconds >= 0).then_some(seconds)
    }
}

/// A row's `created_at`, normalised: the whole ISO 8601 shape must match,
/// with a real calendar date, and a time without zone is UTC and gets a
/// `Z`; an explicit zone and the fraction's precision are kept. Port of
/// LLChatServiceHistoryCore::parseCreatedAt.
pub fn parse_created_at(text: &str) -> Option<String> {
    let parsed = created_at_shape(text)?;
    parsed.wall_seconds()?;
    Some(if parsed.offset.is_some() {
        text.to_owned()
    } else {
        format!("{text}Z")
    })
}

/// The Unix time of a `created_at` (LLDate::secondsSinceEpoch, fraction
/// included, zone offset applied); the service rows' `timestamp` is this
/// truncated to whole seconds.
pub fn created_at_seconds(text: &str) -> Option<f64> {
    let parsed = created_at_shape(text)?;
    let wall = parsed.wall_seconds()?;
    let fraction = if parsed.fraction.is_empty() {
        0.0
    } else {
        format!("0.{}", parsed.fraction).parse::<f64>().ok()?
    };
    Some((wall - parsed.offset.unwrap_or(0)) as f64 + fraction)
}

/// A field that must be an LLSD string: a coercible value (an integer, a
/// UUID) is a refusal, so that paging state cannot drift on a malformed
/// answer.
fn llsd_string<'a>(map: &'a Llsd, name: &str) -> Option<&'a str> {
    match map.get(name) {
        Llsd::String(s) => Some(s),
        _ => None,
    }
}

/// The direct conversations of a conversation-list answer, all or nothing:
/// one malformed entry (unknown type, non-canonical id, a direct entry
/// whose id is not this pair's, a repeated resident) refuses the whole
/// list. Group and ad-hoc entries are checked, then left out; the null
/// participant (system traffic) is accepted but has no history of its own.
/// Port of LLChatServiceHistoryCore::validateConversationList.
pub fn validate_conversation_list(value: &Llsd, agent_id: Uuid) -> Option<Vec<ListEntry>> {
    let Llsd::Array(list) = value else {
        return None;
    };
    if list.len() > MAX_LIST_ENTRIES {
        return None;
    }
    let mut entries = Vec::new();
    let mut residents = HashSet::new();
    for item in list {
        if !item.is_map() {
            return None;
        }
        let kind = llsd_string(item, "conversation_type")?;
        let conversation = llsd_string(item, "conversation_id")?;
        if conversation.is_empty()
            || !clean_text(conversation, MAX_LIST_CONVERSATION_ID_BYTES)
            || !matches!(kind, "direct" | "group" | "adhoc")
        {
            return None;
        }
        if kind != "direct" {
            continue;
        }
        let resident_text = llsd_string(item, "other_participant_id")?;
        let token = llsd_string(item, "last_msg_id")?;
        let resident = parse_canonical_uuid(resident_text)?;
        if resident == agent_id
            || parse_time_uuid(token).is_none()
            || conversation != direct_conversation_id(agent_id, resident)
            || !residents.insert(resident)
        {
            return None;
        }
        if !resident.is_nil() {
            entries.push(ListEntry {
                resident_id: resident,
                last_msg_id: token.to_owned(),
            });
        }
    }
    Some(entries)
}

/// One message of a history page, every field checked (the per-row half
/// of validateHistoryPage).
fn page_row(item: &Llsd, conversation_id: &str, agent_id: Uuid, resident_id: Uuid) -> Option<HistoryRow> {
    if !item.is_map() {
        return None;
    }
    let row_conversation = llsd_string(item, "conversation_id")?;
    let msg_id = llsd_string(item, "msg_id")?;
    let from_text = llsd_string(item, "from_id")?;
    let from_name = llsd_string(item, "from_name")?;
    let message = llsd_string(item, "message")?;
    let Llsd::Integer(dialog) = *item.get("dialog") else {
        return None;
    };
    let created_at = llsd_string(item, "created_at")?;
    if row_conversation != conversation_id {
        return None;
    }
    let key = parse_time_uuid(msg_id)?;
    let from_id = parse_canonical_uuid(from_text)?;
    if (from_id != agent_id && from_id != resident_id)
        || !clean_text(from_name, MAX_FROM_NAME_BYTES)
        || !clean_text(message, MAX_MESSAGE_BYTES)
        || !persisted_direct_dialog(dialog)
    {
        return None;
    }
    Some(HistoryRow {
        conversation_id: row_conversation.to_owned(),
        msg_id: msg_id.to_owned(),
        key,
        from_id,
        from_name: from_name.to_owned(),
        message: message.to_owned(),
        dialog,
        created_at: parse_created_at(created_at)?,
    })
}

/// A history page, all or nothing. The service echoes this conversation,
/// the integer limit 100 and, on an older page, the exact cursor (the head
/// page has no `before_msg_id` at all). Rows must be strictly newest-first
/// and older than the cursor, which also refuses a repeated id within a
/// page and the cursor row coming back. Rows at or below
/// `deleted_before_ticks` (inclusive) are still validated but dropped, and
/// end the paging. Port of LLChatServiceHistoryCore::validateHistoryPage.
pub fn validate_history_page(
    value: &Llsd,
    agent_id: Uuid,
    resident_id: Uuid,
    requested_cursor: &str,
    deleted_before_ticks: u64,
) -> Option<Page> {
    let conversation_id = direct_conversation_id(agent_id, resident_id);
    if !value.is_map()
        || llsd_string(value, "conversation_id") != Some(conversation_id.as_str())
        || *value.get("limit") != Llsd::Integer(HISTORY_LIMIT)
    {
        return None;
    }
    let Llsd::Array(messages) = value.get("messages") else {
        return None;
    };
    if messages.len() > HISTORY_LIMIT as usize {
        return None;
    }
    let echoed = value.get("before_msg_id");
    let cursor_key = if requested_cursor.is_empty() {
        if !echoed.is_undef() {
            return None;
        }
        None
    } else {
        if llsd_string(value, "before_msg_id") != Some(requested_cursor) {
            return None;
        }
        Some(parse_time_uuid(requested_cursor)?)
    };

    let mut page = Page::default();
    let mut previous: Option<TimeUuidKey> = None;
    for item in messages {
        let row = page_row(item, &conversation_id, agent_id, resident_id)?;
        if previous.is_some_and(|p| row.key >= p) || cursor_key.is_some_and(|c| row.key >= c) {
            return None;
        }
        previous = Some(row.key);
        page.next_cursor.clone_from(&row.msg_id);
        if row.key.ticks <= deleted_before_ticks {
            page.terminal = true;
        } else {
            page.rows.push(row);
        }
    }
    page.terminal |= messages.is_empty();
    if page.terminal {
        page.next_cursor.clear();
    }
    Some(page)
}

/// The body of a history request: this pair's conversation, the fixed
/// limit, and `before_msg_id` (the previous page's oldest id) on every page
/// but the head one (LLChatServiceHistory::syncResident).
pub fn history_request_body(agent_id: Uuid, resident_id: Uuid, cursor: &str) -> Llsd {
    let mut body = Llsd::new_map();
    body.insert("conversation_id", direct_conversation_id(agent_id, resident_id));
    body.insert("limit", HISTORY_LIMIT);
    if !cursor.is_empty() {
        body.insert("before_msg_id", cursor);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: Uuid = Uuid::from_u128(1);
    const RESIDENT: Uuid = Uuid::from_u128(2);
    const FIRST: &str = "00000001-0000-1000-8000-000000000000";
    const SECOND: &str = "00000002-0000-1000-8000-000000000000";

    fn wire_row(id: &str, from: Uuid) -> Llsd {
        let mut row = Llsd::new_map();
        row.insert("conversation_id", direct_conversation_id(AGENT, RESIDENT));
        row.insert("msg_id", id);
        row.insert("from_id", from.to_string());
        row.insert("from_name", "Resident");
        row.insert("message", "hello");
        row.insert("dialog", 0);
        row.insert("created_at", "2026-08-12T12:34:56.123Z");
        row
    }

    fn wire_page(rows: Vec<Llsd>, cursor: &str) -> Llsd {
        let mut page = Llsd::new_map();
        page.insert("conversation_id", direct_conversation_id(AGENT, RESIDENT));
        page.insert("limit", 100);
        page.insert("messages", rows);
        if !cursor.is_empty() {
            page.insert("before_msg_id", cursor);
        }
        page
    }

    fn list_entry(resident: Uuid) -> Llsd {
        let mut entry = Llsd::new_map();
        entry.insert("conversation_type", "direct");
        entry.insert("conversation_id", direct_conversation_id(AGENT, resident));
        entry.insert("other_participant_id", resident.to_string());
        entry.insert("last_msg_id", SECOND);
        entry
    }

    #[test]
    fn case_01_direct_conversation_id_is_order_independent() {
        assert_eq!(direct_conversation_id(RESIDENT, AGENT), format!("{AGENT}_{RESIDENT}"));
        assert_eq!(direct_conversation_id(AGENT, RESIDENT), format!("{AGENT}_{RESIDENT}"));
    }

    #[test]
    fn case_02_time_uuid_accepts_canonical_v1_in_time_order() {
        let first = parse_time_uuid(FIRST).expect("UUIDv1 parses");
        let second = parse_time_uuid(SECOND).expect("UUIDv1 parses");
        assert!(first < second, "timestamp ordering");
        assert!(parse_time_uuid("00000001-0000-4000-8000-000000000000").is_none(), "UUIDv4 rejected");
        assert!(
            parse_time_uuid("00000001-0000-1000-8000-00000000000A").is_none(),
            "noncanonical UUID rejected"
        );
    }

    #[test]
    fn case_03_equal_ticks_compare_signed_tail_bytes() {
        let low = parse_time_uuid(FIRST).expect("parses");
        let high = parse_time_uuid("00000001-0000-1000-bfff-000000000000").expect("parses");
        assert!(low < high, "Cassandra signed tail ordering");
    }

    #[test]
    fn case_04_descending_page_cursor_and_inclusive_cutoff() {
        let mut rows = vec![wire_row(SECOND, RESIDENT), wire_row(FIRST, AGENT)];
        let page = validate_history_page(&wire_page(rows.clone(), ""), AGENT, RESIDENT, "", 0).expect("strict descending page");
        assert_eq!(page.rows.len(), 2, "all rows retained");
        assert_eq!(page.next_cursor, FIRST, "oldest cursor");

        // The deletion boundary is inclusive and ends paging, but not validation.
        let page = validate_history_page(&wire_page(rows.clone(), ""), AGENT, RESIDENT, "", 1).expect("cutoff page accepted");
        assert_eq!(page.rows.len(), 1, "only newer rows retained");
        assert_eq!(page.rows[0].msg_id, SECOND, "newer row retained");
        assert!(page.terminal && page.next_cursor.is_empty(), "cutoff ends paging");
        let mut bad = wire_row("00000000-0000-1000-8000-000000000000", RESIDENT);
        bad.insert("dialog", 99);
        rows.push(bad);
        assert!(
            validate_history_page(&wire_page(rows, ""), AGENT, RESIDENT, "", 1).is_none(),
            "invalid row below cutoff rejected"
        );
    }

    #[test]
    fn case_05_order_duplicates_cursor_and_exact_types() {
        let rows = vec![wire_row(FIRST, RESIDENT), wire_row(SECOND, RESIDENT)];
        assert!(
            validate_history_page(&wire_page(rows, ""), AGENT, RESIDENT, "", 0).is_none(),
            "ascending page rejected"
        );

        let rows = vec![wire_row(FIRST, RESIDENT), wire_row(FIRST, RESIDENT)];
        assert!(
            validate_history_page(&wire_page(rows, ""), AGENT, RESIDENT, "", 0).is_none(),
            "duplicate row rejected"
        );
        let rows = vec![wire_row(FIRST, RESIDENT)];
        assert!(
            validate_history_page(&wire_page(rows, FIRST), AGENT, RESIDENT, FIRST, 0).is_none(),
            "cursor row cannot repeat on the next page"
        );

        let mut wrong = wire_page(Vec::new(), "");
        wrong.insert("conversation_id", direct_conversation_id(AGENT, Uuid::nil()));
        assert!(
            validate_history_page(&wrong, AGENT, RESIDENT, "", 0).is_none(),
            "another conversation rejected"
        );
        let mut wrong = wire_page(Vec::new(), "");
        wrong.insert("limit", "100");
        assert!(
            validate_history_page(&wrong, AGENT, RESIDENT, "", 0).is_none(),
            "coercible type rejected"
        );
    }

    #[test]
    fn case_06_list_rejects_duplicates_and_ignores_the_null_participant() {
        let entry = list_entry(RESIDENT);
        assert!(
            validate_conversation_list(&Llsd::Array(vec![entry.clone()]), AGENT).is_some(),
            "strict list accepted"
        );
        assert!(
            validate_conversation_list(&Llsd::Array(vec![entry.clone(), entry]), AGENT).is_none(),
            "duplicate resident rejected"
        );
        let entries =
            validate_conversation_list(&Llsd::Array(vec![list_entry(Uuid::nil())]), AGENT).expect("null system participant ignored");
        assert!(entries.is_empty(), "system conversation has no resident history");
    }

    #[test]
    fn case_07_created_at_forms_and_dialog_allowlist() {
        // Accept the supported UTC, explicit-offset, offset-less, and fractional forms.
        assert!(parse_created_at("2026-08-12T12:34:56Z").is_some(), "UTC timestamp");
        assert!(parse_created_at("2026-08-12T12:34:56+02:30").is_some(), "offset timestamp");
        assert_eq!(
            parse_created_at("2026-08-12T12:34:56").as_deref(),
            Some("2026-08-12T12:34:56Z"),
            "offset-less timestamp"
        );
        assert!(parse_created_at("2026-08-12T12:34:56.123Z").is_some(), "fractional timestamp");

        // Reject malformed suffixes and noncanonical field shapes.
        assert!(
            parse_created_at("2026-08-12T12:34:56Z123").is_none(),
            "reject trailing timestamp data"
        );
        assert!(parse_created_at("2026-08-12T12:34:56+99:99").is_none(), "reject invalid offset");
        assert!(parse_created_at("2026-+1-12T12:34:56Z").is_none(), "reject signed date field");
        assert!(parse_created_at("2026-08-12T12:34:56.Z").is_none(), "reject empty fraction");

        // Enforce calendar and clock ranges after the fixed-width shape validates.
        assert!(parse_created_at("2026-13-12T12:34:56Z").is_none(), "reject month");
        assert!(parse_created_at("2026-08-35T12:34:56Z").is_none(), "reject day");
        assert!(parse_created_at("2026-02-29T12:34:56Z").is_none(), "reject nonleap day");
        assert!(parse_created_at("2024-02-29T12:34:56Z").is_some(), "accept leap day");
        assert!(parse_created_at("2026-08-12T24:34:56Z").is_none(), "reject hour");
        assert!(parse_created_at("2026-08-12T12:60:56Z").is_none(), "reject minute");
        assert!(parse_created_at("2026-08-12T12:34:60Z").is_none(), "reject second");
        assert!(parse_created_at("2026-8-12T12:34:56Z").is_none(), "bad width");

        // Persisted history uses an explicit IM-dialog allowlist.
        assert!(persisted_direct_dialog(38) && !persisted_direct_dialog(99), "dialog allowlist");
    }

    #[test]
    fn created_at_seconds_apply_the_fraction_and_the_zone() {
        assert_eq!(created_at_seconds("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(
            created_at_seconds("2026-09-10T17:17:00.904000Z").map(|s| s as i32),
            Some(1_789_060_620)
        );
        assert_eq!(
            created_at_seconds("2026-09-10T19:17:00+02:00"),
            created_at_seconds("2026-09-10T17:17:00Z")
        );
        assert_eq!(
            created_at_seconds("2026-09-10T16:47:00-00:30"),
            created_at_seconds("2026-09-10T17:17:00Z")
        );
        assert!(
            parse_created_at("1969-12-31T23:59:59Z").is_none(),
            "LLDate refuses a wall time before 1970"
        );
    }

    #[test]
    fn history_request_body_carries_the_cursor_on_older_pages_only() {
        let head = history_request_body(AGENT, RESIDENT, "");
        assert_eq!(head["conversation_id"].as_str(), direct_conversation_id(AGENT, RESIDENT));
        assert_eq!(head["limit"], Llsd::Integer(100));
        assert!(!head.has("before_msg_id"));
        assert_eq!(history_request_body(AGENT, RESIDENT, FIRST)["before_msg_id"].as_str(), FIRST);
    }

    #[test]
    fn case_36_system_entry_does_not_block_resident_discovery() {
        let peer = list_entry(RESIDENT);
        let system = list_entry(Uuid::nil());
        for system_first in [true, false] {
            let list = if system_first {
                vec![system.clone(), peer.clone()]
            } else {
                vec![peer.clone(), system.clone()]
            };
            let entries = validate_conversation_list(&Llsd::Array(list), AGENT).expect("system entry does not block resident discovery");
            assert_eq!(entries.len(), 1, "only the resident is scheduled");
            assert_eq!(entries[0].resident_id, RESIDENT, "resident identity preserved");
            assert_eq!(entries[0].last_msg_id, SECOND, "history token preserved");
        }
    }

    #[test]
    fn case_75_format_time_uuid_round_trips_every_bit() {
        assert_eq!(
            format_time_uuid(&TimeUuidKey::default()),
            "",
            "the default service key formats as empty"
        );
        for id in [
            FIRST,
            SECOND,
            "00000000-0000-1000-8000-000000000000",
            "ffffffff-0000-1000-80ff-7f0080ff7f00",
            "00000000-0001-1000-80ff-7f0080ff7f00",
            "ffffffff-ffff-1fff-bfff-ffffffffffff",
        ] {
            let key = parse_time_uuid(id).expect("boundary fixture parses");
            assert_eq!(format_time_uuid(&key), id, "formatting preserves the canonical boundary");
            let restored = parse_time_uuid(&format_time_uuid(&key)).expect("formatted boundary parses again");
            assert_eq!(restored, key, "all timestamp and tail bits survive");
        }
        let before = parse_time_uuid("ffffffff-0000-1000-8000-000000000000").expect("parses");
        let after = parse_time_uuid("00000000-0001-1000-8000-000000000000").expect("parses");
        assert!(before < after, "the timestamp rollover keeps UUID ordering");
    }

    #[test]
    fn staged_bytes_follow_the_pass_estimate() {
        let page = validate_history_page(&wire_page(vec![wire_row(FIRST, AGENT)], ""), AGENT, RESIDENT, "", 0).expect("valid");
        let row = &page.rows[0];
        let expected = row.conversation_id.len() + 36 + 36 + "Resident".len() + "hello".len() + 1 + row.created_at.len();
        assert_eq!(row.staged_bytes(), expected);
    }
}
