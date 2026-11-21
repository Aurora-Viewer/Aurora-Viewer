//! The Chat Service's wire format: ids, field checks and the two response
//! validators, which also tell why they refused an answer (the LL viewer
//! refuses silently; the reason only goes to the log, never row text).
//! Port of the wire primitives of LLChatServiceHistoryCore
//! (indra/newview/llchatservicehistorycore.cpp, LL viewer 26.5, LGPL 2.1).

use super::{
    ConversationList, HISTORY_LIMIT, HistoryRow, ListEntry, MAX_FROM_NAME_BYTES, MAX_LIST_CONVERSATION_ID_BYTES, MAX_LIST_ENTRIES,
    MAX_MESSAGE_BYTES, Page, TimeUuidKey,
};
use aurora_llsd::Llsd;
use std::collections::HashSet;
use std::fmt;
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

/// Characters of an unknown `conversation_type` kept in a refusal.
const TYPE_EXCERPT_CHARS: usize = 32;
/// Characters of a received id (resident, message, conversation) kept in a
/// refusal: a direct conversation id is 73.
const ID_EXCERPT_CHARS: usize = 80;

/// The name of an LLSD value's type, for a refusal; `undef` is also what a
/// missing map field reads as.
fn llsd_kind(value: &Llsd) -> &'static str {
    match value {
        Llsd::Undef => "undef",
        Llsd::Boolean(_) => "boolean",
        Llsd::Integer(_) => "integer",
        Llsd::Real(_) => "real",
        Llsd::String(_) => "string",
        Llsd::Uuid(_) => "uuid",
        Llsd::Date(_) => "date",
        Llsd::Uri(_) => "uri",
        Llsd::Binary(_) => "binary",
        Llsd::Array(_) => "array",
        Llsd::Map(_) => "map",
    }
}

/// A received id or type for the log: at most `max` characters, everything
/// but printable ASCII escaped, `...` when cut. Only ids and types go
/// through it, never a message, a name or a time.
fn excerpt(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars().take(max) {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            ' '..='~' => out.push(c),
            _ => out.extend(c.escape_unicode()),
        }
    }
    if text.chars().nth(max).is_some() {
        out.push_str("...");
    }
    out
}

/// A field of the wrong LLSD type, or missing (`found` is then `undef`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldFault {
    pub field: &'static str,
    pub expected: &'static str,
    pub found: &'static str,
}

impl fmt::Display for FieldFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.found == "undef" {
            write!(f, "{} missing", self.field)
        } else {
            write!(f, "{} is {}, expected {}", self.field, self.found, self.expected)
        }
    }
}

/// Why a text failed [`clean_text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFault {
    TooLong { bytes: usize, limit: usize },
    Nul,
}

impl fmt::Display for TextFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { bytes, limit } => write!(f, "too long ({bytes} bytes, limit {limit})"),
            Self::Nul => f.write_str("contains NUL"),
        }
    }
}

/// [`clean_text`]'s refusal, if any.
fn text_fault(value: &str, limit: usize) -> Option<TextFault> {
    if value.len() > limit {
        Some(TextFault::TooLong { bytes: value.len(), limit })
    } else if value.contains('\0') {
        Some(TextFault::Nul)
    } else {
        None
    }
}

/// Why an id failed [`parse_canonical_uuid`] or [`parse_time_uuid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdFault {
    /// Not 36 bytes long.
    Length(usize),
    /// 36 bytes, but not lowercase hex with the hyphens in place.
    NotCanonical,
    /// Canonical, but its version nibble is not 1.
    Version(u8),
    /// Version 1, but not the RFC 4122 variant.
    Variant,
}

impl fmt::Display for IdFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length(bytes) => write!(f, "{bytes} bytes, expected 36"),
            Self::NotCanonical => f.write_str("not lowercase hex with hyphens"),
            Self::Version(version) => write!(f, "version {version}, expected 1"),
            Self::Variant => f.write_str("not the RFC 4122 variant"),
        }
    }
}

/// Why `text` is not a canonical UUID (only asked once it was refused).
fn canonical_fault(text: &str) -> IdFault {
    if text.len() == 36 {
        IdFault::NotCanonical
    } else {
        IdFault::Length(text.len())
    }
}

/// Why `text` is not a time UUID (only asked once it was refused).
fn time_uuid_fault(text: &str) -> IdFault {
    match parse_canonical_uuid(text) {
        None => canonical_fault(text),
        Some(id) if id.as_bytes()[6] >> 4 != 1 => IdFault::Version(id.as_bytes()[6] >> 4),
        Some(_) => IdFault::Variant,
    }
}

/// A field that must be an LLSD string: a coercible value (an integer, a
/// UUID) is a refusal, so that paging state cannot drift on a malformed
/// answer.
fn string_field<'a>(map: &'a Llsd, field: &'static str) -> Result<&'a str, FieldFault> {
    match map.get(field) {
        Llsd::String(s) => Ok(s),
        other => Err(FieldFault {
            field,
            expected: "string",
            found: llsd_kind(other),
        }),
    }
}

