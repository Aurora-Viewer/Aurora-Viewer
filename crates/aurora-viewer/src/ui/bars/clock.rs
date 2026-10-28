//! The bars' clock (#453): Second Life time or the computer's local time,
//! which a left click on the clock switches, with or without its seconds,
//! in 12 or 24 hours (`SystemIsland::clock_zone`, `clock_seconds`,
//! `clock_format`). Local time carries its zone's abbreviation (CET in
//! winter, CEST in summer), else its offset (UTC+4). The system island's
//! tooltip gives both times, with how far local time is from SLT.
//!
//! The Legacy Viewer's status bar shows SLT alone (LLStatusBar::
//! updateClockDisplay); FSStatusBarTimeFormat picks 12 or 24 hours, the
//! seconds and the time zone, by default the format of the viewer's
//! language (24 hours in English and French). Aurora's Auto follows the
//! system's regional format instead, the user's own 12 / 24 hours choice.

use crate::settings::{ClockFormat, ClockZone, SystemIsland};
use std::sync::{Mutex, OnceLock};

/// Seconds since the epoch, now.
pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The computer's tz database zone (Windows' own zones mapped to IANA
/// names by `iana_time_zone`), read at startup (`read_system_clock`); none
/// when it cannot be read or is not in the database.
static LOCAL_ZONE: OnceLock<Option<chrono_tz::Tz>> = OnceLock::new();

/// Local time's offset and label, worked out once a minute: a daylight
/// saving switch falls on a whole minute, so the label follows it.
struct LocalNow {
    minute: i64,
    offset: i64,
    label: String,
}

static LOCAL_NOW: Mutex<Option<LocalNow>> = Mutex::new(None);

/// Seconds to add to UTC for local time at `unix`, and its label.
fn local(unix: i64) -> (i64, String) {
    let minute = unix.div_euclid(60);
    let Ok(mut now) = LOCAL_NOW.lock() else {
        return local_at(unix);
    };
    match now.as_ref() {
        Some(n) if n.minute == minute => (n.offset, n.label.clone()),
        _ => {
            let (offset, label) = local_at(unix);
            *now = Some(LocalNow {
                minute,
                offset,
                label: label.clone(),
            });
            (offset, label)
        }
    }
}

/// Local time's offset and label at `unix`: from the zone read at startup,
/// else from the system's offset alone (a label like UTC+1).
fn local_at(unix: i64) -> (i64, String) {
    if let Some(tz) = LOCAL_ZONE.get().copied().flatten() {
        return zone_at(tz, unix);
    }
    use chrono::{Offset, TimeZone};
    let offset = chrono::Local
        .timestamp_opt(unix, 0)
        .single()
        .map_or(0, |t| i64::from(t.offset().fix().local_minus_utc()));
    (offset, zone_label(None, offset))
}

/// A zone's offset and label at `unix`.
fn zone_at(tz: chrono_tz::Tz, unix: i64) -> (i64, String) {
    use chrono::{Offset, TimeZone};
    use chrono_tz::OffsetName;
    let Some(utc) = chrono::DateTime::from_timestamp(unix, 0) else {
        return (0, zone_label(None, 0));
    };
    let offset = tz.offset_from_utc_datetime(&utc.naive_utc());
    let secs = i64::from(offset.fix().local_minus_utc());
    (secs, zone_label(offset.abbreviation(), secs))
}

/// What tells local time's zone: its abbreviation when the tz database has
/// a word for it (CET, CEST, IST, EDT), else its offset: "UTC+4",
/// "UTC+5:45", "UTC−3" (the database writes "+04" for many zones, which
/// chrono-tz gives as none).
fn zone_label(abbreviation: Option<&str>, offset: i64) -> String {
    if let Some(word) = abbreviation.filter(|w| w.len() >= 2 && w.chars().all(|c| c.is_ascii_alphabetic())) {
        return word.to_owned();
    }
    if offset == 0 {
        return "UTC".to_owned();
    }
    let sign = if offset > 0 { '+' } else { '−' };
    let minutes = offset.abs() / 60;
    match minutes % 60 {
        0 => format!("UTC{sign}{}", minutes / 60),
        m => format!("UTC{sign}{}:{m:02}", minutes / 60),
    }
}

/// Seconds to add to UTC for `zone` at `unix`.
pub(crate) fn offset(zone: ClockZone, unix: i64) -> i64 {
    match zone {
        ClockZone::Slt => crate::world::chat_log::format::pacific_offset_hours(unix) * 3600,
        ClockZone::Local => local(unix).0,
    }
}

