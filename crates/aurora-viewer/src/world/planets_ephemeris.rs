//! Where the Sun, the Moon and the naked-eye planets (and Uranus) stand
//! among the stars, and how bright the planets are, for one moment
//! (world/planets.rs draws them).
//!
//! Everything here is in the star table's own frame: equatorial, mean
//! equator and equinox of J2000 (x towards the March equinox, z towards the
//! north celestial pole), the frame of aurora-render `stars::equatorial`.
//! The star field's rotation to the region (`StarFrame::axes`) then places
//! these directions exactly as it places the stars, whatever the place, the
//! hour or the season. Nothing here depends on the observer: the only input
//! is a moment. The Moon's topocentric parallax (up to about a degree) is
//! the one observer-dependent step, left to [`topocentric`], which works in
//! the region's frame after that rotation.
//!
//! Methods, all published as numbers and formulas only (credited in
//! NOTICE.md, "Bundled assets"):
//!
//! - Planets: the approximate Keplerian elements of the major planets of
//!   E. M. Standish and J. G. Williams, Table 1, as NASA/JPL Solar System
//!   Dynamics publishes them (<https://ssd.jpl.nasa.gov/planets/approx_pos.html>;
//!   public domain; valid 1800 AD to 2050 AD, elements and rates on the
//!   mean ecliptic and equinox of J2000). The Earth is the Earth-Moon
//!   barycentre of the same table, moved by the Moon's share; one
//!   light-time step is applied.
//! - Planet magnitudes: A. Mallama and J. L. Hilton, "Computing Apparent
//!   Planetary Magnitudes for The Astronomical Almanac", Astronomy and
//!   Computing 25 (2018) (<https://arxiv.org/abs/1808.01973>), its Eqs. 2
//!   to 14 (the Mars rotation and season terms, at most a few hundredths of
//!   a magnitude, are left out).
//! - Moon: the Astronomical Almanac's low-precision formulae for the Moon's
//!   ecliptic longitude, latitude and parallax, as D. G. Simpson (NASA
//!   Goddard Space Flight Center, 1998) reviews them, Eqs. 1 to 13
//!   (<https://ntrs.nasa.gov/api/citations/19990040527>), precessed from the
//!   ecliptic of date to J2000.
//! - Saturn's and Uranus' poles of rotation (for the rings' tilt and the
//!   sub-latitudes the magnitudes need) and Uranus' flattening: the NSSDCA
//!   planetary fact sheets of NASA Goddard Space Flight Center
//!   (<https://nssdc.gsfc.nasa.gov/planetary/factsheet/saturnfact.html>,
//!   `uranusfact.html`).
//!
//! The tests compare with reference values of NASA/JPL's Horizons system
//! (ephemeris DE441, `planets_reference.rs`).
//!
//! No allocation, no panic: any input is either computed or refused with
//! `None` (outside 1800 to 2050, or not a number), which the caller treats
//! as "no planets".

/// Days from the Unix epoch (1970-01-01 00:00 UTC) to Julian date 0.
const UNIX_EPOCH_JD: f64 = 2_440_587.5;
/// Julian date of J2000.0.
const J2000_JD: f64 = 2_451_545.0;
/// Terrestrial Time less UTC, taken as fixed (seconds). The tables are in
/// TDB; the real difference drifts from about 57 s (1990) to about 70 s
/// (2026), a few seconds' error over 1990-2040, well under the methods'
/// own (the Moon, the fastest, moves about 0.15 arcminute in 15 s). Around
/// 1800-1900 the real difference was near zero, about a minute off: 0.5
/// arcminute for the Moon, still under its own error.
const DELTA_T_SECONDS: f64 = 69.184;
/// First and last moments of the elements' validity (Julian dates, TT):
/// 1800-01-01 00:00 and 2051-01-01 00:00.
pub const FIRST_JD: f64 = 2_378_496.5;
pub const LAST_JD: f64 = 2_470_172.5;
/// Obliquity of the ecliptic at J2000 used by the elements' table (degrees).
const OBLIQUITY_J2000: f64 = 23.439_28;
/// Obliquity used by the lunar formulae: 23° 26' 21.448" (IAU 1976).
const OBLIQUITY_MOON: f64 = 23.0 + 26.0 / 60.0 + 21.448 / 3600.0;
/// Light's travel time per astronomical unit (days).
const LIGHT_DAYS_PER_AU: f64 = 499.004_783_8 / 86_400.0;
/// Earth's equatorial radius (km, IAU 1976), the lunar formulae's unit.
const EARTH_RADIUS_KM: f64 = 6_378.140;
const AU_KM: f64 = 149_597_870.7;
/// Earth to Moon mass ratio: the barycentre's offset from the Earth.
const EARTH_MOON_MASS_RATIO: f64 = 81.300_56;

