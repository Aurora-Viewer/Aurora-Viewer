//! The bars' clock (#453): Second Life time, the computer's local time or
//! UTC, which a left click on the clock cycles through, with or without its
//! seconds (`SystemIsland::clock_zone`, `clock_seconds`). The system
//! island's tooltip gives the three, with how far local time is from SLT.
//! The Legacy Viewer's status bar shows SLT alone, its seconds and 12 / 24
//! hours chosen in Preferences (LLStatusBar::updateClockDisplay,
//! FSStatusBarTimeFormat).

use crate::settings::ClockZone;

/// Seconds since the epoch, now.
pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Seconds to add to UTC for the computer's time zone at `unix`.
fn local_offset(unix: i64) -> i64 {
    use chrono::{Offset, TimeZone};
    chrono::Local
        .timestamp_opt(unix, 0)
        .single()
        .map_or(0, |t| i64::from(t.offset().fix().local_minus_utc()))
}

/// Seconds to add to UTC for `zone` at `unix`.
pub(crate) fn offset(zone: ClockZone, unix: i64) -> i64 {
    match zone {
        ClockZone::Slt => crate::world::chat_log::format::pacific_offset_hours(unix) * 3600,
        ClockZone::Local => local_offset(unix),
        ClockZone::Utc => 0,
    }
}

/// The zone after `zone` for a left click on the clock.
pub(crate) fn next(zone: ClockZone) -> ClockZone {
    match zone {
        ClockZone::Slt => ClockZone::Local,
        ClockZone::Local => ClockZone::Utc,
        ClockZone::Utc => ClockZone::Slt,
    }
}

/// "14:05" or "14:05:09", at `offset` seconds from UTC.
fn hours(unix: i64, offset: i64, seconds: bool) -> String {
    let t = (unix + offset).rem_euclid(86400);
    if seconds {
        format!("{:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
    } else {
        format!("{:02}:{:02}", t / 3600, (t / 60) % 60)
    }
}

/// What tells the zone after the time: SLT, UTC, the word for local time.
fn suffix(zone: ClockZone) -> &'static str {
    match zone {
        ClockZone::Slt => "SLT",
        ClockZone::Utc => "UTC",
        ClockZone::Local => aurora_i18n::tr!("ui-bars-clock-local"),
    }
}

/// The time of `zone` at `unix`, with what tells its zone.
fn text_at(zone: ClockZone, seconds: bool, unix: i64, offset: i64) -> String {
    format!("{} {}", hours(unix, offset, seconds), suffix(zone))
}

/// The clock's text, now.
pub(crate) fn text(zone: ClockZone, seconds: bool) -> String {
    let unix = now_unix();
    text_at(zone, seconds, unix, offset(zone, unix))
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

/// The tooltip's rows: SLT, local time with how far it is from SLT, UTC.
pub(crate) fn tip_values(seconds: bool) -> [String; 3] {
    let unix = now_unix();
    let (slt, local) = (offset(ClockZone::Slt, unix), local_offset(unix));
    [
        text_at(ClockZone::Slt, seconds, unix, slt),
        format!("{} ({})", hours(unix, local, seconds), from_slt(local - slt)),
        text_at(ClockZone::Utc, seconds, unix, 0),
    ]
}

/// The Legacy bars' tooltip over the clock: the three times, then what a
/// click does.
pub(crate) fn hover_text(seconds: bool) -> String {
    use aurora_i18n::tr;
    let [slt, local, utc] = tip_values(seconds);
    format!(
        "{}: {slt}\n{}: {local}\n{}: {utc}\n{}",
        tr!("ui-bars-tip-slt"),
        tr!("ui-bars-tip-local"),
        tr!("ui-bars-tip-utc"),
        tr!("ui-bars-clock-click-hint")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_cycle_through_the_three_times() {
        assert_eq!(next(ClockZone::Slt), ClockZone::Local);
        assert_eq!(next(ClockZone::Local), ClockZone::Utc);
        assert_eq!(next(ClockZone::Utc), ClockZone::Slt);
    }

    #[test]
    fn hours_with_and_without_seconds() {
        // 2026-01-15 22:05:09 UTC
        let unix = 1_768_514_709;
        assert_eq!(hours(unix, 0, true), "22:05:09");
        assert_eq!(hours(unix, 0, false), "22:05");
        // in January SLT is PST, UTC-8
        assert_eq!(text_at(ClockZone::Slt, false, unix, offset(ClockZone::Slt, unix)), "14:05 SLT");
        // a positive offset past midnight wraps
        assert_eq!(hours(unix, 3 * 3600, false), "01:05");
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
}