/// What made [`validate_conversation_list_checked`] refuse a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListFault {
    /// The answer is not an array (its LLSD type).
    NotArray(&'static str),
    /// More than [`MAX_LIST_ENTRIES`] entries (their number).
    TooManyEntries(usize),
    /// An entry is not a map (its LLSD type).
    EntryNotMap(&'static str),
    Field(FieldFault),
    EmptyConversationId,
    ConversationId(TextFault),
    /// The `conversation_type`, cut and escaped.
    UnknownType(String),
    /// A direct entry's `other_participant_id`, cut and escaped.
    ParticipantNotCanonical {
        value: String,
        fault: IdFault,
    },
    /// A direct entry's `last_msg_id`, cut and escaped.
    LastMsgIdNotTimeUuid {
        value: String,
        fault: IdFault,
    },
    /// A direct entry's `conversation_id` (cut and escaped) is not that of
    /// the agent and `resident`.
    ConversationMismatch {
        value: String,
        resident: Uuid,
    },
    RepeatedResident(Uuid),
}

impl From<FieldFault> for ListFault {
    fn from(fault: FieldFault) -> Self {
        Self::Field(fault)
    }
}

impl fmt::Display for ListFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotArray(kind) => write!(f, "answer is {kind}, expected array"),
            Self::TooManyEntries(count) => write!(f, "{count} entries, limit {MAX_LIST_ENTRIES}"),
            Self::EntryNotMap(kind) => write!(f, "entry is {kind}, expected map"),
            Self::Field(fault) => fmt::Display::fmt(fault, f),
            Self::EmptyConversationId => f.write_str("conversation_id empty"),
            Self::ConversationId(fault) => write!(f, "conversation_id {fault}"),
            Self::UnknownType(value) => write!(f, "unknown conversation_type \"{value}\""),
            Self::ParticipantNotCanonical { value, fault } => {
                write!(f, "other_participant_id \"{value}\" is not a canonical UUID ({fault})")
            }
            Self::LastMsgIdNotTimeUuid { value, fault } => write!(f, "last_msg_id \"{value}\" is not a time UUID ({fault})"),
            Self::ConversationMismatch { value, resident } => {
                write!(f, "direct conversation_id \"{value}\" does not match the pair with {resident}")
            }
            Self::RepeatedResident(resident) => write!(f, "resident {resident} listed twice"),
        }
    }
}

/// Why a conversation list was refused, for the log: the first fault
/// found and, for an entry, its 0-based index in a list of `total`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRefusal {
    pub fault: ListFault,
    pub entry: Option<usize>,
    pub total: usize,
}

impl fmt::Display for ListRefusal {
    /// The entry is written 1-based: `(entry 1 of 3)` is the first.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.fault, f)?;
        match self.entry {
            Some(index) => write!(f, " (entry {} of {})", index + 1, self.total),
            None => Ok(()),
        }
    }
}

/// The direct conversations of a conversation-list answer, all or nothing:
/// one malformed entry (unknown type, non-canonical id, a direct entry
/// whose id is not this pair's, a repeated resident) refuses the whole
/// list. Group and ad-hoc entries are checked, then left out; the null
/// participant (system traffic) is accepted but has no history of its own,
/// and a conversation with the agent itself is left out (unlike the LL
/// viewer, which refuses the list on it: see `list_entry`).
/// Port of LLChatServiceHistoryCore::validateConversationList.
pub fn validate_conversation_list(value: &Llsd, agent_id: Uuid) -> Option<Vec<ListEntry>> {
    validate_conversation_list_checked(value, agent_id).ok().map(|list| list.entries)
}

/// [`validate_conversation_list`], with the counts of an accepted list and
/// the reason of a refusal. The checks run in the LL viewer's order, so the
/// reason is the first one it would have met, except after a conversation
/// with the agent itself: the LL viewer refuses the list there, this goes
/// on to the next entry.
pub fn validate_conversation_list_checked(value: &Llsd, agent_id: Uuid) -> Result<ConversationList, ListRefusal> {
    let Llsd::Array(list) = value else {
        return Err(ListRefusal {
            fault: ListFault::NotArray(llsd_kind(value)),
            entry: None,
            total: 0,
        });
    };
    let total = list.len();
    if total > MAX_LIST_ENTRIES {
        return Err(ListRefusal {
            fault: ListFault::TooManyEntries(total),
            entry: None,
            total,
        });
    }
    let mut accepted = ConversationList {
        total,
        ..ConversationList::default()
    };
    let mut residents = HashSet::new();
    for (index, item) in list.iter().enumerate() {
        list_entry(item, agent_id, &mut residents, &mut accepted).map_err(|fault| ListRefusal {
            fault,
            entry: Some(index),
            total,
        })?;
    }
    Ok(accepted)
}

