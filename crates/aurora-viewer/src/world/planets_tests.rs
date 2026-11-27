//! Checks against an independent, numerically integrated ephemeris (not
//! the tables used here) for twelve moments from 1990 to 2040, including
//! Venus at and near inferior conjunction (2012, 2020, 2025) and Mars at
//! opposition (2003, 2018, 2035).

use super::reference::*;
use super::*;

/// Unix days of a UT Julian date.
fn unix(jd: f64) -> f64 {
    jd - UNIX_EPOCH_JD
}

fn sep_arcmin(a: [f64; 3], ra: f64, dec: f64) -> f64 {
    angle_deg(a, radec(ra.to_radians(), dec.to_radians())) * 60.0
}

/// The table's published nominal heliocentric errors for 1800-2050: per
/// body, longitude (arcsec) and distance (1000 km); then the barycentre's.
const HELIO_ERRORS: [(f64, f64); 6] = [
    (15.0, 1.0),
    (20.0, 4.0),
    (40.0, 25.0),
    (400.0, 600.0),
    (600.0, 1500.0),
    (50.0, 1000.0),
];
const EMB_ERROR: (f64, f64) = (20.0, 6.0);

/// The position tolerance (arcminutes) for one body at one moment: the
/// published heliocentric errors of the planet and of the Earth, both seen
/// from the planet's distance (an angle error at the Sun's distance r
/// moves the planet by r times it; a distance error moves it sideways by
/// at most its size), doubled, plus half an arcminute for what the method
/// leaves out (aberration is in neither value; light time is applied).
fn tolerance_arcmin(i: usize, r: f64, delta: f64) -> f64 {
    let km_to_arcsec = |km: f64, au: f64| (km / (au * AU_KM)).to_degrees() * 3600.0;
    let (lp, rp) = HELIO_ERRORS[i];
    let (le, re) = EMB_ERROR;
    let planet = lp * r / delta + km_to_arcsec(rp * 1000.0, delta);
    let earth = le * 1.0 / delta + km_to_arcsec(re * 1000.0, delta);
    2.0 * (planet + earth) / 60.0 + 0.5
}

#[test]
fn planets_match_the_reference_ephemeris() {
    let mut worst = [0.0f64; 6];
    for (k, &jd) in DATES.iter().enumerate() {
        let sky = sky_at(unix(jd)).expect("in range");
        for (i, p) in sky.planets.iter().enumerate() {
            let [ra, dec, _, r, delta, phase] = PLANETS[i][k];
            let err = sep_arcmin(p.dir, ra, dec);
            let tol = tolerance_arcmin(i, r, delta);
            assert!(err < tol, "{:?} at JD {jd}: {err:.2}' > {tol:.2}'", p.body);
            assert!(
                (p.distance - delta).abs() / delta < 2e-3,
                "{:?} at JD {jd}: delta {} vs {delta}",
                p.body,
                p.distance
            );
            assert!((p.sun_distance - r).abs() / r < 2e-3, "{:?} r", p.body);
            assert!(
                (p.phase_angle - phase).abs() < 0.3,
                "{:?} at JD {jd}: phase {} vs {phase}",
                p.body,
                p.phase_angle
            );
            worst[i] = worst[i].max(err / tol);
            println!("{:?} JD {jd}: {err:.2}' (tolerance {tol:.2}')", p.body);
        }
    }
    println!("worst error / tolerance per body: {worst:?}");
}

/// Magnitude tolerance per body: the reference computes V from the same
/// published model (its Saturn note matches Eq. 10), so this checks the
/// transcription and the geometry, not the model. Mars carries the left
/// out rotation and season terms; Venus near inferior conjunction moves
/// about 0.1 mag per degree of phase.
const MAG_TOLERANCE: [f64; 6] = [0.06, 0.1, 0.15, 0.03, 0.06, 0.03];

