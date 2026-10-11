//! The C preprocessor's __DATE__ / __TIME__ builtins, using the local clock
//! on Windows (as Boost.Wave does). Other platforms use the UTC clock.

#[cfg(windows)]
pub(super) fn builtins() -> (String, String) {
    let mut time = windows_sys::Win32::Foundation::SYSTEMTIME::default();
    // SAFETY: GetLocalTime writes one initialized SYSTEMTIME through this valid
    // exclusive pointer, retains nothing, and needs no external input.
    unsafe {
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut time);
    }
    format(
        time.wYear.into(),
        time.wMonth.into(),
        time.wDay.into(),
        time.wHour.into(),
        time.wMinute.into(),
        time.wSecond.into(),
    )
}

#[cfg(not(windows))]
pub(super) fn builtins() -> (String, String) {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Civil date from days since the Unix epoch, via Gregorian 400-year eras.
    let days = seconds / 86400 + 719468;
    let era = days / 146097;
    let doe = days % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format(year, month, day, seconds / 3600 % 24, seconds / 60 % 60, seconds % 60)
}

fn format(year: u64, month: u64, day: u64, hour: u64, minute: u64, second: u64) -> (String, String) {
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        .get(month.saturating_sub(1) as usize)
        .unwrap_or(&"???");
    (format!("{month} {day:2} {year:04}"), format!("{hour:02}:{minute:02}:{second:02}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn builtin_clock_uses_c_literal_format() {
        assert_eq!(super::format(2026, 10, 1, 3, 4, 5), ("Oct  1 2026".into(), "03:04:05".into()));
        let (date, time) = super::builtins();
        assert_eq!(date.len(), 11);
        assert_eq!(time.len(), 8);
    }
}