/// The bodies drawn, in this order everywhere (`Sky::planets`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
}

impl Body {
    pub const ALL: [Body; 6] = [Body::Mercury, Body::Venus, Body::Mars, Body::Jupiter, Body::Saturn, Body::Uranus];

    /// Lower-case identifier (data files, logs).
    pub fn id(self) -> &'static str {
        match self {
            Body::Mercury => "mercury",
            Body::Venus => "venus",
            Body::Mars => "mars",
            Body::Jupiter => "jupiter",
            Body::Saturn => "saturn",
            Body::Uranus => "uranus",
        }
    }

    /// The point's colour, linear, luminance 1 like `stars::colour`. A look
    /// choice, not a measurement: Venus white (its B-V would make it
    /// yellowish), Mars orange-red, Jupiter cream, Saturn pale gold, Uranus
    /// pale cyan, Mercury grey-white; tuned by eye on the captures.
    pub fn colour(self) -> [f32; 3] {
        let c = match self {
            Body::Mercury => [0.97, 0.95, 0.90],
            Body::Venus => [1.0, 0.99, 0.96],
            Body::Mars => [1.6, 0.85, 0.55],
            Body::Jupiter => [1.1, 1.0, 0.85],
            Body::Saturn => [1.15, 1.0, 0.72],
            Body::Uranus => [0.9, 1.02, 1.05],
        };
        let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        [c[0] / l, c[1] / l, c[2] / l]
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// The table of approximate elements: per body, the elements a (au),
/// e, I, L, long. peri., long. node (degrees) at J2000, then their rates
/// per Julian century. Mean ecliptic and equinox of J2000.
const ELEMENTS: [[[f64; 6]; 2]; 6] = [
    // Mercury
    [
        [
            0.387_099_27,
            0.205_635_93,
            7.004_979_02,
            252.250_323_50,
            77.457_796_28,
            48.330_765_93,
        ],
        [
            0.000_000_37,
            0.000_019_06,
            -0.005_947_49,
            149_472.674_111_75,
            0.160_476_89,
            -0.125_340_81,
        ],
    ],
    // Venus
    [
        [
            0.723_335_66,
            0.006_776_72,
            3.394_676_05,
            181.979_099_50,
            131.602_467_18,
            76.679_842_55,
        ],
        [
            0.000_003_90,
            -0.000_041_07,
            -0.000_788_90,
            58_517.815_387_29,
            0.002_683_29,
            -0.277_694_18,
        ],
    ],
    // Mars
    [
        [
            1.523_710_34,
            0.093_394_10,
            1.849_691_42,
            -4.553_432_05,
            -23.943_629_59,
            49.559_538_91,
        ],
        [
            0.000_018_47,
            0.000_078_82,
            -0.008_131_31,
            19_140.302_684_99,
            0.444_410_88,
            -0.292_573_43,
        ],
    ],
    // Jupiter
    [
        [
            5.202_887_00,
            0.048_386_24,
            1.304_396_95,
            34.396_440_51,
            14.728_479_83,
            100.473_909_09,
        ],
        [
            -0.000_116_07,
            -0.000_132_53,
            -0.001_837_14,
            3_034.746_127_75,
            0.212_526_68,
            0.204_691_06,
        ],
    ],
    // Saturn
    [
        [
            9.536_675_94,
            0.053_861_79,
            2.485_991_87,
            49.954_244_23,
            92.598_878_31,
            113.662_424_48,
        ],
        [
            -0.001_250_60,
            -0.000_509_91,
            0.001_936_09,
            1_222.493_622_01,
            -0.418_972_16,
            -0.288_677_94,
        ],
    ],
    // Uranus
    [
        [
            19.189_164_64,
            0.047_257_44,
            0.772_637_83,
            313.238_104_51,
            170.954_276_30,
            74.016_925_03,
        ],
        [
            -0.001_961_76,
            -0.000_043_97,
            -0.002_429_39,
            428.482_027_85,
            0.408_052_81,
            0.042_405_89,
        ],
    ],
];

/// The Earth-Moon barycentre's row of the same table.
const EM_BARY: [[f64; 6]; 2] = [
    [1.000_002_61, 0.016_711_23, -0.000_015_31, 100.464_571_66, 102.937_681_93, 0.0],
    [0.000_005_62, -0.000_043_92, -0.012_946_68, 35_999.372_449_81, 0.323_273_64, 0.0],
];

/// One planet as seen from the Earth's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Planet {
    pub body: Body,
    /// Unit direction, star table frame (J2000 equatorial), light time
    /// applied (astrometric).
    pub dir: [f64; 3],
    /// Distance from the Earth and from the Sun (au).
    pub distance: f64,
    pub sun_distance: f64,
    /// Sun-planet-Earth angle (degrees).
    pub phase_angle: f64,
    /// Angle from the Sun as seen from the Earth (degrees).
    pub elongation: f64,
    /// Apparent visual magnitude (V).
    pub magnitude: f64,
}