/// One entry of the list, counted into `accepted` once it validated.
///
/// A direct entry whose `other_participant_id` is the agent itself (in
/// canonical spelling; any other is a non-canonical refusal) is left out,
/// like the null participant: there is no resident history to fetch.
/// Deliberate difference from validateConversationList, which refuses the
/// whole list on it: one such entry from the service stopped every history
/// fetch of an account (#4499). It is skipped where the LL viewer refuses
/// it: the checks before still apply (type, conversation id, participant
/// id, a string token), those after do not (a time UUID token, the agent's
/// own pair, one entry per resident), since the service's entry is only
/// known to pass the first ones and nothing of it is kept. Every other
/// refusal is unchanged; history pages stay all or nothing.
fn list_entry(item: &Llsd, agent_id: Uuid, residents: &mut HashSet<Uuid>, accepted: &mut ConversationList) -> Result<(), ListFault> {
    if !item.is_map() {
        return Err(ListFault::EntryNotMap(llsd_kind(item)));
    }
    let kind = string_field(item, "conversation_type")?;
    let conversation = string_field(item, "conversation_id")?;
    if conversation.is_empty() {
        return Err(ListFault::EmptyConversationId);
    }
    if let Some(fault) = text_fault(conversation, MAX_LIST_CONVERSATION_ID_BYTES) {
        return Err(ListFault::ConversationId(fault));
    }
    match kind {
        "direct" => {}
        "group" => {
            accepted.group += 1;
            return Ok(());
        }
        "adhoc" => {
            accepted.adhoc += 1;
            return Ok(());
        }
        other => return Err(ListFault::UnknownType(excerpt(other, TYPE_EXCERPT_CHARS))),
    }
    let resident_text = string_field(item, "other_participant_id")?;
    let token = string_field(item, "last_msg_id")?;
    let resident = parse_canonical_uuid(resident_text).ok_or_else(|| ListFault::ParticipantNotCanonical {
        value: excerpt(resident_text, ID_EXCERPT_CHARS),
        fault: canonical_fault(resident_text),
    })?;
    if resident == agent_id {
        accepted.direct += 1;
        accepted.own += 1;
        return Ok(());
    }
    if parse_time_uuid(token).is_none() {
        return Err(ListFault::LastMsgIdNotTimeUuid {
            value: excerpt(token, ID_EXCERPT_CHARS),
            fault: time_uuid_fault(token),
        });
    }
    if conversation != direct_conversation_id(agent_id, resident) {
        return Err(ListFault::ConversationMismatch {
            value: excerpt(conversation, ID_EXCERPT_CHARS),
            resident,
        });
    }
    if !residents.insert(resident) {
        return Err(ListFault::RepeatedResident(resident));
    }
    accepted.direct += 1;
    if !resident.is_nil() {
        accepted.entries.push(ListEntry {
            resident_id: resident,
            last_msg_id: token.to_owned(),
        });
    }
    Ok(())
}

/// What made [`validate_history_page_checked`] refuse a page. A row's
/// `from_name`, `message` and `created_at` are never carried, only their
/// field name and size: the refusal goes to the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageFault {
    /// The answer is not a map (its LLSD type).
    NotMap(&'static str),
    /// A field of the page or of a row is missing or of the wrong type.
    Field(FieldFault),
    /// The page's `conversation_id` (cut and escaped) is not this pair's.
    OtherConversation(String),
    /// The echoed `limit` is an integer other than [`HISTORY_LIMIT`].
    Limit(i32),
    /// More than [`HISTORY_LIMIT`] messages (their number).
    TooManyMessages(usize),
    /// The head page echoes a `before_msg_id` (its LLSD type).
    CursorOnHeadPage(&'static str),
    /// An older page echoes another `before_msg_id` than the one asked.
    CursorMismatch,
    /// The cursor this pass asked with is not a time UUID.
    RequestedCursor(IdFault),
    /// A row is not a map (its LLSD type).
    RowNotMap(&'static str),
    /// A row's `conversation_id` (cut and escaped) is not this pair's.
    RowOtherConversation(String),
    MsgIdNotTimeUuid(IdFault),
    FromIdNotCanonical(IdFault),
    /// The sender is neither the agent nor the resident.
    FromIdNotParticipant(Uuid),
    /// `from_name` or `message` failed [`clean_text`].
    Text {
        field: &'static str,
        fault: TextFault,
    },
    /// Not an IM dialog a direct history keeps.
    DialogNotPersisted(i32),
    CreatedAtInvalid,
    /// The row is not older than the row before it.
    NotNewestFirst,
    /// The row is not older than the requested cursor.
    NotOlderThanCursor,
}

impl From<FieldFault> for PageFault {
    fn from(fault: FieldFault) -> Self {
        Self::Field(fault)
    }
}

impl fmt::Display for PageFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMap(kind) => write!(f, "answer is {kind}, expected map"),
            Self::Field(fault) => fmt::Display::fmt(fault, f),
            Self::OtherConversation(value) => write!(f, "conversation_id \"{value}\" is not this pair's"),
            Self::Limit(limit) => write!(f, "limit {limit}, expected {HISTORY_LIMIT}"),
            Self::TooManyMessages(count) => write!(f, "{count} messages, limit {HISTORY_LIMIT}"),
            Self::CursorOnHeadPage(kind) => write!(f, "before_msg_id ({kind}) echoed on the head page"),
            Self::CursorMismatch => f.write_str("before_msg_id differs from the requested cursor"),
            Self::RequestedCursor(fault) => write!(f, "requested cursor is not a time UUID ({fault})"),
            Self::RowNotMap(kind) => write!(f, "row is {kind}, expected map"),
            Self::RowOtherConversation(value) => write!(f, "row conversation_id \"{value}\" is not this pair's"),
            Self::MsgIdNotTimeUuid(fault) => write!(f, "msg_id is not a time UUID ({fault})"),
            Self::FromIdNotCanonical(fault) => write!(f, "from_id is not a canonical UUID ({fault})"),
            Self::FromIdNotParticipant(id) => write!(f, "from_id {id} is neither participant"),
            Self::Text { field, fault } => write!(f, "{field} {fault}"),
            Self::DialogNotPersisted(dialog) => write!(f, "dialog {dialog} is not kept in a direct history"),
            Self::CreatedAtInvalid => f.write_str("created_at is not a valid ISO 8601 time"),
            Self::NotNewestFirst => f.write_str("msg_id not older than the previous row's (rows not strictly newest first)"),
            Self::NotOlderThanCursor => f.write_str("msg_id not older than the requested cursor"),
        }
    }
}