#[test]
fn magnitudes_match_the_reference() {
    let mut worst = [0.0f64; 6];
    for (k, &jd) in DATES.iter().enumerate() {
        let sky = sky_at(unix(jd)).expect("in range");
        for (i, p) in sky.planets.iter().enumerate() {
            let v = PLANETS[i][k][2];
            assert!(p.magnitude.is_finite());
            if v.is_nan() {
                continue;
            }
            let err = (p.magnitude - v).abs();
            worst[i] = worst[i].max(err);
            assert!(err < MAG_TOLERANCE[i], "{:?} at JD {jd}: V {:.3} vs {v}", p.body, p.magnitude);
        }
    }
    println!("worst magnitude error per body: {worst:?}");
}

#[test]
fn sun_matches_the_reference() {
    let mut worst = 0.0f64;
    for (k, &jd) in DATES.iter().enumerate() {
        let sky = sky_at(unix(jd)).expect("in range");
        let [ra, dec, _] = SUN[k];
        let err = sep_arcmin(sky.sun, ra, dec);
        worst = worst.max(err);
        // the barycentre's 20" and the light time's 20", with margin
        assert!(err < 1.5, "Sun at JD {jd}: {err:.2}'");
    }
    println!("Sun worst: {worst:.2}'");
}

#[test]
fn moon_matches_the_reference() {
    let mut worst = 0.0f64;
    for (k, &jd) in DATES.iter().enumerate() {
        let sky = sky_at(unix(jd)).expect("in range");
        let [ra, dec, delta] = MOON[k];
        let err = sep_arcmin(sky.moon.dir, ra, dec);
        worst = worst.max(err);
        // claimed at most about 0.35°
        assert!(err < 0.4 * 60.0, "Moon at JD {jd}: {err:.1}'");
        let d = sky.moon.distance_earth_radii * EARTH_RADIUS_KM / AU_KM;
        assert!((d - delta).abs() / delta < 0.005, "Moon distance {d} vs {delta}");
    }
    println!("Moon worst: {worst:.1}'");
}

/// The region frame of night_sky.rs (`equatorial_to_region`) at the real
/// hour: Greenwich mean sidereal time plus the longitude.
fn to_region(v: [f64; 3], lst: f64, lat: f64) -> [f64; 3] {
    let (s, c) = lst.sin_cos();
    let h = [c * v[0] + s * v[1], -s * v[0] + c * v[1], v[2]];
    let (sl, cl) = lat.sin_cos();
    [h[1], -sl * h[0] + cl * h[2], cl * h[0] + sl * h[2]]
}

fn from_region(r: [f64; 3], lst: f64, lat: f64) -> [f64; 3] {
    let (sl, cl) = lat.sin_cos();
    let h = [-sl * r[1] + cl * r[2], r[0], cl * r[1] + sl * r[2]];
    let (s, c) = lst.sin_cos();
    [c * h[0] - s * h[1], s * h[0] + c * h[1], h[2]]
}

#[test]
fn the_moons_parallax_seen_from_paris() {
    let (lon, lat) = (2.3522f64.to_radians(), 48.8566f64.to_radians());
    let mut worst = 0.0f64;
    for (k, &jd) in DATES.iter().enumerate() {
        let sky = sky_at(unix(jd)).expect("in range");
        let n = jd - J2000_JD;
        let lst = (280.460_618_37 + 360.985_647_366_29 * n).rem_euclid(360.0).to_radians() + lon;
        let geo = to_region(sky.moon.dir, lst, lat);
        let topo = from_region(topocentric(geo, sky.moon.distance_earth_radii), lst, lat);
        let [ra, dec, _] = MOON_PARIS[k];
        // the shift itself, against the reference's own shift: this checks
        // the parallax step apart from the series' error
        let ours = angle_deg(topo, sky.moon.dir);
        let theirs = angle_deg(
            radec(ra.to_radians(), dec.to_radians()),
            radec(MOON[k][0].to_radians(), MOON[k][1].to_radians()),
        );
        assert!(
            (ours - theirs).abs() * 60.0 < 1.5,
            "parallax at JD {jd}: {ours:.3}° vs {theirs:.3}°"
        );
        let err = sep_arcmin(topo, ra, dec);
        worst = worst.max(err);
        assert!(err < 0.4 * 60.0, "topocentric Moon at JD {jd}: {err:.1}'");
    }
    println!("Moon from Paris worst: {worst:.1}'");
}

