//! Where the night sky's stars stand (Preferences › Graphics › Environment,
//! "Night sky"): the rotation the sky shader turns its star catalogue by
//! (aurora-render stars.rs, `StarFrame`).
//!
//! - **Real sky** (Aurora's default): the stars and the Milky Way as seen
//!   from Paris (48.9° N), turning around the Pole Star, which stands 48.9°
//!   above the region's north. The hour follows the region's sun (its hour
//!   angle as seen from Paris), so the sky turns once per day of the
//!   region's day cycle; which stars are up at night follows the season of
//!   today's date (the Sun's right ascension), as the real sky does.
//! - **Legacy**: the Legacy Viewer's random stars, turning slowly about the
//!   vertical, as LLDrawPoolWLSky::renderStarsDeferred
//!   (indra/newview/lldrawpoolwlsky.cpp) does: `rotatef(gFrameTimeSeconds
//!   * 0.01, 0, 0, 1)`, 0.01 degree per second.
//!
//! The region's frame is x east, y north, z up.

use aurora_render::stars::{LEGACY, REAL, StarFrame};
use glam::{DMat3, DVec3, Vec3};
use serde::{Deserialize, Serialize};

/// Latitude of Paris (degrees north).
pub const LATITUDE: f64 = 48.8566;

/// Days of the J2000.0 epoch (2000-01-01 12:00 TT) after the Unix epoch.
const J2000_UNIX_DAYS: f64 = 10957.5;

/// Which stars the night sky shows.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum NightSky {
    /// The real stars and the Milky Way over Paris.
    #[default]
    Real,
    /// The Legacy Viewer's 1,000 random stars.
    Legacy,
}

/// Where on Earth the real sky is seen from (Preferences › Graphics ›
/// Environment and Personal Lighting, "Sky seen from"). Only the latitude
/// shows: the hour follows the region's sun, so a longitude would change
/// nothing.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkyPlace {
    #[default]
    Paris,
    London,
    NewYork,
    LosAngeles,
    Tokyo,
    RioDeJaneiro,
    CapeTown,
    Sydney,
    Equator,
    NorthPole,
    /// The latitude typed in the settings.
    Custom,
}

impl SkyPlace {
    pub const ALL: [SkyPlace; 11] = [
        SkyPlace::Paris,
        SkyPlace::London,
        SkyPlace::NewYork,
        SkyPlace::LosAngeles,
        SkyPlace::Tokyo,
        SkyPlace::RioDeJaneiro,
        SkyPlace::CapeTown,
        SkyPlace::Sydney,
        SkyPlace::Equator,
        SkyPlace::NorthPole,
        SkyPlace::Custom,
    ];