/// The Moon as seen from the Earth's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moon {
    /// Unit direction, star table frame.
    pub dir: [f64; 3],
    /// Distance in Earth equatorial radii (about 56 to 64).
    pub distance_earth_radii: f64,
}

/// Everything for one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    pub planets: [Planet; 6],
    /// The Sun's unit direction from the Earth's centre, star table frame.
    pub sun: [f64; 3],
    pub moon: Moon,
}

/// The Sun, the Moon and the planets at `unix_days` (days since
/// 1970-01-01 00:00 UTC, with the time of day: the moment the star field
/// uses). `None` outside the tables' 1800 to 2050 or for a moment that is
/// not a number.
pub fn sky_at(unix_days: f64) -> Option<Sky> {
    if !unix_days.is_finite() {
        return None;
    }
    let jde = unix_days + UNIX_EPOCH_JD + DELTA_T_SECONDS / 86_400.0;
    if !(FIRST_JD..LAST_JD).contains(&jde) {
        return None;
    }
    let t = (jde - J2000_JD) / 36_525.0;
    let moon_geo = moon_position(t);
    // the Earth: the barycentre less the Moon's share of their distance
    let emb = heliocentric(&EM_BARY, t);
    let earth = sub(emb, scale(moon_geo, 1.0 / (1.0 + EARTH_MOON_MASS_RATIO)));
    let sun = unit(scale(earth, -1.0));
    let mut planets = [Planet {
        body: Body::Mercury,
        dir: [1.0, 0.0, 0.0],
        distance: 1.0,
        sun_distance: 1.0,
        phase_angle: 0.0,
        elongation: 0.0,
        magnitude: 0.0,
    }; 6];
    for (slot, body) in planets.iter_mut().zip(Body::ALL) {
        let el = &ELEMENTS[body.index()];
        // one light-time step: the planet where it was when the light we
        // see left it (at most about 0.5 arcminute for the nearest)
        let first = heliocentric(el, t);
        let tau = norm(sub(first, earth)) * LIGHT_DAYS_PER_AU;
        let p = heliocentric(el, t - tau / 36_525.0);
        let geo = sub(p, earth);
        let distance = norm(geo);
        let sun_distance = norm(p);
        let dir = unit(geo);
        let phase_angle = angle_deg(scale(p, -1.0), scale(geo, -1.0));
        let elongation = angle_deg(dir, sun);
        let magnitude = magnitude(body, t, p, earth, sun_distance, distance, phase_angle);
        *slot = Planet {
            body,
            dir,
            distance,
            sun_distance,
            phase_angle,
            elongation,
            magnitude,
        };
    }
    Some(Sky {
        planets,
        sun,
        moon: Moon {
            dir: unit(moon_geo),
            distance_earth_radii: norm(moon_geo) * AU_KM / EARTH_RADIUS_KM,
        },
    })
}

