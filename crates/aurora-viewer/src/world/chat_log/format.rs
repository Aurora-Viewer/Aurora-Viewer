//! Text format of the conversation transcripts, the one Firestorm writes and
//! reads: port of LLChatLogFormatter::format, LLChatLogParser::parse and
//! LLLogChat::timestamp2LogString (indra/newview/lllogchat.cpp, originally
//! LGPL 2.1).
//!
//! One message per line, `[2026/10/12 14:05]  Name: text`. The timestamp is
//! Second Life time (US Pacific): Firestorm builds it from the "slt" date
//! codes (`TimeYear` = "year,datetime,slt"...), with LogTimestamp and
//! LogTimestampDate on and Use24HourClock on by default. The lines after the
//! first of a multi-line message start with one space. A ':' in the name is
//! written "%3A" so that the first ':' of a line always ends the name.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Sender written for system lines (SYSTEM_FROM): saveHistory names them so
/// that the parser never takes the start of their text for a name.
pub const SYSTEM_FROM: &str = "Second Life";

/// A message read back from a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The timestamp as written, without its brackets ("2026/10/12 14:05");
    /// empty when the line had none.
    pub stamp: String,
    /// When the stamp gives a date: that moment.
    pub time: Option<SystemTime>,
    /// Sender; `SYSTEM_FROM` for a system line or a line without a name.
    pub from: String,
    pub text: String,
}

impl Entry {
    pub fn is_system(&self) -> bool {
        self.from == SYSTEM_FROM
    }
}

/// Hours to add to UTC for US Pacific time: -7 from the second Sunday of
/// March 2:00 PST to the first Sunday of November 2:00 PDT, else -8 (the
/// rule since 2007).
pub fn pacific_offset_hours(unix: i64) -> i64 {
    let (year, ..) = civil(unix.div_euclid(86400));
    // 2:00 PST = 10:00 UTC, 2:00 PDT = 9:00 UTC
    let start = nth_sunday(year, 3, 2) * 86400 + 10 * 3600;
    let end = nth_sunday(year, 11, 1) * 86400 + 9 * 3600;
    if (start..end).contains(&unix) { -7 } else { -8 }
}

/// Days since the epoch of the `n`th Sunday of a month.
fn nth_sunday(year: i64, month: i64, n: i64) -> i64 {
    let first = days_from_civil(year, month, 1);
    // 1970-01-01 was a Thursday: (days + 4) % 7 == 0 on Sundays
    let to_sunday = (7 - (first + 4).rem_euclid(7)) % 7;
    first + to_sunday + (n - 1) * 7
}

/// Days since 1970-01-01 of a civil date (H. Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (year, month, day) of a number of days since 1970-01-01.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A moment in Second Life time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slt {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
}

impl Slt {
    pub fn of(time: SystemTime) -> Slt {
        Slt::from_unix(unix_of(time))
    }

    pub fn from_unix(unix: i64) -> Slt {
        let t = unix + pacific_offset_hours(unix) * 3600;
        let (year, month, day) = civil(t.div_euclid(86400));
        let tod = t.rem_euclid(86400);
        Slt {
            year,
            month,
            day,
            hour: tod / 3600,
            minute: (tod / 60) % 60,
        }
    }

    /// The same calendar day.
    pub fn same_day(&self, other: &Slt) -> bool {
        (self.year, self.month, self.day) == (other.year, other.month, other.day)
    }

    /// The moment this Pacific wall-clock time names (seconds added). An
    /// hour that happens twice at the end of daylight saving is taken as
    /// its first occurrence.
    fn to_unix(self, second: i64) -> i64 {
        let local = days_from_civil(self.year, self.month, self.day) * 86400 + self.hour * 3600 + self.minute * 60 + second;
        for off in [-7, -8] {
            let unix = local - off * 3600;
            if pacific_offset_hours(unix) == off {
                return unix;
            }
        }
        // the hour skipped at the start of daylight saving
        local + 8 * 3600
    }
}