/// Why a history page was refused, for the log: the first fault found and,
/// for a row, its 0-based index in a page of `total` rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRefusal {
    pub fault: PageFault,
    pub row: Option<usize>,
    pub total: usize,
}

impl PageRefusal {
    fn page(fault: PageFault) -> Self {
        Self {
            fault,
            row: None,
            total: 0,
        }
    }
}

impl fmt::Display for PageRefusal {
    /// The row is written 1-based: `(row 1 of 3)` is the newest.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.fault, f)?;
        match self.row {
            Some(index) => write!(f, " (row {} of {})", index + 1, self.total),
            None => Ok(()),
        }
    }
}

/// One message of a history page, every field checked (the per-row half
/// of validateHistoryPage).
fn page_row(item: &Llsd, conversation_id: &str, agent_id: Uuid, resident_id: Uuid) -> Result<HistoryRow, PageFault> {
    if !item.is_map() {
        return Err(PageFault::RowNotMap(llsd_kind(item)));
    }
    let row_conversation = string_field(item, "conversation_id")?;
    let msg_id = string_field(item, "msg_id")?;
    let from_text = string_field(item, "from_id")?;
    let from_name = string_field(item, "from_name")?;
    let message = string_field(item, "message")?;
    let dialog = match item.get("dialog") {
        Llsd::Integer(dialog) => *dialog,
        other => {
            return Err(PageFault::Field(FieldFault {
                field: "dialog",
                expected: "integer",
                found: llsd_kind(other),
            }));
        }
    };
    let created_at = string_field(item, "created_at")?;
    if row_conversation != conversation_id {
        return Err(PageFault::RowOtherConversation(excerpt(row_conversation, ID_EXCERPT_CHARS)));
    }
    let key = parse_time_uuid(msg_id).ok_or_else(|| PageFault::MsgIdNotTimeUuid(time_uuid_fault(msg_id)))?;
    let from_id = parse_canonical_uuid(from_text).ok_or_else(|| PageFault::FromIdNotCanonical(canonical_fault(from_text)))?;
    if from_id != agent_id && from_id != resident_id {
        return Err(PageFault::FromIdNotParticipant(from_id));
    }
    for (field, text, limit) in [
        ("from_name", from_name, MAX_FROM_NAME_BYTES),
        ("message", message, MAX_MESSAGE_BYTES),
    ] {
        if let Some(fault) = text_fault(text, limit) {
            return Err(PageFault::Text { field, fault });
        }
    }
    if !persisted_direct_dialog(dialog) {
        return Err(PageFault::DialogNotPersisted(dialog));
    }
    let created_at = parse_created_at(created_at).ok_or(PageFault::CreatedAtInvalid)?;
    Ok(HistoryRow {
        conversation_id: row_conversation.to_owned(),
        msg_id: msg_id.to_owned(),
        key,
        from_id,
        from_name: from_name.to_owned(),
        message: message.to_owned(),
        dialog,
        created_at,
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
    validate_history_page_checked(value, agent_id, resident_id, requested_cursor, deleted_before_ticks).ok()
}

/// [`validate_history_page`], with the reason of a refusal. The checks run
/// in the LL viewer's order, so the reason is the first one it would have
/// met.
pub fn validate_history_page_checked(
    value: &Llsd,
    agent_id: Uuid,
    resident_id: Uuid,
    requested_cursor: &str,
    deleted_before_ticks: u64,
) -> Result<Page, PageRefusal> {
    let conversation_id = direct_conversation_id(agent_id, resident_id);
    if !value.is_map() {
        return Err(PageRefusal::page(PageFault::NotMap(llsd_kind(value))));
    }
    let echoed_conversation = string_field(value, "conversation_id").map_err(|fault| PageRefusal::page(fault.into()))?;
    if echoed_conversation != conversation_id {
        return Err(PageRefusal::page(PageFault::OtherConversation(excerpt(
            echoed_conversation,
            ID_EXCERPT_CHARS,
        ))));
    }
    match value.get("limit") {
        Llsd::Integer(HISTORY_LIMIT) => {}
        Llsd::Integer(limit) => return Err(PageRefusal::page(PageFault::Limit(*limit))),
        other => {
            return Err(PageRefusal::page(PageFault::Field(FieldFault {
                field: "limit",
                expected: "integer",
                found: llsd_kind(other),
            })));
        }
    }
    let messages = match value.get("messages") {
        Llsd::Array(messages) => messages,
        other => {
            return Err(PageRefusal::page(PageFault::Field(FieldFault {
                field: "messages",
                expected: "array",
                found: llsd_kind(other),
            })));
        }
    };
    let total = messages.len();
    if total > HISTORY_LIMIT as usize {
        return Err(PageRefusal::page(PageFault::TooManyMessages(total)));
    }
    let echoed = value.get("before_msg_id");
    let cursor_key = if requested_cursor.is_empty() {
        if !echoed.is_undef() {
            return Err(PageRefusal::page(PageFault::CursorOnHeadPage(llsd_kind(echoed))));
        }
        None
    } else {
        let echoed_cursor = string_field(value, "before_msg_id").map_err(|fault| PageRefusal::page(fault.into()))?;
        if echoed_cursor != requested_cursor {
            return Err(PageRefusal::page(PageFault::CursorMismatch));
        }
        let key = parse_time_uuid(requested_cursor)
            .ok_or_else(|| PageRefusal::page(PageFault::RequestedCursor(time_uuid_fault(requested_cursor))))?;
        Some(key)
    };

    let mut page = Page::default();
    let mut previous: Option<TimeUuidKey> = None;
    for (index, item) in messages.iter().enumerate() {
        let refusal = |fault| PageRefusal {
            fault,
            row: Some(index),
            total,
        };
        let row = page_row(item, &conversation_id, agent_id, resident_id).map_err(refusal)?;
        if previous.is_some_and(|p| row.key >= p) {
            return Err(refusal(PageFault::NotNewestFirst));
        }
        if cursor_key.is_some_and(|c| row.key >= c) {
            return Err(refusal(PageFault::NotOlderThanCursor));
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
    Ok(page)
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

    fn without(mut value: Llsd, key: &str) -> Llsd {
        if let Llsd::Map(map) = &mut value {
            map.remove(key);
        }
        value
    }

    fn list_refusal(list: Vec<Llsd>) -> ListRefusal {
        let refusal = validate_conversation_list_checked(&Llsd::Array(list.clone()), AGENT).expect_err("list refused");
        assert!(
            validate_conversation_list(&Llsd::Array(list), AGENT).is_none(),
            "the wrapper refuses it too"
        );
        refusal
    }

    fn page_refusal(page: &Llsd, cursor: &str) -> PageRefusal {
        let refusal = validate_history_page_checked(page, AGENT, RESIDENT, cursor, 0).expect_err("page refused");
        assert!(
            validate_history_page(page, AGENT, RESIDENT, cursor, 0).is_none(),
            "the wrapper refuses it too"
        );
        refusal
    }

    #[test]
    fn list_refusal_names_the_answer_fault() {
        let refusal = validate_conversation_list_checked(&Llsd::new_map(), AGENT).expect_err("a map is not a list");
        assert_eq!((refusal.fault.clone(), refusal.entry), (ListFault::NotArray("map"), None));
        assert_eq!(refusal.to_string(), "answer is map, expected array");

        let refusal = list_refusal(vec![Llsd::Undef; MAX_LIST_ENTRIES + 1]);
        assert_eq!(refusal.fault, ListFault::TooManyEntries(MAX_LIST_ENTRIES + 1));
        assert_eq!(refusal.entry, None, "counted before any entry is read");
    }

    #[test]
    fn list_refusal_names_the_entry_and_its_field() {
        let refusal = list_refusal(vec![list_entry(RESIDENT), Llsd::from("x")]);
        assert_eq!((refusal.entry, refusal.total), (Some(1), 2));
        assert_eq!(refusal.to_string(), "entry is string, expected map (entry 2 of 2)");

        let refusal = list_refusal(vec![without(list_entry(RESIDENT), "last_msg_id")]);
        assert_eq!(refusal.to_string(), "last_msg_id missing (entry 1 of 1)");

        // A UUID-typed id is not coerced: the type is named.
        let mut typed = list_entry(RESIDENT);
        typed.insert("other_participant_id", RESIDENT);
        let refusal = list_refusal(vec![typed]);
        assert_eq!(
            refusal.fault,
            ListFault::Field(FieldFault {
                field: "other_participant_id",
                expected: "string",
                found: "uuid",
            })
        );
        assert_eq!(refusal.to_string(), "other_participant_id is uuid, expected string (entry 1 of 1)");

        let mut empty = list_entry(RESIDENT);
        empty.insert("conversation_id", "");
        assert_eq!(list_refusal(vec![empty]).fault, ListFault::EmptyConversationId);
        let mut long = list_entry(RESIDENT);
        long.insert("conversation_id", "x".repeat(MAX_LIST_CONVERSATION_ID_BYTES + 1));
        assert_eq!(
            list_refusal(vec![long]).fault,
            ListFault::ConversationId(TextFault::TooLong {
                bytes: MAX_LIST_CONVERSATION_ID_BYTES + 1,
                limit: MAX_LIST_CONVERSATION_ID_BYTES,
            })
        );
    }

    #[test]
    fn list_refusal_cuts_and_escapes_an_unknown_type() {
        let mut entry = list_entry(RESIDENT);
        entry.insert("conversation_type", format!("Direct\n\"{}", "z".repeat(40)));
        let refusal = list_refusal(vec![list_entry(Uuid::nil()), entry]);
        let ListFault::UnknownType(value) = &refusal.fault else {
            panic!("unknown type expected, got {:?}", refusal.fault);
        };
        assert_eq!(*value, format!("Direct\\u{{a}}\\\"{}...", "z".repeat(24)), "32 characters, escaped");
        assert!(refusal.to_string().ends_with("...\" (entry 2 of 2)"), "{refusal}");
    }

    #[test]
    fn list_refusal_names_each_direct_entry_fault() {
        let hex = Uuid::from_u128(0xabc);
        let mut upper = list_entry(hex);
        upper.insert("other_participant_id", hex.to_string().to_uppercase());
        assert!(matches!(
            list_refusal(vec![upper]).fault,
            ListFault::ParticipantNotCanonical {
                fault: IdFault::NotCanonical,
                ..
            }
        ));
        let mut short = list_entry(RESIDENT);
        short.insert("other_participant_id", "1234");
        assert!(matches!(
            list_refusal(vec![short]).fault,
            ListFault::ParticipantNotCanonical {
                fault: IdFault::Length(4),
                ..
            }
        ));

        let mut v4 = list_entry(RESIDENT);
        v4.insert("last_msg_id", "00000001-0000-4000-8000-000000000000");
        assert_eq!(
            list_refusal(vec![v4]).fault,
            ListFault::LastMsgIdNotTimeUuid {
                value: "00000001-0000-4000-8000-000000000000".into(),
                fault: IdFault::Version(4),
            }
        );
        let mut variant = list_entry(RESIDENT);
        variant.insert("last_msg_id", "00000001-0000-1000-c000-000000000000");
        assert!(matches!(
            list_refusal(vec![variant]).fault,
            ListFault::LastMsgIdNotTimeUuid {
                fault: IdFault::Variant,
                ..
            }
        ));

        let mut other_pair = list_entry(RESIDENT);
        other_pair.insert("conversation_id", direct_conversation_id(AGENT, Uuid::from_u128(3)));
        assert_eq!(
            list_refusal(vec![other_pair]).fault,
            ListFault::ConversationMismatch {
                value: direct_conversation_id(AGENT, Uuid::from_u128(3)),
                resident: RESIDENT,
            }
        );

        let refusal = list_refusal(vec![list_entry(RESIDENT), list_entry(Uuid::nil()), list_entry(RESIDENT)]);
        assert_eq!(
            (refusal.fault.clone(), refusal.entry),
            (ListFault::RepeatedResident(RESIDENT), Some(2))
        );
        assert_eq!(refusal.to_string(), format!("resident {RESIDENT} listed twice (entry 3 of 3)"));
    }

    #[test]
    fn accepted_list_counts_every_type() {
        let mut group = Llsd::new_map();
        group.insert("conversation_type", "group");
        group.insert("conversation_id", Uuid::from_u128(7).to_string());
        let mut adhoc = group.clone();
        adhoc.insert("conversation_type", "adhoc");
        let value = Llsd::Array(vec![list_entry(RESIDENT), group.clone(), list_entry(Uuid::nil()), adhoc, group]);
        let list = validate_conversation_list_checked(&value, AGENT).expect("valid list");
        assert_eq!((list.total, list.direct, list.group, list.adhoc, list.own), (5, 2, 2, 1, 0));
        assert_eq!(
            list.entries,
            validate_conversation_list(&value, AGENT).expect("the wrapper accepts it")
        );
        assert_eq!(list.entries.len(), 1, "the null participant has no history");
    }

    /// The direct conversation with the account itself that the service
    /// listed for one account (#4499) is left out wherever it comes: it no
    /// longer refuses the whole list.
    #[test]
    fn a_self_entry_is_left_out_wherever_it_is_listed() {
        let mut group = Llsd::new_map();
        group.insert("conversation_type", "group");
        group.insert("conversation_id", Uuid::from_u128(7).to_string());
        let others = [list_entry(RESIDENT), list_entry(Uuid::nil()), group];
        for at in 0..=others.len() {
            let mut entries = others.to_vec();
            entries.insert(at, list_entry(AGENT));
            let list = validate_conversation_list_checked(&Llsd::Array(entries), AGENT).expect("a self entry does not refuse the list");
            assert_eq!(
                (list.total, list.direct, list.group, list.adhoc, list.own),
                (4, 3, 1, 0, 1),
                "self entry at {at}"
            );
            let kept = [ListEntry {
                resident_id: RESIDENT,
                last_msg_id: SECOND.into(),
            }];
            assert_eq!(list.entries, kept, "only the resident is kept, self entry at {at}");
        }
    }

    #[test]
    fn a_list_of_only_a_self_entry_has_no_entries() {
        let value = Llsd::Array(vec![list_entry(AGENT)]);
        let list = validate_conversation_list_checked(&value, AGENT).expect("a self entry alone is accepted");
        assert!(list.entries.is_empty(), "nothing to fetch");
        assert_eq!((list.total, list.direct, list.own), (1, 1, 1));
        assert_eq!(validate_conversation_list(&value, AGENT), Some(Vec::new()), "the wrapper agrees");
    }

    /// A self entry keeps only the checks the LL viewer makes before it
    /// refuses one: the service's entry is not known to pass the later
    /// ones, and nothing of it is kept.
    #[test]
    fn a_self_entry_needs_no_time_uuid_nor_its_own_pair_and_may_repeat() {
        let mut other_pair = list_entry(AGENT);
        other_pair.insert("conversation_id", direct_conversation_id(AGENT, RESIDENT));
        let mut free_id = list_entry(AGENT);
        free_id.insert("conversation_id", "a-conversation-with-oneself");
        let mut v4 = list_entry(AGENT);
        v4.insert("last_msg_id", "00000001-0000-4000-8000-000000000000");
        let mut garbage = list_entry(AGENT);
        garbage.insert("last_msg_id", "not-an-id");
        let mut empty = list_entry(AGENT);
        empty.insert("last_msg_id", "");
        for (case, entry) in [other_pair, free_id, v4, garbage, empty].into_iter().enumerate() {
            let value = Llsd::Array(vec![entry, list_entry(RESIDENT)]);
            let list = validate_conversation_list_checked(&value, AGENT).unwrap_or_else(|refusal| panic!("case {case}: {refusal}"));
            assert_eq!((list.own, list.entries.len()), (1, 1), "case {case}");
        }

        let value = Llsd::Array(vec![list_entry(AGENT), list_entry(RESIDENT), list_entry(AGENT)]);
        let list = validate_conversation_list_checked(&value, AGENT).expect("a second self entry is skipped too");
        assert_eq!((list.direct, list.own, list.entries.len()), (3, 2, 1));
    }

    #[test]
    fn a_self_entry_keeps_the_shape_checks() {
        let mut empty = list_entry(AGENT);
        empty.insert("conversation_id", "");
        assert_eq!(list_refusal(vec![empty]).fault, ListFault::EmptyConversationId);
        let mut nul = list_entry(AGENT);
        nul.insert("conversation_id", "a\0b");
        assert_eq!(list_refusal(vec![nul]).fault, ListFault::ConversationId(TextFault::Nul));
        let mut kind = list_entry(AGENT);
        kind.insert("conversation_type", "Direct");
        assert_eq!(list_refusal(vec![kind]).fault, ListFault::UnknownType("Direct".into()));
        let mut token = list_entry(AGENT);
        token.insert("last_msg_id", 5);
        assert_eq!(
            list_refusal(vec![token]).to_string(),
            "last_msg_id is integer, expected string (entry 1 of 1)"
        );
        assert_eq!(
            list_refusal(vec![without(list_entry(AGENT), "last_msg_id")]).to_string(),
            "last_msg_id missing (entry 1 of 1)"
        );

        // Only the canonical spelling is the agent: another one is refused
        // as for any resident.
        let mut typed = list_entry(AGENT);
        typed.insert("other_participant_id", AGENT);
        assert_eq!(
            list_refusal(vec![typed]).to_string(),
            "other_participant_id is uuid, expected string (entry 1 of 1)"
        );
        let mut braced = list_entry(AGENT);
        braced.insert("other_participant_id", format!("{{{AGENT}}}"));
        assert!(matches!(
            list_refusal(vec![braced]).fault,
            ListFault::ParticipantNotCanonical {
                fault: IdFault::Length(38),
                ..
            }
        ));
    }

    #[test]
    fn a_malformed_entry_after_a_self_entry_still_refuses_the_list() {
        let mut v4 = list_entry(RESIDENT);
        v4.insert("last_msg_id", "00000001-0000-4000-8000-000000000000");
        let refusal = list_refusal(vec![list_entry(AGENT), v4]);
        assert!(matches!(
            refusal.fault,
            ListFault::LastMsgIdNotTimeUuid {
                fault: IdFault::Version(4),
                ..
            }
        ));
        assert_eq!((refusal.entry, refusal.total), (Some(1), 2), "the later entry is named");
    }

    #[test]
    fn page_refusal_names_the_page_fault() {
        let refusal = page_refusal(&Llsd::Undef, "");
        assert_eq!((refusal.fault.clone(), refusal.row), (PageFault::NotMap("undef"), None));
        assert_eq!(refusal.to_string(), "answer is undef, expected map");

        let refusal = page_refusal(&without(wire_page(Vec::new(), ""), "conversation_id"), "");
        assert_eq!(refusal.to_string(), "conversation_id missing");
        let mut other = wire_page(Vec::new(), "");
        other.insert("conversation_id", direct_conversation_id(AGENT, Uuid::nil()));
        assert_eq!(
            page_refusal(&other, "").fault,
            PageFault::OtherConversation(direct_conversation_id(AGENT, Uuid::nil()))
        );

        let mut limit = wire_page(Vec::new(), "");
        limit.insert("limit", 50);
        assert_eq!(page_refusal(&limit, "").to_string(), "limit 50, expected 100");
        limit.insert("limit", "100");
        assert_eq!(page_refusal(&limit, "").to_string(), "limit is string, expected integer");

        let page = without(wire_page(Vec::new(), ""), "messages");
        assert_eq!(page_refusal(&page, "").to_string(), "messages missing");
        let page = wire_page(vec![wire_row(FIRST, RESIDENT); HISTORY_LIMIT as usize + 1], "");
        assert_eq!(
            page_refusal(&page, "").fault,
            PageFault::TooManyMessages(HISTORY_LIMIT as usize + 1)
        );

        assert_eq!(
            page_refusal(&wire_page(Vec::new(), FIRST), "").fault,
            PageFault::CursorOnHeadPage("string")
        );
        assert_eq!(
            page_refusal(&wire_page(Vec::new(), ""), SECOND).to_string(),
            "before_msg_id missing"
        );
        assert_eq!(page_refusal(&wire_page(Vec::new(), FIRST), SECOND).fault, PageFault::CursorMismatch);
        let v4 = "00000001-0000-4000-8000-000000000000";
        assert_eq!(
            page_refusal(&wire_page(Vec::new(), v4), v4).fault,
            PageFault::RequestedCursor(IdFault::Version(4))
        );
    }

    #[test]
    fn page_refusal_names_the_row_and_its_fault() {
        let refusal = page_refusal(&wire_page(vec![wire_row(SECOND, RESIDENT), Llsd::from(1)], ""), "");
        assert_eq!((refusal.row, refusal.total), (Some(1), 2));
        assert_eq!(refusal.to_string(), "row is integer, expected map (row 2 of 2)");

        let mut dialog = wire_row(FIRST, RESIDENT);
        dialog.insert("dialog", "0");
        assert_eq!(
            page_refusal(&wire_page(vec![dialog], ""), "").to_string(),
            "dialog is string, expected integer (row 1 of 1)"
        );
        let mut dialog = wire_row(FIRST, RESIDENT);
        dialog.insert("dialog", 41);
        assert_eq!(
            page_refusal(&wire_page(vec![dialog], ""), "").fault,
            PageFault::DialogNotPersisted(41)
        );

        let mut conversation = wire_row(FIRST, RESIDENT);
        conversation.insert("conversation_id", direct_conversation_id(AGENT, Uuid::nil()));
        assert!(matches!(
            page_refusal(&wire_page(vec![conversation], ""), "").fault,
            PageFault::RowOtherConversation(_)
        ));
        let v4 = wire_row("00000001-0000-4000-8000-000000000000", RESIDENT);
        assert_eq!(
            page_refusal(&wire_page(vec![v4], ""), "").fault,
            PageFault::MsgIdNotTimeUuid(IdFault::Version(4))
        );
        let stranger = Uuid::from_u128(3);
        assert_eq!(
            page_refusal(&wire_page(vec![wire_row(FIRST, stranger)], ""), "").fault,
            PageFault::FromIdNotParticipant(stranger)
        );
        let mut from = wire_row(FIRST, RESIDENT);
        from.insert("from_id", "{00000000-0000-0000-0000-000000000002}");
        assert_eq!(
            page_refusal(&wire_page(vec![from], ""), "").fault,
            PageFault::FromIdNotCanonical(IdFault::Length(38))
        );
        let mut created = wire_row(FIRST, RESIDENT);
        created.insert("created_at", "2026-02-29T12:00:00Z");
        assert_eq!(page_refusal(&wire_page(vec![created], ""), "").fault, PageFault::CreatedAtInvalid);
    }

    #[test]
    fn page_refusal_names_the_key_order() {
        let refusal = page_refusal(&wire_page(vec![wire_row(FIRST, RESIDENT), wire_row(SECOND, RESIDENT)], ""), "");
        assert_eq!((refusal.fault.clone(), refusal.row), (PageFault::NotNewestFirst, Some(1)));
        let refusal = page_refusal(&wire_page(vec![wire_row(SECOND, RESIDENT)], FIRST), FIRST);
        assert_eq!((refusal.fault.clone(), refusal.row), (PageFault::NotOlderThanCursor, Some(0)));
        assert_eq!(refusal.to_string(), "msg_id not older than the requested cursor (row 1 of 1)");
    }

    #[test]
    fn page_refusal_never_carries_the_row_text() {
        let mut long = wire_row(FIRST, RESIDENT);
        long.insert("message", "SECRET-TEXT ".repeat(100));
        let mut nul = wire_row(FIRST, RESIDENT);
        nul.insert("from_name", "Secret\0Name");
        let mut created = wire_row(FIRST, RESIDENT);
        created.insert("message", "SECRET-TEXT");
        created.insert("created_at", "SECRET-TIME");
        for (row, expected) in [
            (long, "message too long (1200 bytes, limit 1024) (row 1 of 1)"),
            (nul, "from_name contains NUL (row 1 of 1)"),
            (created, "created_at is not a valid ISO 8601 time (row 1 of 1)"),
        ] {
            let refusal = page_refusal(&wire_page(vec![row], ""), "");
            let text = format!("{refusal} {refusal:?}");
            assert!(!text.contains("SECRET") && !text.contains("Secret"), "{text}");
            assert_eq!(refusal.to_string(), expected);
        }
    }

    #[test]
    fn text_fault_agrees_with_clean_text() {
        for (value, limit) in [("", 0), ("abc", 3), ("abcd", 3), ("a\0b", 3), ("a\0bcd", 3)] {
            assert_eq!(text_fault(value, limit).is_none(), clean_text(value, limit), "{value:?}");
        }
    }

    #[test]
    fn staged_bytes_follow_the_pass_estimate() {
        let page = validate_history_page(&wire_page(vec![wire_row(FIRST, AGENT)], ""), AGENT, RESIDENT, "", 0).expect("valid");
        let row = &page.rows[0];
        let expected = row.conversation_id.len() + 36 + 36 + "Resident".len() + "hello".len() + 1 + row.created_at.len();
        assert_eq!(row.staged_bytes(), expected);
    }
}