/// The Moon seen from the ground rather than from the Earth's centre: its
/// geocentric direction already turned into the region's frame (`StarFrame`
/// axes, z up) and its distance in Earth radii. The observer stands one
/// Earth radius up the local vertical, so the Moon drops by up to about a
/// degree near the horizon (its horizontal parallax). The geocentric and
/// geodetic verticals differ by at most 0.2°, which moves the result by
/// under 0.01°.
pub fn topocentric(geo_region: [f64; 3], distance_earth_radii: f64) -> [f64; 3] {
    if !(distance_earth_radii.is_finite() && distance_earth_radii > 1.5) {
        return unit(geo_region);
    }
    let v = scale(unit(geo_region), distance_earth_radii);
    unit([v[0], v[1], v[2] - 1.0])
}

/// The planet in the star buffer's layout (`GpuStar`: unit direction and
/// peak intensity, then linear colour and sigma), f32 like the star table,
/// by the stars' own law (aurora-render stars.rs `intensity`, `point_size`,
/// cap included): a planet and a star of the same magnitude look the same.
pub fn gpu_point(p: &Planet) -> [[f32; 4]; 2] {
    use aurora_render::stars::{intensity, point_size};
    let m = p.magnitude as f32;
    let c = p.body.colour();
    [
        [p.dir[0] as f32, p.dir[1] as f32, p.dir[2] as f32, intensity(m)],
        [c[0], c[1], c[2], point_size(m)],
    ]
}

/// Heliocentric position (au, J2000 equatorial) of a body with these
/// elements, `t` Julian centuries (TT) after J2000: the table's Keplerian
/// steps (this span of it needs no extra terms in the mean anomaly).
fn heliocentric(el: &[[f64; 6]; 2], t: f64) -> [f64; 3] {
    let at = |i: usize| el[0][i] + el[1][i] * t;
    let (a, e, inc, l, peri, node) = (at(0), at(1), at(2), at(3), at(4), at(5));
    let w = peri - node;
    let m = (l - peri + 180.0).rem_euclid(360.0) - 180.0;
    let ea = eccentric_anomaly(m, e).to_radians();
    let xp = a * (ea.cos() - e);
    let yp = a * (1.0 - e * e).max(0.0).sqrt() * ea.sin();
    let (sw, cw) = w.to_radians().sin_cos();
    let (sn, cn) = node.to_radians().sin_cos();
    let (si, ci) = inc.to_radians().sin_cos();
    let x = (cw * cn - sw * sn * ci) * xp + (-sw * cn - cw * sn * ci) * yp;
    let y = (cw * sn + sw * cn * ci) * xp + (-sw * sn + cw * cn * ci) * yp;
    let z = (sw * si) * xp + (cw * si) * yp;
    ecliptic_to_equatorial([x, y, z], OBLIQUITY_J2000)
}

/// Kepler's equation M = E - e* sin E, in degrees, by the table's Newton
/// steps from E0 = M + e* sin M, to 1e-6°. Bounded: the eccentricities here
/// are below 0.21, where it settles in a few steps.
fn eccentric_anomaly(m: f64, e: f64) -> f64 {
    let e_star = e.to_degrees();
    let mut ea = m + e_star * m.to_radians().sin();
    for _ in 0..32 {
        let dm = m - (ea - e_star * ea.to_radians().sin());
        let de = dm / (1.0 - e * ea.to_radians().cos());
        ea += de;
        if de.abs() <= 1e-6 {
            break;
        }
    }
    ea
}