fn unix_of(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// LLLogChat::timestamp2LogString with a 24 hour clock: "2026/10/12 14:05",
/// or "14:05" without the date (LogTimestampDate off).
pub fn stamp(time: SystemTime, with_date: bool) -> String {
    let t = Slt::of(time);
    if with_date {
        format!("{:04}/{:02}/{:02} {:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute)
    } else {
        format!("{:02}:{:02}", t.hour, t.minute)
    }
}

/// LLChatLogFormatter::format: one message as its transcript text, without
/// the end of line. An empty `from` writes no name (Firestorm's saveHistory
/// always gives one: `SYSTEM_FROM` for system lines).
pub fn format_line(stamp: Option<&str>, from: &str, text: &str) -> String {
    let mut out = String::with_capacity(text.len() + from.len() + 24);
    if let Some(stamp) = stamp {
        out.push('[');
        out.push_str(stamp.trim());
        out.push_str("]  ");
    }
    let from = from.trim();
    if !from.is_empty() {
        // LLURI::encodeCharacter(':')
        out.push_str(&from.replace(':', "%3A"));
        out.push_str(": ");
    }
    // the next lines of a message start with a space; a "\r\n" inside a
    // message would otherwise leave a bare '\r' before it
    let mut first = true;
    for part in text.split('\n') {
        if !first {
            out.push_str("\n ");
        }
        first = false;
        out.push_str(part.strip_suffix('\r').unwrap_or(part));
    }
    out
}

/// The text of a transcript line that continues the previous message, when
/// it is one (updated 1.23 plain text log format: a leading space).
pub fn continuation(line: &str) -> Option<&str> {
    line.strip_prefix(' ')
}

/// LLChatLogParser::parse: timestamp, name and text of the first line of a
/// message. None for an empty line.
pub fn parse_line(raw: &str) -> Option<Entry> {
    if raw.is_empty() {
        return None;
    }
    let (stamp, stuff) = split_stamp(raw);
    let stamp = stamp.unwrap_or_default();
    let time = parse_stamp(stamp);
    let entry = |from: String, text: &str| Entry {
        stamp: stamp.to_owned(),
        time,
        from,
        text: text.to_owned(),
    };
    // NAME_AND_TEXT "([^:]+[:]{1})?(\s*)(.*)": the name is what comes before
    // the first ':' (at least one character)
    let name_end = stuff.find(':').filter(|&i| i > 0);
    let Some(name_end) = name_end else {
        // no name: maybe a complex one cut by ": " further on, else a
        // message from Second Life
        return Some(match stuff.find(": ").filter(|&i| i + 2 < stuff.len()) {
            Some(i) => entry(unescape(&stuff[..i]), &stuff[i + 2..]),
            None => entry(SYSTEM_FROM.to_owned(), stuff),
        });
    };
    let name = unescape(&stuff[..name_end]);
    let rest = &stuff[name_end + 1..];
    // <FS:Ansariel> an IM stored in the nearby chat history ("IM: name: text")
    if name == "IM"
        && let Some(i) = stuff.get(3..).and_then(|s| s.find(": ")).map(|i| i + 3)
        && i + 2 < stuff.len()
    {
        return Some(entry(stuff[..i].to_owned(), &stuff[i + 2..]));
    }
    Some(entry(name, rest.trim_start()))
}

/// The leading `[timestamp]` of a line (without its brackets) and the rest.
/// TIMESTAMP_AND_STUFF: a timestamp is a date and a time or a time alone,
/// with optional seconds and AM/PM, followed by at least one blank.
fn split_stamp(raw: &str) -> (Option<&str>, &str) {
    let Some(inner_end) = raw.strip_prefix('[').and_then(|r| r.find(']')) else {
        return (None, raw);
    };
    let inner = &raw[1..=inner_end];
    let after = &raw[inner_end + 2..];
    let rest = after.trim_start();
    if rest.len() == after.len() || !is_stamp(inner) {
        return (None, raw);
    }
    (Some(inner), rest)
}

/// "2009/11/20 3:00", "3:00", "3:00:59 PM"...
fn is_stamp(s: &str) -> bool {
    stamp_fields(s).is_some()
}

/// (date, hour, minute, second, pm) of a timestamp.
#[allow(clippy::type_complexity)]
fn stamp_fields(s: &str) -> Option<(Option<(i64, i64, i64)>, i64, i64, i64, Option<bool>)> {
    let digits = |t: &str, min: usize, max: usize| -> Option<i64> {
        ((min..=max).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit()))
            .then(|| t.parse().ok())
            .flatten()
    };
    let mut parts = s.split_ascii_whitespace();
    let mut part = parts.next()?;
    let mut date = None;
    if part.contains('/') {
        let mut d = part.split('/');
        let (y, m, day) = (digits(d.next()?, 4, 4)?, digits(d.next()?, 1, 2)?, digits(d.next()?, 1, 2)?);
        if d.next().is_some() {
            return None;
        }
        date = Some((y, m, day));
        part = parts.next()?;
    }
    let mut t = part.split(':');
    let (hour, minute) = (digits(t.next()?, 1, 2)?, digits(t.next()?, 2, 2)?);
    let second = match t.next() {
        Some(sec) => digits(sec, 2, 2)?,
        None => 0,
    };
    if t.next().is_some() {
        return None;
    }
    let pm = match parts.next() {
        None => None,
        Some(x) if x.eq_ignore_ascii_case("am") => Some(false),
        Some(x) if x.eq_ignore_ascii_case("pm") => Some(true),
        Some(_) => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((date, hour, minute, second, pm))
}

/// The moment a timestamp names, when it carries a date (Second Life time).
pub fn parse_stamp(stamp: &str) -> Option<SystemTime> {
    let (date, mut hour, minute, second, pm) = stamp_fields(stamp)?;
    let (year, month, day) = date?;
    if let Some(pm) = pm {
        hour = hour % 12 + if pm { 12 } else { 0 };
    }
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let unix = Slt {
        year,
        month,
        day,
        hour,
        minute,
    }
    .to_unix(second);
    Some(UNIX_EPOCH + Duration::from_secs(unix.max(0) as u64))
}

const MENTION_PREFIX: &str = "secondlife:///app/agent/";
const MENTION_SUFFIX: &str = "/mention";

/// The mention links of a message as LLLogChat::saveHistory writes them:
/// `[@name](secondlife:///app/agent/<id>/mention)`, so the transcript reads
/// without the viewer.
pub fn encode_mentions(text: &str, mut name_of: impl FnMut(uuid::Uuid) -> String) -> String {
    if !text.contains(MENTION_SUFFIX) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + 32);
    let mut rest = text;
    while let Some(i) = rest.find(MENTION_PREFIX) {
        let link = &rest[i..];
        let id = link[MENTION_PREFIX.len()..].get(..36).and_then(|u| uuid::Uuid::parse_str(u).ok());
        let end = MENTION_PREFIX.len() + 36 + MENTION_SUFFIX.len();
        match id {
            Some(id) if link[MENTION_PREFIX.len() + 36..].starts_with(MENTION_SUFFIX) => {
                out.push_str(&rest[..i]);
                // a ']' in the name would end the label early
                let name = name_of(id).replace(']', ")");
                out.push_str(&format!("[@{name}]({})", &link[..end]));
                rest = &link[end..];
            }
            _ => {
                out.push_str(&rest[..i + MENTION_PREFIX.len()]);
                rest = &link[MENTION_PREFIX.len()..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The reverse, as LLLogChat::loadChatHistory does: `[@name](link)` back to
/// the link.
pub fn decode_mentions(text: &str) -> String {
    if !text.contains("/mention)") {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("[@") {
        let tail = &rest[i..];
        let link = tail.find("](").and_then(|close| {
            let url = &tail[close + 2..];
            let end = MENTION_PREFIX.len() + 36 + MENTION_SUFFIX.len();
            let is_mention = url.starts_with(MENTION_PREFIX)
                && url
                    .get(MENTION_PREFIX.len()..MENTION_PREFIX.len() + 36)
                    .is_some_and(|u| uuid::Uuid::parse_str(u).is_ok())
                && url.get(MENTION_PREFIX.len() + 36..end + 1) == Some("/mention)");
            // the label is one bracket pair: no ']' before its end
            (is_mention && !tail[2..close].is_empty() && !tail[2..close].contains(']')).then(|| (&url[..end], close + 2 + end + 1))
        });
        match link {
            Some((url, len)) => {
                out.push_str(&rest[..i]);
                out.push_str(url);
                rest = &tail[len..];
            }
            None => {
                out.push_str(&rest[..i + 2]);
                rest = &tail[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// LLURI::unescape: "%XX" back to its byte (invalid UTF-8 replaced).
fn unescape(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(unix: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(unix)
    }

    #[test]
    fn pacific_time_follows_the_us_rule() {
        // 2026: daylight saving from March 8 to November 1
        let mar8_10utc = days_from_civil(2026, 3, 8) * 86400 + 10 * 3600;
        assert_eq!(pacific_offset_hours(mar8_10utc - 1), -8);
        assert_eq!(pacific_offset_hours(mar8_10utc), -7);
        let nov1_9utc = days_from_civil(2026, 11, 1) * 86400 + 9 * 3600;
        assert_eq!(pacific_offset_hours(nov1_9utc - 1), -7);
        assert_eq!(pacific_offset_hours(nov1_9utc), -8);
        // 2024: March 10 and November 3
        assert_eq!(nth_sunday(2024, 3, 2), days_from_civil(2024, 3, 10));
        assert_eq!(nth_sunday(2024, 11, 1), days_from_civil(2024, 11, 3));
        // a month that starts on a Sunday
        assert_eq!(nth_sunday(2026, 11, 1), days_from_civil(2026, 11, 1));
    }

    #[test]
    fn stamps_are_second_life_time() {
        // 2026-10-12 21:05:30 UTC is 14:05 PDT
        let t = at(days_from_civil(2026, 10, 12) as u64 * 86400 + 21 * 3600 + 5 * 60 + 30);
        assert_eq!(stamp(t, true), "2026/10/12 14:05");
        assert_eq!(stamp(t, false), "14:05");
        // 2026-01-02 03:04 UTC is the day before in PST
        let t = at(days_from_civil(2026, 1, 2) as u64 * 86400 + 3 * 3600 + 4 * 60);
        assert_eq!(stamp(t, true), "2026/01/01 19:04");
    }

    #[test]
    fn a_stamp_reads_back_to_its_minute() {
        for unix in [
            days_from_civil(2026, 10, 12) * 86400 + 21 * 3600 + 5 * 60,
            days_from_civil(2026, 1, 2) * 86400 + 3 * 3600 + 4 * 60,
            days_from_civil(2026, 3, 8) * 86400 + 10 * 3600,
            days_from_civil(2026, 11, 1) * 86400 + 7 * 3600,
        ] {
            let t = at(unix as u64);
            assert_eq!(parse_stamp(&stamp(t, true)), Some(t), "{unix}");
        }
        assert_eq!(parse_stamp("14:05"), None);
        assert_eq!(parse_stamp("2026/13/01 10:00"), None);
        // 12 hour clocks and seconds of other viewers' settings
        assert_eq!(parse_stamp("2026/10/12 2:05 PM"), parse_stamp("2026/10/12 14:05"));
        assert_eq!(parse_stamp("2026/10/12 12:05 AM"), parse_stamp("2026/10/12 00:05"));
        assert_eq!(
            parse_stamp("2026/10/12 14:05:30"),
            parse_stamp("2026/10/12 14:05").map(|t| t + Duration::from_secs(30))
        );
    }

    #[test]
    fn writes_firestorms_lines() {
        assert_eq!(
            format_line(Some("2026/10/12 14:05"), "Jane Doe", "hello there"),
            "[2026/10/12 14:05]  Jane Doe: hello there"
        );
        assert_eq!(
            format_line(Some("2026/10/12 14:05"), SYSTEM_FROM, "Jane Doe is online."),
            "[2026/10/12 14:05]  Second Life: Jane Doe is online."
        );
        // LogTimestamp off
        assert_eq!(format_line(None, "Jane Doe", "hello"), "Jane Doe: hello");
        // multi-line: the next lines start with a space
        assert_eq!(
            format_line(Some("2026/10/12 14:05"), "Jane Doe", "one\ntwo\r\n\nfour"),
            "[2026/10/12 14:05]  Jane Doe: one\n two\n \n four"
        );
        // a ':' in an object name
        assert_eq!(
            format_line(Some("14:05"), "Shop: terminal v2.2", "Welcome: read the rules"),
            "[14:05]  Shop%3A terminal v2.2: Welcome: read the rules"
        );
    }

    #[test]
    fn reads_firestorms_lines() {
        let e = parse_line("[2026/10/12 14:05]  Jane Doe: hello: there").unwrap();
        assert_eq!(
            (e.stamp.as_str(), e.from.as_str(), e.text.as_str()),
            ("2026/10/12 14:05", "Jane Doe", "hello: there")
        );
        assert!(e.time.is_some());
        // complete names with a display name and non-ASCII letters
        let e = parse_line("[2026/10/12 14:05]  Ælfrëd 猫 (alfred.cat): ça va ? ☕").unwrap();
        assert_eq!((e.from.as_str(), e.text.as_str()), ("Ælfrëd 猫 (alfred.cat)", "ça va ? ☕"));
        // the escaped ':' of a name comes back
        let e = parse_line("[14:05]  Shop%3A terminal v2.2: Welcome: read the rules").unwrap();
        assert_eq!(
            (e.stamp.as_str(), e.from.as_str(), e.text.as_str()),
            ("14:05", "Shop: terminal v2.2", "Welcome: read the rules")
        );
        assert_eq!(e.time, None);
        // system lines
        let e = parse_line("[2026/10/12 14:05]  Second Life: Jane Doe is online.").unwrap();
        assert!(e.is_system());
        assert_eq!(e.text, "Jane Doe is online.");
        // no name at all: a message from Second Life
        let e = parse_line("[3:00]  Katar Ivercourt is Offline").unwrap();
        assert!(e.is_system());
        assert_eq!(e.text, "Katar Ivercourt is Offline");
        // no timestamp (LogTimestamp off)
        let e = parse_line("SuperCar: You aren't the owner").unwrap();
        assert_eq!(
            (e.stamp.as_str(), e.from.as_str(), e.text.as_str()),
            ("", "SuperCar", "You aren't the owner")
        );
        // 12 hour clock and seconds
        let e = parse_line("[2009/11/20 3:00:07 PM]  Corba ProductEngine: hi").unwrap();
        assert_eq!(
            (e.stamp.as_str(), e.from.as_str()),
            ("2009/11/20 3:00:07 PM", "Corba ProductEngine")
        );
        // an IM kept in the nearby chat transcript
        let e = parse_line("[2026/10/12 14:05]  IM: Jane Doe: psst").unwrap();
        assert_eq!((e.from.as_str(), e.text.as_str()), ("IM: Jane Doe", "psst"));
        assert_eq!(parse_line(""), None);
    }

    #[test]
    fn brackets_that_are_not_a_timestamp_stay_in_the_text() {
        let e = parse_line("[Shop] Vendor: sold out").unwrap();
        assert_eq!(
            (e.stamp.as_str(), e.from.as_str(), e.text.as_str()),
            ("", "[Shop] Vendor", "sold out")
        );
        // a timestamp needs a blank after its bracket
        let e = parse_line("[14:05]Jane: hi").unwrap();
        assert_eq!(e.stamp, "");
        assert_eq!(e.from, "[14");
    }

    #[test]
    fn what_is_written_reads_back() {
        for (from, text) in [
            ("Jane Doe", "hello"),
            ("Ælfrëd 猫 (alfred.cat)", "a: b: c"),
            ("Shop: terminal: v2", "100% of the %3A stays"),
            ("Jane Doe", "/me waves"),
        ] {
            let line = format_line(Some("2026/10/12 14:05"), from, text);
            let e = parse_line(&line).unwrap();
            assert_eq!((e.from.as_str(), e.text.as_str()), (from, text), "{line}");
        }
        // a multi-line message: the reader joins the continuation lines
        let line = format_line(Some("2026/10/12 14:05"), "Jane Doe", "one\ntwo\n\nfour");
        let mut rows = line.split('\n');
        let mut e = parse_line(rows.next().unwrap()).unwrap();
        for row in rows {
            e.text.push('\n');
            e.text.push_str(continuation(row).unwrap());
        }
        assert_eq!(e.text, "one\ntwo\n\nfour");
    }

    #[test]
    fn mentions_are_written_readable_and_read_back() {
        let id = uuid::Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        let link = format!("secondlife:///app/agent/{id}/mention");
        let text = format!("hey {link}, and {link}! see secondlife:///app/agent/{id}/about [@x](y)");
        let written = encode_mentions(&text, |_| "Jane [the] Doe".to_owned());
        assert_eq!(
            written,
            format!("hey [@Jane [the) Doe]({link}), and [@Jane [the) Doe]({link})! see secondlife:///app/agent/{id}/about [@x](y)")
        );
        assert_eq!(decode_mentions(&written), text);
        assert_eq!(
            encode_mentions("no link /mention here", |_| unreachable!()),
            "no link /mention here"
        );
        assert_eq!(decode_mentions("[@a](b) /mention) [@"), "[@a](b) /mention) [@");
        // an id that is not one stays as it is
        let bad = "secondlife:///app/agent/not-an-id/mention";
        assert_eq!(encode_mentions(bad, |_| unreachable!()), bad);
    }

    #[test]
    fn percent_sequences_of_a_name() {
        assert_eq!(unescape("100%"), "100%");
        assert_eq!(unescape("a%3Ab%3a"), "a:b:");
        assert_eq!(unescape("%zz%3"), "%zz%3");
        assert_eq!(unescape("caf%C3%A9"), "café");
    }
}