    /// Latitude (degrees north) of a preset; `None` for Custom.
    pub fn latitude(self) -> Option<f32> {
        Some(match self {
            SkyPlace::Paris => LATITUDE as f32,
            SkyPlace::London => 51.507,
            SkyPlace::NewYork => 40.713,
            SkyPlace::LosAngeles => 34.052,
            SkyPlace::Tokyo => 35.676,
            SkyPlace::RioDeJaneiro => -22.907,
            SkyPlace::CapeTown => -33.925,
            SkyPlace::Sydney => -33.869,
            SkyPlace::Equator => 0.0,
            SkyPlace::NorthPole => 90.0,
            SkyPlace::Custom => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            SkyPlace::Paris => aurora_i18n::tr!("ui-options-night-sky-place-paris"),
            SkyPlace::London => aurora_i18n::tr!("ui-options-night-sky-place-london"),
            SkyPlace::NewYork => aurora_i18n::tr!("ui-options-night-sky-place-new-york"),
            SkyPlace::LosAngeles => aurora_i18n::tr!("ui-options-night-sky-place-los-angeles"),
            SkyPlace::Tokyo => aurora_i18n::tr!("ui-options-night-sky-place-tokyo"),
            SkyPlace::RioDeJaneiro => aurora_i18n::tr!("ui-options-night-sky-place-rio"),
            SkyPlace::CapeTown => aurora_i18n::tr!("ui-options-night-sky-place-cape-town"),
            SkyPlace::Sydney => aurora_i18n::tr!("ui-options-night-sky-place-sydney"),
            SkyPlace::Equator => aurora_i18n::tr!("ui-options-night-sky-place-equator"),
            SkyPlace::NorthPole => aurora_i18n::tr!("ui-options-night-sky-place-north-pole"),
            SkyPlace::Custom => aurora_i18n::tr!("ui-options-night-sky-place-custom"),
        }
    }
}

/// Default gain of the real stars' brightness (Preferences, "Star
/// brightness", 0.5 to 4 times): twice the catalogue's, so the stars show
/// over a night sky that is not quite black.
pub const DEFAULT_GAIN: f32 = 2.0;
pub const GAIN_RANGE: std::ops::RangeInclusive<f32> = 0.5..=4.0;

/// The star frame for this sky: `latitude` where the real sky is seen
/// from (degrees north), `gain` the real stars' brightness, `sun` the
/// region's sun direction, `unix_days` today's date (days since 1970-01-01
/// UTC), `seconds` the viewer's running time (Legacy rotation).
pub fn star_frame(mode: NightSky, latitude: f32, gain: f32, sun: Vec3, unix_days: f64, seconds: f64) -> StarFrame {
    match mode {
        NightSky::Real => {
            let lat = f64::from(latitude.clamp(-90.0, 90.0)).to_radians();
            let lst = sun_right_ascension(unix_days - J2000_UNIX_DAYS) + sun_hour_angle(sun.as_dvec3(), lat);
            let m = equatorial_to_region(lst, lat);
            StarFrame {
                catalogue: Some(REAL),
                axes: [m.x_axis.as_vec3(), m.y_axis.as_vec3(), m.z_axis.as_vec3()],
                milky_way: 1.0,
                gain: if gain.is_finite() {
                    gain.clamp(*GAIN_RANGE.start(), *GAIN_RANGE.end())
                } else {
                    DEFAULT_GAIN
                },
            }
        }
        NightSky::Legacy => {
            let m = DMat3::from_rotation_z((seconds * 0.01).rem_euclid(360.0).to_radians());
            StarFrame {
                catalogue: Some(LEGACY),
                axes: [m.x_axis.as_vec3(), m.y_axis.as_vec3(), m.z_axis.as_vec3()],
                milky_way: 0.0,
                gain: 1.0,
            }
        }
    }
}

/// Days since 1970-01-01 UTC, now.
pub fn unix_days_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(J2000_UNIX_DAYS, |d| d.as_secs_f64() / 86400.0)
}

/// The Sun's right ascension (radians), `n` days after J2000.0: the
/// low-precision solar position of the Astronomical Almanac (about 0.01°).
fn sun_right_ascension(n: f64) -> f64 {
    let l = (280.460 + 0.985_647_4 * n).to_radians();
    let g = (357.528 + 0.985_600_3 * n).to_radians();
    let lambda = l + (1.915 * g.sin() + 0.020 * (2.0 * g).sin()).to_radians();
    let epsilon = (23.439 - 0.000_000_4 * n).to_radians();
    (epsilon.cos() * lambda.sin()).atan2(lambda.cos())
}

/// Hour angle (radians, 0 at noon, growing westwards) of a direction of the
/// region's sky seen from latitude `lat`.
fn sun_hour_angle(s: DVec3, lat: f64) -> f64 {
    (-s.x).atan2(s.z * lat.cos() - s.y * lat.sin())
}