/// The Moon's geocentric position (au, J2000 equatorial), `t` Julian
/// centuries (TT) after J2000: the almanac formula's low-precision
/// series for the ecliptic longitude, latitude and horizontal parallax of
/// date, precessed to the J2000 ecliptic, then to the equator. Its stated
/// rms error is about 0.11°, at most about 0.35°.
fn moon_position(t: f64) -> [f64; 3] {
    let s = |amp: f64, rate: f64, phase: f64| amp * (rate * t + phase).to_radians().sin();
    let c = |amp: f64, rate: f64, phase: f64| amp * (rate * t + phase).to_radians().cos();
    let lambda = 218.32 + 481_267.883 * t + s(6.29, 477_198.85, 134.9) - s(1.27, -413_335.38, 259.2)
        + s(0.66, 890_534.23, 235.7)
        + s(0.21, 954_397.70, 269.9)
        - s(0.19, 35_999.05, 357.5)
        - s(0.11, 966_404.05, 186.6);
    let beta = s(5.13, 483_202.03, 93.3) + s(0.28, 960_400.87, 228.2) - s(0.28, 6_003.18, 318.3) - s(0.17, -407_332.20, 217.6);
    let parallax =
        0.9508 + c(0.0518, 477_198.85, 134.9) + c(0.0095, -413_335.38, 259.2) + c(0.0078, 890_534.23, 235.7) + c(0.0028, 954_397.70, 269.9);
    // precession from the ecliptic and equinox of date to J2000 (Eqs. 5-9)
    let pa = 1.396_971 * t + 0.000_308_6 * t * t;
    let pb = 0.013_056 * t - 0.000_009_2 * t * t;
    let pc = 5.123_62 - 1.155_358 * t - 0.000_196_4 * t * t;
    let beta0 = beta - pb * (lambda + pc).to_radians().sin();
    let lambda0 = lambda - pa + pb * (lambda + pc).to_radians().cos() * beta0.to_radians().tan();
    let r = EARTH_RADIUS_KM / parallax.to_radians().sin() / AU_KM;
    let (sl, cl) = lambda0.to_radians().sin_cos();
    let (sb, cb) = beta0.to_radians().sin_cos();
    ecliptic_to_equatorial([r * cb * cl, r * cb * sl, r * sb], OBLIQUITY_MOON)
}

/// Saturn's north pole of rotation, the rings' axis (fact sheet: RA
/// 40.589 - 0.036 T, Dec 83.537 - 0.004 T, degrees, T centuries from
/// J2000), and Uranus' (RA 257.311, Dec -15.175).
fn saturn_pole(t: f64) -> [f64; 3] {
    radec((40.589 - 0.036 * t).to_radians(), (83.537 - 0.004 * t).to_radians())
}

fn uranus_pole() -> [f64; 3] {
    radec(257.311f64.to_radians(), (-15.175f64).to_radians())
}

/// Uranus' flattening (fact sheet 0.02293; the magnitude model prints
/// 0.0022927, a misplaced digit: either moves V by under 0.002).
const URANUS_FLATTENING: f64 = 0.022_93;

