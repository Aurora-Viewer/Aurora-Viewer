//! The resource monitor's numbers, in the language's own way of writing
//! them (decimal separator, units).

use aurora_i18n::{number, tr};

const MIB: f64 = 1024.0 * 1024.0;
const GIB: f64 = MIB * 1024.0;

/// A share (%): one decimal under 10, none above.
pub fn pct(v: f64) -> String {
    format!("{} %", number(v, if v.abs() < 10.0 { 1 } else { 0 }))
}

/// A time (ms), one decimal.
pub fn ms(v: f64) -> String {
    format!("{} ms", number(v, 1))
}

/// A size: KB under a megabyte, MB under a gigabyte, GB with two
/// decimals above.
pub fn bytes(b: f64) -> String {
    if b >= GIB {
        tr!("memory-gb", value = number(b / GIB, 2))
    } else if b > 0.0 && b < MIB {
        tr!("monitor-unit-kb", value = number(b / 1024.0, 0))
    } else if b >= 10.0 * MIB || b == 0.0 {
        tr!("memory-mb", value = number(b / MIB, 0))
    } else {
        tr!("memory-mb", value = number(b / MIB, 1))
    }
}

/// A network rate given in kb/s, in the unit that reads best.
pub fn kbps(v: f64) -> String {
    if v >= 1e6 {
        format!("{} Gb/s", number(v / 1e6, 2))
    } else if v >= 1e3 {
        format!("{} Mb/s", number(v / 1e3, 1))
    } else {
        format!("{} kb/s", number(v, 0))
    }
}

/// A disk rate given in bytes per second.
pub fn disk_rate(b: f64) -> String {
    if b >= 0.1 * MIB {
        tr!("monitor-unit-mb-s", value = number(b / MIB, 1))
    } else {
        tr!("monitor-unit-kb-s", value = number(b / 1024.0, 0))
    }
}

/// A count, grouped.
pub fn count(n: usize) -> String {
    aurora_i18n::integer(n as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_in_the_language() {
        // (the tests run in whichever language the machine is in: compare
        // with the separator it uses)
        let sep = aurora_i18n::text("number-decimal-separator");
        assert_eq!(pct(4.5), format!("4{sep}5 %"));
        assert_eq!(pct(42.4), "42 %");
        assert_eq!(ms(3.04), format!("3{sep}0 ms"));
        assert_eq!(kbps(850.4), "850 kb/s");
        assert_eq!(kbps(1250.0), format!("1{sep}2 Mb/s"));
        assert!(bytes(3.0 * GIB).starts_with(&format!("3{sep}00 ")));
        assert!(bytes(512.0 * MIB).starts_with("512 "));
    }
}