/// The time a left click on the clock switches to.
pub(crate) fn next(zone: ClockZone) -> ClockZone {
    match zone {
        ClockZone::Slt => ClockZone::Local,
        ClockZone::Local => ClockZone::Slt,
    }
}

/// Whether the system's regional format writes 12 hours, read at startup
/// (`read_system_clock`); 24 hours until then or when it cannot be read.
static SYSTEM_12H: OnceLock<bool> = OnceLock::new();

/// Reads the system's 12 / 24 hours and time zone once, at startup, off
/// the frames.
pub(crate) fn read_system_clock() {
    SYSTEM_12H.get_or_init(|| {
        let twelve = system_pattern_is_12h();
        log::info!("clock: the system's regional format writes {} hours", if twelve { 12 } else { 24 });
        twelve
    });
    LOCAL_ZONE.get_or_init(|| match iana_time_zone::get_timezone() {
        Ok(name) => match name.parse::<chrono_tz::Tz>() {
            Ok(tz) => {
                log::info!("clock: local time zone {name}");
                Some(tz)
            }
            Err(_) => {
                log::warn!("clock: local time zone {name} is not in the tz database, its offset shown instead");
                None
            }
        },
        Err(e) => {
            log::warn!("clock: local time zone unknown ({e}), its offset shown instead");
            None
        }
    });
}

/// Windows: the user's short time format (Control Panel › Region), such as
/// "HH:mm" or "h:mm tt".
#[cfg(windows)]
fn system_pattern_is_12h() -> bool {
    use windows_sys::Win32::Globalization::{GetLocaleInfoEx, LOCALE_SSHORTTIME};
    let mut buf = [0u16; 80];
    // SAFETY: a null locale name is LOCALE_NAME_USER_DEFAULT; the buffer and
    // its length in UTF-16 units are ours and live for the call
    let n = unsafe { GetLocaleInfoEx(std::ptr::null(), LOCALE_SSHORTTIME, buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 1 {
        return false;
    }
    windows_pattern_is_12h(&String::from_utf16_lossy(&buf[..n as usize - 1]))
}

/// Linux: the time format of LC_TIME (LC_ALL, LC_TIME, LANG), read through
/// a locale object of our own, without touching the process's locale.
#[cfg(target_os = "linux")]
fn system_pattern_is_12h() -> bool {
    // SAFETY: newlocale gets a NUL-terminated empty name (the environment's
    // locale) and no base; nl_langinfo_l reads that locale, whose string is
    // copied before freelocale releases it
    unsafe {
        let loc = libc::newlocale(libc::LC_TIME_MASK, c"".as_ptr(), std::ptr::null_mut());
        if loc.is_null() {
            return false;
        }
        let fmt = libc::nl_langinfo_l(libc::T_FMT, loc);
        let pattern = if fmt.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(fmt).to_string_lossy().into_owned()
        };
        libc::freelocale(loc);
        posix_pattern_is_12h(&pattern)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn system_pattern_is_12h() -> bool {
    false
}

/// Whether a Windows time pattern writes 12 hours: `h` (12) rather than
/// `H` (24), or the AM / PM marker `t`, outside quoted text. Tested on
/// every system, read on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_pattern_is_12h(pattern: &str) -> bool {
    let mut quoted = false;
    for c in pattern.chars() {
        match c {
            '\'' => quoted = !quoted,
            'h' | 't' if !quoted => return true,
            'H' if !quoted => return false,
            _ => {}
        }
    }
    false
}

/// Whether a strftime pattern (LC_TIME's T_FMT) writes 12 hours: %I, %l,
/// %r or %p rather than %H, %k or %T. Tested on every system, read on
/// Linux.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn posix_pattern_is_12h(pattern: &str) -> bool {
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            // the flags and widths of GNU strftime before the conversion
            let conv = chars
                .by_ref()
                .find(|c| !matches!(c, '-' | '_' | '0' | '^' | '#' | 'E' | 'O' | '1'..='9'));
            match conv {
                Some('I' | 'l' | 'r' | 'p' | 'P') => return true,
                Some('H' | 'k' | 'T' | 'R') => return false,
                _ => {}
            }
        }
    }
    false
}

/// Whether the clock writes 12 hours.
fn twelve_hours(format: ClockFormat) -> bool {
    match format {
        ClockFormat::Auto => SYSTEM_12H.get().copied().unwrap_or(false),
        ClockFormat::H12 => true,
        ClockFormat::H24 => false,
    }
}