/// Apparent V magnitude (the 2018 magnitude model above). `p` the planet's
/// heliocentric position, `earth` the Earth's (au), `r` and `d` its
/// distances from the Sun and the Earth, `alpha` its phase angle
/// (degrees). Each equation is used inside its published range; beyond
/// the last one (Venus past 179°, Mercury past 170°) the angle is held at
/// that limit so the result stays finite and plausible.
fn magnitude(body: Body, t: f64, p: [f64; 3], earth: [f64; 3], r: f64, d: f64, alpha: f64) -> f64 {
    let base = 5.0 * (r * d).log10();
    let a = alpha;
    match body {
        Body::Mercury => {
            let a = a.min(170.0);
            // Eq. 2
            base - 0.613 + 6.3280e-02 * a - 1.6336e-03 * a.powi(2) + 3.3644e-05 * a.powi(3) - 3.4265e-07 * a.powi(4)
                + 1.6893e-09 * a.powi(5)
                - 3.0334e-12 * a.powi(6)
        }
        Body::Venus => {
            let a = a.min(179.0);
            if a <= 163.7 {
                // Eq. 3
                base - 4.384 - 1.044e-03 * a + 3.687e-04 * a.powi(2) - 2.814e-06 * a.powi(3) + 8.938e-09 * a.powi(4)
            } else {
                // Eq. 4: forward scattering near inferior conjunction
                base + 236.058_28 - 2.819_14 * a + 8.390_34e-03 * a.powi(2)
            }
        }
        Body::Mars => {
            // Eqs. 6 and 7, without the rotation L(λe) and season L(Ls)
            // terms (rms about 0.035 mag, at most 0.06 for the rotation)
            if a <= 50.0 {
                base - 1.601 + 0.022_67 * a - 0.000_130_2 * a.powi(2)
            } else {
                base - 0.367 - 0.025_73 * a + 0.000_344_5 * a.powi(2)
            }
        }
        Body::Jupiter => {
            if a <= 12.0 {
                // Eq. 8
                base - 9.395 - 3.7e-04 * a + 6.16e-04 * a.powi(2)
            } else {
                // Eq. 9 (12° to 130°; never reached from the Earth)
                let x = a.min(130.0) / 180.0;
                let albedo = 1.0 - 1.507 * x - 0.363 * x.powi(2) - 0.062 * x.powi(3) + 2.809 * x.powi(4) - 1.876 * x.powi(5);
                base - 9.428 - 2.5 * albedo.max(1e-6).log10()
            }
        }
        Body::Saturn => {
            // the rings' tilt seen from the Earth and from the Sun
            // (planetocentric latitudes); effective tilt sqrt(bE bS) when
            // they are lit and seen from the same side, else 0 (backlit)
            let pole = saturn_pole(t);
            let be = dot(unit(sub(earth, p)), pole).clamp(-1.0, 1.0).asin();
            let bs = dot(unit(scale(p, -1.0)), pole).clamp(-1.0, 1.0).asin();
            let beta = if be * bs > 0.0 { (be * bs).sqrt() } else { 0.0 };
            if a < 6.5 && beta.to_degrees() < 27.0 {
                // Eq. 10: globe and rings
                base - 8.914 - 1.825 * beta.sin() + 0.026 * a - 0.378 * beta.sin() * (-2.25 * a).exp()
            } else {
                // Eq. 11: the globe alone
                base - 8.95 - 3.7e-04 * a + 6.16e-04 * a.powi(2)
            }
        }
        Body::Uranus => {
            // Eq. 14: the mean of the planetographic sub-Earth and
            // sub-solar latitudes' sizes (Eq. 13 from planetocentric)
            let pole = uranus_pole();
            let graphic = |v: [f64; 3]| {
                let phi = dot(unit(v), pole).clamp(-1.0, 1.0).asin();
                (phi.tan() / (1.0 - URANUS_FLATTENING).powi(2)).atan().to_degrees().abs()
            };
            let phi = 0.5 * (graphic(sub(earth, p)) + graphic(scale(p, -1.0)));
            base - 7.110 - 8.4e-04 * phi
        }
    }
}

fn ecliptic_to_equatorial(v: [f64; 3], obliquity_deg: f64) -> [f64; 3] {
    let (s, c) = obliquity_deg.to_radians().sin_cos();
    [v[0], c * v[1] - s * v[2], s * v[1] + c * v[2]]
}

/// Unit vector of right ascension and declination (radians), the layout of
/// `stars::equatorial`.
pub fn radec(ra: f64, dec: f64) -> [f64; 3] {
    [dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin()]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = norm(a);
    if n > 0.0 && n.is_finite() {
        scale(a, 1.0 / n)
    } else {
        [1.0, 0.0, 0.0]
    }
}

/// Angle between two vectors (degrees), stable at 0 and 180.
pub fn angle_deg(a: [f64; 3], b: [f64; 3]) -> f64 {
    let c = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    norm(c).atan2(dot(a, b)).to_degrees()
}

#[cfg(test)]
#[path = "planets_reference.rs"]
mod reference;
#[cfg(test)]
#[path = "planets_tests.rs"]
mod tests;