#[test]
fn smooth_over_a_day() {
    // minute by minute: no jump anywhere (Kepler's steps, angle wraps)
    let start = unix(2_459_205.25);
    let mut last = sky_at(start).expect("in range");
    for minute in 1..=1440 {
        let now = sky_at(start + f64::from(minute) / 1440.0).expect("in range");
        for (a, b) in last.planets.iter().zip(now.planets.iter()) {
            assert!(angle_deg(a.dir, b.dir) < 0.005, "{:?} jumps", a.body);
            assert!((a.magnitude - b.magnitude).abs() < 0.01, "{:?} brightness jumps", a.body);
        }
        assert!(angle_deg(last.sun, now.sun) < 0.002);
        // the Moon moves about 0.009° a minute
        assert!(angle_deg(last.moon.dir, now.moon.dir) < 0.02);
        last = now;
    }
}

#[test]
fn finite_over_the_whole_range_and_refused_outside() {
    let first = FIRST_JD - UNIX_EPOCH_JD;
    let last = LAST_JD - UNIX_EPOCH_JD;
    let mut d = first + 0.01;
    let mut n = 0;
    while d < last {
        let sky = sky_at(d).expect("in range");
        for p in &sky.planets {
            assert!(p.dir.iter().all(|x| x.is_finite()), "{:?} at {d}", p.body);
            assert!(
                p.magnitude.is_finite() && p.magnitude > -6.0 && p.magnitude < 7.0,
                "{:?} V {} at {d}",
                p.body,
                p.magnitude
            );
            assert!(gpu_point(p).iter().flatten().all(|x| x.is_finite()));
        }
        assert!(sky.sun.iter().chain(sky.moon.dir.iter()).all(|x| x.is_finite()));
        assert!((55.0..65.0).contains(&sky.moon.distance_earth_radii));
        d += 1.37;
        n += 1;
    }
    assert!(n > 60_000);
    for bad in [first - 1.0, last + 1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300] {
        assert!(sky_at(bad).is_none(), "{bad}");
    }
}

#[test]
fn uranus_stays_within_its_published_band() {
    // the magnitude model's own range: 5.38 brightest (2054), 6.03 faintest
    let mut d = unix(2_447_892.5);
    while d < unix(2_466_335.5) {
        let v = sky_at(d).expect("in range").planets[5].magnitude;
        assert!((5.3..6.1).contains(&v), "Uranus V {v} at {d}");
        d += 5.0;
    }
}

#[test]
fn inner_planets_keep_close_to_the_sun() {
    let mut d = unix(2_447_892.5);
    while d < unix(2_466_335.5) {
        let sky = sky_at(d).expect("in range");
        assert!(sky.planets[0].elongation < 28.5);
        assert!(sky.planets[1].elongation < 47.9);
        d += 1.0;
    }
}

#[test]
fn colours_and_brightness_follow_the_star_law() {
    for b in Body::ALL {
        let c = b.colour();
        let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        assert!((l - 1.0).abs() < 1e-5, "{b:?}");
    }
    // the star table's own law: magnitude 0 is 2, the cap at 3
    let p = |m: f64| Planet {
        body: Body::Venus,
        dir: [0.0, 0.0, 1.0],
        distance: 1.0,
        sun_distance: 1.0,
        phase_angle: 0.0,
        elongation: 40.0,
        magnitude: m,
    };
    assert!((gpu_point(&p(0.0))[0][3] - 2.0).abs() < 1e-6);
    assert_eq!(gpu_point(&p(-4.4))[0][3], 3.0);
    assert!((gpu_point(&p(-1.0))[1][3] - 1.2).abs() < 1e-6);
    assert_eq!(gpu_point(&p(1.3))[0][3], aurora_render::stars::intensity(1.3));
}