/// "14:05", "14:05:09", "2:05 PM" or "2:05:09 PM", at `offset` seconds
/// from UTC.
fn hours(unix: i64, offset: i64, seconds: bool, twelve: bool) -> String {
    let t = (unix + offset).rem_euclid(86400);
    let (h, m, s) = (t / 3600, (t / 60) % 60, t % 60);
    let secs = if seconds { format!(":{s:02}") } else { String::new() };
    if twelve {
        let marker = if h < 12 {
            aurora_i18n::tr!("ui-bars-clock-am")
        } else {
            aurora_i18n::tr!("ui-bars-clock-pm")
        };
        let h12 = match h % 12 {
            0 => 12,
            h => h,
        };
        format!("{h12}:{m:02}{secs} {marker}")
    } else {
        format!("{h:02}:{m:02}{secs}")
    }
}

/// The clock's offset from UTC and its zone's label, at `unix`.
fn zone(zone: ClockZone, unix: i64) -> (i64, String) {
    match zone {
        ClockZone::Slt => (offset(ClockZone::Slt, unix), "SLT".to_owned()),
        ClockZone::Local => local(unix),
    }
}

/// The clock's text, now: the time and SLT, or local time and its zone.
pub(crate) fn text(s: SystemIsland) -> String {
    let unix = now_unix();
    let (offset, label) = zone(s.clock_zone, unix);
    format!("{} {label}", hours(unix, offset, s.clock_seconds, twelve_hours(s.clock_format)))
}

/// The clock's texts at 10:00 and 22:00 in its current format, which its
/// readout keeps room for (#1246): a 12-hour clock's two-digit hours and
/// the wider of its two markers do not move the bar at 10:00 or at noon.
pub(crate) fn widest(s: SystemIsland) -> [String; 2] {
    let (_, label) = zone(s.clock_zone, now_unix());
    let twelve = twelve_hours(s.clock_format);
    [10, 22].map(|h| format!("{} {label}", hours(0, h * 3600, s.clock_seconds, twelve)))
}

/// How far a time `diff` seconds from SLT is: "SLT +9 h", "SLT −8 h",
/// "SLT +5 h 30", "SLT" when they are the same.
fn from_slt(diff: i64) -> String {
    if diff == 0 {
        return "SLT".to_owned();
    }
    let sign = if diff > 0 { '+' } else { '−' };
    let minutes = diff.abs() / 60;
    match minutes % 60 {
        0 => format!("SLT {sign}{} h", minutes / 60),
        m => format!("SLT {sign}{} h {m:02}", minutes / 60),
    }
}

/// The tooltip's rows: SLT, then local time with how far it is from SLT.
pub(crate) fn tip_values(s: SystemIsland) -> [String; 2] {
    let unix = now_unix();
    let slt = offset(ClockZone::Slt, unix);
    let (local, label) = local(unix);
    let twelve = twelve_hours(s.clock_format);
    [
        format!("{} SLT", hours(unix, slt, s.clock_seconds, twelve)),
        format!(
            "{} {label} ({})",
            hours(unix, local, s.clock_seconds, twelve),
            from_slt(local - slt)
        ),
    ]
}

