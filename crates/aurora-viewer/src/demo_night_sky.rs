//! Night sky in the offline demo
//! (`AURORA_DEMO_NIGHT_SKY=real|legacy[:<azimuth>,<elevation>]`): the
//! demo's midnight (unless `AURORA_DEMO_TOD` says otherwise), the chosen
//! stars (Preferences › Graphics › Environment, "Night sky", in memory only)
//! and the camera looking at the sky, by default north 35° up: the Pole Star
//! with the Plough and Cassiopeia on either side. Azimuth in degrees from
//! north towards east, elevation in degrees above the horizon.

use crate::world::night_sky::NightSky;
use glam::Vec3;

/// `AURORA_DEMO_NIGHT_SKY_PLACE=<latitude>[,<gain>[,<milky way>]]`: where
/// the real sky is seen from (degrees north; the place becomes Custom), the
/// stars' brightness and the Milky Way's, for this launch. An empty field
/// keeps the setting (`48.86,,1` sets the Milky Way alone).
pub fn place() -> Option<(f32, Option<f32>, Option<f32>)> {
    parse_place(&std::env::var("AURORA_DEMO_NIGHT_SKY_PLACE").ok()?)
}

fn parse_place(v: &str) -> Option<(f32, Option<f32>, Option<f32>)> {
    let mut numbers = v.split(',').map(|s| s.trim().parse::<f32>().ok());
    let latitude = numbers.next().flatten().filter(|l| l.is_finite())?;
    Some((latitude.clamp(-90.0, 90.0), numbers.next().flatten(), numbers.next().flatten()))
}

/// `AURORA_DEMO_NIGHT_SKY_CLOCK=<longitude>[,<YYYY-MM-DDTHH:MM>]`: the real
/// sky's hour from the clock (Preferences, "Star hour": Real time) at this
/// longitude (degrees east; the place becomes Custom), for this launch, and
/// the moment, in UTC, if given (the clock otherwise).
fn clock() -> Option<(f32, Option<f64>)> {
    static CLOCK: std::sync::OnceLock<Option<(f32, Option<f64>)>> = std::sync::OnceLock::new();
    *CLOCK.get_or_init(|| parse_clock(&std::env::var("AURORA_DEMO_NIGHT_SKY_CLOCK").ok()?))
}

/// The longitude of `AURORA_DEMO_NIGHT_SKY_CLOCK`.
pub fn clock_longitude() -> Option<f32> {
    clock().map(|(longitude, _)| longitude)
}

/// The fixed moment of `AURORA_DEMO_NIGHT_SKY_CLOCK`, in days since
/// 1970-01-01 UTC.
pub fn instant() -> Option<f64> {
    clock()?.1
}

fn parse_clock(v: &str) -> Option<(f32, Option<f64>)> {
    let (longitude, moment) = v.split_once(',').unwrap_or((v, ""));
    let longitude = longitude.trim().parse::<f32>().ok().filter(|l| l.is_finite())?;
    let moment = moment.trim();
    let moment = if moment.is_empty() { None } else { Some(unix_days(moment)?) };
    Some((longitude.clamp(-180.0, 180.0), moment))
}

/// `YYYY-MM-DDTHH:MM` (UTC) in days since 1970-01-01 (days_from_civil,
/// Howard Hinnant's algorithm).
fn unix_days(v: &str) -> Option<f64> {
    let (date, time) = v.split_once('T')?;
    let mut d = date.split('-').map(|s| s.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (hh, mm) = time.split_once(':')?;
    let (hh, mm) = (hh.parse::<f64>().ok()?, mm.parse::<f64>().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || !(0.0..24.0).contains(&hh) || !(0.0..60.0).contains(&mm) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days as f64 + (hh + mm / 60.0) / 24.0)
}

/// The scenario: which stars, and where the camera looks (a unit
/// direction, x east, y north, z up). `None` when off or not understood.
pub fn scenario() -> Option<(NightSky, Vec3)> {
    parse(&std::env::var("AURORA_DEMO_NIGHT_SKY").ok()?)
}

fn parse(v: &str) -> Option<(NightSky, Vec3)> {
    let (mode, view) = v.trim().split_once(':').unwrap_or((v.trim(), ""));
    let mode = match mode {
        "real" | "1" => NightSky::Real,
        "legacy" => NightSky::Legacy,
        _ => return None,
    };
    let numbers: Vec<f32> = view.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let (azimuth, elevation) = match numbers[..] {
        [a, e] => (a, e.clamp(-10.0, 89.0)),
        _ => (0.0, 35.0),
    };
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    Some((mode, Vec3::new(a.sin() * e.cos(), a.cos() * e.cos(), e.sin())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scenario_reads_the_stars_and_where_to_look() {
        let (mode, dir) = parse("real").expect("real");
        assert_eq!(mode, NightSky::Real);
        assert!(dir.y > 0.8 && dir.x.abs() < 1e-6 && dir.z > 0.5, "north, 35° up: {dir}");
        let (mode, dir) = parse("legacy:90,10").expect("legacy");
        assert_eq!(mode, NightSky::Legacy);
        assert!(dir.x > 0.98 && dir.z > 0.17, "east, 10° up: {dir}");
        assert_eq!(parse("stars"), None);
        assert_eq!(parse_place("-33.87"), Some((-33.87, None, None)));
        assert_eq!(parse_place("120,3"), Some((90.0, Some(3.0), None)));
        assert_eq!(parse_place("48.86,,0.25"), Some((48.86, None, Some(0.25))));
        assert_eq!(parse_place("48.86,1,2"), Some((48.86, Some(1.0), Some(2.0))));
        assert_eq!(parse_place("south"), None);
    }

    #[test]
    fn the_clock_reads_a_longitude_and_a_utc_moment() {
        assert_eq!(parse_clock("-74.006"), Some((-74.006, None)));
        // 2026-01-01 00:00 UTC is day 20,454; 18:00 three quarters later
        assert_eq!(parse_clock("2.35,2026-01-01T00:00"), Some((2.35, Some(20454.0))));
        assert_eq!(parse_clock("2.35,2026-01-01T18:00"), Some((2.35, Some(20454.75))));
        assert_eq!(unix_days("2000-03-01T00:00"), Some(11017.0));
        assert_eq!(parse_clock("400"), Some((180.0, None)));
        assert_eq!(parse_clock("2.35,tomorrow"), None);
        assert_eq!(parse_clock("east"), None);
    }
}
