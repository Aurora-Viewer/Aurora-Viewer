//! The C preprocessor's __DATE__ / __TIME__ builtins, using the local clock
//! (as Boost.Wave does).

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
    use chrono::{Datelike, Timelike};
    let t = chrono::Local::now();
    format(
        u64::try_from(t.year()).unwrap_or(0),
        t.month().into(),
        t.day().into(),
        t.hour().into(),
        t.minute().into(),
        t.second().into(),
    )
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