/// The Legacy bars' tooltip over the clock: both times, then what a click
/// does.
pub(crate) fn hover_text(s: SystemIsland) -> String {
    use aurora_i18n::tr;
    let [slt, local] = tip_values(s);
    format!(
        "{}: {slt}\n{}: {local}\n{}",
        tr!("ui-bars-tip-slt"),
        tr!("ui-bars-tip-local"),
        tr!("ui-bars-clock-click-hint")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_switches_slt_and_local_time() {
        assert_eq!(next(ClockZone::Slt), ClockZone::Local);
        assert_eq!(next(ClockZone::Local), ClockZone::Slt);
    }

    #[test]
    fn hours_in_24_and_12_hours() {
        // 2026-01-15 22:05:09 UTC
        let unix = 1_768_514_709;
        assert_eq!(hours(unix, 0, true, false), "22:05:09");
        assert_eq!(hours(unix, 0, false, false), "22:05");
        assert_eq!(hours(unix, 0, false, true), "10:05 PM");
        assert_eq!(hours(unix, 0, true, true), "10:05:09 PM");
        // in January SLT is PST, UTC-8
        let slt = offset(ClockZone::Slt, unix);
        assert_eq!(hours(unix, slt, false, false), "14:05");
        assert_eq!(hours(unix, slt, false, true), "2:05 PM");
        // midnight and noon
        assert_eq!(hours(unix, 2 * 3600 - 5 * 60 - 9, false, true), "12:00 AM");
        assert_eq!(hours(unix, -10 * 3600 - 5 * 60 - 9, false, true), "12:00 PM");
        // a positive offset past midnight wraps
        assert_eq!(hours(unix, 3 * 3600, false, false), "01:05");
        assert_eq!(hours(unix, 3 * 3600, false, true), "1:05 AM");
    }

    #[test]
    fn slt_follows_daylight_saving_time() {
        // 2026-07-15 12:00 UTC: PDT, UTC-7
        assert_eq!(offset(ClockZone::Slt, 1_784_116_800), -7 * 3600);
        // 2026-01-15: PST, UTC-8
        assert_eq!(offset(ClockZone::Slt, 1_768_478_400), -8 * 3600);
    }

    #[test]
    fn distance_from_slt() {
        assert_eq!(from_slt(0), "SLT");
        assert_eq!(from_slt(9 * 3600), "SLT +9 h");
        assert_eq!(from_slt(-3 * 3600), "SLT −3 h");
        assert_eq!(from_slt(13 * 3600 + 1800), "SLT +13 h 30");
        assert_eq!(from_slt(-(3600 + 2700)), "SLT −1 h 45");
    }

    /// The zone's word in summer and winter, a word all year, the offset
    /// where the tz database only has numbers, and without a zone.
    #[test]
    fn local_time_zone_labels() {
        // 2026-07-15 and 2026-01-15, 12:00 UTC
        let (summer, winter) = (1_784_116_800, 1_768_478_400);
        assert_eq!(zone_at(chrono_tz::Europe::Paris, summer), (7200, "CEST".to_owned()));
        assert_eq!(zone_at(chrono_tz::Europe::Paris, winter), (3600, "CET".to_owned()));
        assert_eq!(zone_at(chrono_tz::Asia::Kolkata, summer), (19800, "IST".to_owned()));
        assert_eq!(zone_at(chrono_tz::America::New_York, winter), (-18000, "EST".to_owned()));
        // "+04", "+0545", "-03" in the database
        assert_eq!(zone_at(chrono_tz::Asia::Dubai, summer), (14400, "UTC+4".to_owned()));
        assert_eq!(zone_at(chrono_tz::Asia::Kathmandu, winter), (20700, "UTC+5:45".to_owned()));
        assert_eq!(zone_at(chrono_tz::America::Sao_Paulo, summer), (-10800, "UTC−3".to_owned()));
        assert_eq!(zone_at(chrono_tz::UTC, summer), (0, "UTC".to_owned()));
        // no zone, or a label that is not a word
        assert_eq!(zone_label(None, 19800), "UTC+5:30");
        assert_eq!(zone_label(None, -12600), "UTC−3:30");
        assert_eq!(zone_label(Some("+03"), 10800), "UTC+3");
        assert_eq!(zone_label(Some(""), 0), "UTC");
    }

    /// Windows short time patterns of a few regional formats.
    #[test]
    fn windows_regional_patterns() {
        // en-US, en-AU, ko-KR (tt before the hours), zh-TW
        for twelve in ["h:mm tt", "h:mm tt", "tt h:mm", "tt hh:mm"] {
            assert!(windows_pattern_is_12h(twelve), "{twelve}");
        }
        // fr-FR, de-DE, en-GB, ja-JP, a quoted literal with an h in it
        for twenty_four in ["HH:mm", "HH:mm", "HH:mm", "H:mm", "HH'h'mm", "H 'heures' mm"] {
            assert!(!windows_pattern_is_12h(twenty_four), "{twenty_four}");
        }
        assert!(!windows_pattern_is_12h(""));
    }

    /// LC_TIME's T_FMT of a few glibc locales.
    #[test]
    fn posix_time_patterns() {
        // en_US, en_AU, a %l without padding, a flagged %-I
        for twelve in ["%r", "%I:%M:%S %p", "%l:%M:%S %P", "%-I:%M %p"] {
            assert!(posix_pattern_is_12h(twelve), "{twelve}");
        }
        // C, fr_FR, de_DE, %k, %R, literal percents
        for twenty_four in ["%H:%M:%S", "%T", "%H:%M:%S", "%k:%M", "%R", "%%I %H"] {
            assert!(!posix_pattern_is_12h(twenty_four), "{twenty_four}");
        }
        assert!(!posix_pattern_is_12h(""));
    }
}