/// Rotation from equatorial coordinates (x towards the March equinox, z
/// towards the north celestial pole) to the region's (x east, y north, z up)
/// at local sidereal time `lst` and latitude `lat`.
fn equatorial_to_region(lst: f64, lat: f64) -> DMat3 {
    // to hour angle coordinates (x on the meridian, y towards the east
    // reversed), then tilted by the latitude
    let to_hour = DMat3::from_rotation_z(-lst);
    let tilt = DMat3::from_cols(
        DVec3::new(0.0, -lat.sin(), lat.cos()),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, lat.cos(), lat.sin()),
    );
    tilt * to_hour
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_render::stars::equatorial;

    fn region(frame: &StarFrame, ra_hours: f64, dec_deg: f64) -> Vec3 {
        let e = equatorial((ra_hours * 15.0).to_radians() as f32, (dec_deg as f32).to_radians());
        frame.axes[0] * e.x + frame.axes[1] * e.y + frame.axes[2] * e.z
    }

    /// The sun straight below the horizon: the region's midnight.
    const MIDNIGHT: Vec3 = Vec3::new(0.0, 0.0, -1.0);
    /// 2026-01-01 and 2026-07-01, 00:00 UTC.
    const WINTER: f64 = 20454.0;
    const SUMMER: f64 = 20635.0;
    const PARIS: f32 = LATITUDE as f32;

    /// Seen from any place, the celestial pole stands at its latitude: the
    /// Pole Star above the north in the north, under the horizon in the
    /// south, where the Southern Cross is up instead.
    #[test]
    fn each_place_sees_its_own_sky() {
        for place in SkyPlace::ALL {
            let Some(lat) = place.latitude() else { continue };
            let f = star_frame(NightSky::Real, lat, DEFAULT_GAIN, MIDNIGHT, WINTER, 0.0);
            let polaris = region(&f, 2.53, 89.26);
            let altitude = polaris.z.asin().to_degrees();
            assert!((altitude - lat).abs() < 1.0, "{place:?}: {altitude}");
        }
        // Acrux (RA 12h27m, Dec -63.1°): never above Paris, high over Sydney
        // at some hour of the night
        let highest = |lat: f32| {
            (0..24)
                .map(|h| {
                    let a = (h as f32 / 24.0) * std::f32::consts::TAU;
                    let sun = Vec3::new(a.cos() * 0.3, a.sin() * 0.3, -0.95).normalize();
                    region(&star_frame(NightSky::Real, lat, DEFAULT_GAIN, sun, WINTER, 0.0), 12.44, -63.1).z
                })
                .fold(f32::MIN, f32::max)
        };
        assert!(highest(PARIS) < 0.0);
        assert!(highest(SkyPlace::Sydney.latitude().unwrap()) > 0.5);
    }

    #[test]
    fn the_gain_reaches_the_real_stars_only_within_range() {
        let real = |g: f32| star_frame(NightSky::Real, PARIS, g, MIDNIGHT, WINTER, 0.0).uniform()[2][3];
        assert_eq!(real(2.0), 2.0);
        assert_eq!(real(10.0), 4.0);
        assert_eq!(real(0.0), 0.5);
        assert_eq!(real(f32::NAN), DEFAULT_GAIN);
        let legacy = star_frame(NightSky::Legacy, PARIS, 3.0, MIDNIGHT, WINTER, 0.0);
        assert_eq!(legacy.uniform()[2][3], 1.0);
        // the north pole: no NaN in the rotation
        let pole = star_frame(NightSky::Real, 90.0, DEFAULT_GAIN, MIDNIGHT, WINTER, 0.0);
        assert!(pole.axes.iter().all(|a| a.is_finite() && (a.length() - 1.0).abs() < 1e-4));
    }

    #[test]
    fn the_pole_star_stands_above_the_north_at_the_latitude_of_paris() {
        for sun in [MIDNIGHT, Vec3::Z, Vec3::X, Vec3::new(0.3, -0.5, 0.2).normalize()] {
            let f = star_frame(NightSky::Real, PARIS, DEFAULT_GAIN, sun, WINTER, 0.0);
            let polaris = region(&f, 2.53, 89.26);
            let altitude = polaris.z.asin().to_degrees();
            assert!((altitude - 48.86).abs() < 1.0, "altitude {altitude}");
            assert!(polaris.y > 0.6 && polaris.x.abs() < 0.02, "{polaris}");
        }
    }

    #[test]
    fn orion_is_up_on_winter_nights_and_down_on_summer_nights() {
        // Betelgeuse (RA 5h55m, Dec +7.4°)
        let winter = region(&star_frame(NightSky::Real, PARIS, DEFAULT_GAIN, MIDNIGHT, WINTER, 0.0), 5.92, 7.41);
        let summer = region(&star_frame(NightSky::Real, PARIS, DEFAULT_GAIN, MIDNIGHT, SUMMER, 0.0), 5.92, 7.41);
        assert!(winter.z > 0.5, "winter {winter}");
        assert!(winter.y < 0.0, "Orion is in the south: {winter}");
        assert!(summer.z < 0.0, "summer {summer}");
        // Vega (RA 18h37m, Dec +38.8°) the other way round
        let vega = region(
            &star_frame(NightSky::Real, PARIS, DEFAULT_GAIN, MIDNIGHT, SUMMER, 0.0),
            18.62,
            38.78,
        );
        assert!(vega.z > 0.7, "{vega}");
    }

    #[test]
    fn stars_rise_in_the_east_as_the_region_night_goes_on() {
        // Betelgeuse at the region's sunset, then at midnight
        let sunset = Vec3::new(-1.0, 0.0, -0.05).normalize();
        let early = region(&star_frame(NightSky::Real, PARIS, DEFAULT_GAIN, sunset, WINTER, 0.0), 5.92, 7.41);
        let late = region(&star_frame(NightSky::Real, PARIS, DEFAULT_GAIN, MIDNIGHT, WINTER, 0.0), 5.92, 7.41);
        assert!(early.x > 0.5, "in the east at dusk: {early}");
        assert!(late.z > early.z, "higher at midnight: {early} -> {late}");
    }

    #[test]
    fn the_rotation_keeps_directions_unit_and_right_handed() {
        let f = star_frame(
            NightSky::Real,
            PARIS,
            DEFAULT_GAIN,
            Vec3::new(0.2, 0.4, -0.9).normalize(),
            SUMMER,
            0.0,
        );
        let [x, y, z] = f.axes;
        assert!((x.length() - 1.0).abs() < 1e-5 && (y.length() - 1.0).abs() < 1e-5);
        assert!(x.dot(y).abs() < 1e-5 && x.cross(y).distance(z) < 1e-5);
    }

    #[test]
    fn legacy_stars_turn_a_hundredth_of_a_degree_a_second_about_the_vertical() {
        let f = star_frame(NightSky::Legacy, PARIS, DEFAULT_GAIN, MIDNIGHT, WINTER, 9000.0);
        assert_eq!(f.catalogue, Some(LEGACY));
        assert_eq!(f.milky_way, 0.0);
        assert!(f.axes[2].distance(Vec3::Z) < 1e-6);
        // 90 degrees after 9,000 s
        assert!(f.axes[0].distance(Vec3::Y) < 1e-5, "{}", f.axes[0]);
    }

    #[test]
    fn the_sun_right_ascension_follows_the_seasons() {
        let ra = |unix_days: f64| {
            sun_right_ascension(unix_days - J2000_UNIX_DAYS)
                .rem_euclid(std::f64::consts::TAU)
                .to_degrees()
        };
        // March equinox 2026 (March 20): 0°; June solstice (June 21): 90°
        assert!(ra(20532.6) < 1.0 || ra(20532.6) > 359.0, "{}", ra(20532.6));
        assert!((ra(20625.4) - 90.0).abs() < 1.0, "{}", ra(20625.4));
    }
}
