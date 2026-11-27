//! The planets in the real night sky (the Constellations window's Solar
//! system card, World › Environment › Constellations): Mercury, Venus, Mars,
//! Jupiter, Saturn and faint Uranus where they really stand among the
//! stars at the star field's moment, as points drawn like the stars' (the
//! stars' brightness law, gain, extinction, twilight and clouds), steady
//! rather than twinkling, each in a colour of its own. Aurora's own: the
//! Legacy Viewer draws no planets.
//!
//! Also two display-only switches for the region's Sun and Moon: their
//! discs drawn where the real Sun and Moon stand at that moment and place
//! (the Moon seen from the ground, its parallax included), or not drawn at
//! all; hidden wins. Only the discs move: the light, the sky's colour and
//! glow, the shadows, the water's glint, the constellations' darkness gate
//! and the star field's hour all keep the region's sun and moon.
//!
//! The positions come from [`ephemeris`] (1800 to 2050): outside that span
//! the planets and the real discs show nothing, "hidden" still hides. A
//! planet within 18° of the real Sun fades out, gone at 10°, as it is lost
//! in the Sun's glare.
//!
//! Cost: off (or the Legacy Viewer's stars) nothing is computed and the
//! frame's uniform carries a planet count of 0 and discs of mode 0, which
//! the sky shader reads as before (one comparison each). On, the positions
//! are computed every 5 minutes of the sky's moment (every second with a
//! real Sun or Moon), about 10 µs, and placed each frame with the stars'
//! own rotation (`StarFrame::axes`).
//!
//! Test variables (demo only, docs/TESTING.md): `AURORA_DEMO_PLANETS=on|off|names`
//! and `AURORA_DEMO_SKY_DISCS=real|hide` ([`demo_bodies`]).

#[path = "planets_ephemeris.rs"]
pub mod ephemeris;

use aurora_render::constellations::{DISC_HIDDEN, DISC_REAL, PLANET_SLOTS};
use aurora_render::stars::{REAL, StarFrame};
use ephemeris::{Body, Sky};
use glam::{DVec3, Vec3};

/// Days of the sky's moment the planets' positions are kept (5 minutes:
/// the fastest, Mercury, moves under 0.02° in that time).
const PLANETS_EVERY: f64 = 300.0 / 86_400.0;
/// With a real Sun or Moon: one second (the Moon moves 0.008° in it).
const DISCS_EVERY: f64 = 1.0 / 86_400.0;
/// A planet this close to the real Sun (degrees) is gone, from the second
/// one it begins to fade.
const GLARE_GONE: f64 = 10.0;
const GLARE_FROM: f64 = 18.0;

/// What the settings ask of the planets and the discs (four switches;
/// the names show with the planets only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    pub planets: bool,
    pub names: bool,
    /// The Sun's and the Moon's discs where the real bodies stand.
    pub real: bool,
    /// No disc at all (wins over `real`).
    pub hidden: bool,
}

impl Wanted {
    pub fn of(s: &crate::settings::Settings) -> Self {
        Wanted {
            planets: s.night_sky_planets,
            names: s.night_sky_planet_names,
            real: s.night_sky_sun_moon_real,
            hidden: s.night_sky_sun_moon_hidden,
        }
    }

    pub fn store(self, s: &mut crate::settings::Settings) {
        s.night_sky_planets = self.planets;
        s.night_sky_planet_names = self.names;
        s.night_sky_sun_moon_real = self.real;
        s.night_sky_sun_moon_hidden = self.hidden;
    }
}

/// The planets' names for the constellation overlay (world/constellations.rs):
/// per body, in `Body::ALL`'s order, its catalogue direction and opacity
/// (0: not drawn). `on`: the names are wanted and the planets drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Labels {
    pub on: bool,
    pub points: [[f32; 4]; 6],
}

/// What the log last said.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shown {
    Off,
    OutOfRange,
    On { planets: bool, real: bool, hidden: bool },
}

/// The planets' state on the main thread (`App::planets`).
#[derive(Default)]
pub struct Cache {
    /// The sky last computed, and its moment (days since 1970).
    sky: Option<(f64, Sky)>,
    labels: Labels,
    logged: Option<Shown>,
}

/// A planet's opacity against the real Sun's glare: 1 from 18° of
/// elongation, 0 at 10° and closer, smooth between.
pub fn glare_fade(elongation: f64) -> f32 {
    let t = ((elongation - GLARE_GONE) / (GLARE_FROM - GLARE_GONE)).clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

impl Cache {
    /// The planets' names for this frame's overlay.
    pub fn labels(&self) -> &Labels {
        &self.labels
    }

    /// Fills this frame's planets and discs into `stars` (just made by
    /// night_sky.rs `star_frame`, whose axes place them): `wanted` from the
    /// settings, `moment` the star field's (days since 1970-01-01 UTC);
    /// `demo`: the demo's test variables apply.
    pub fn fill(&mut self, stars: &mut StarFrame, wanted: Wanted, moment: f64, demo: bool) {
        let w = if demo { demo_bodies(wanted) } else { wanted };
        self.labels = Labels::default();
        stars.planets = [[0.0; 4]; PLANET_SLOTS];
        stars.discs = [[0.0; 4]; 2];
        let real_discs = w.real && !w.hidden;
        if stars.catalogue != Some(REAL) || !(w.planets || real_discs || w.hidden) {
            self.note(Shown::Off, stars, None);
            return;
        }
        if w.hidden {
            // straight down: the sun's stand-in disc draws nothing there
            stars.discs = [[0.0, 0.0, -1.0, DISC_HIDDEN]; 2];
        }
        if !(w.planets || real_discs) {
            self.note(
                Shown::On {
                    planets: false,
                    real: false,
                    hidden: true,
                },
                stars,
                None,
            );
            return;
        }
        let every = if real_discs { DISCS_EVERY } else { PLANETS_EVERY };
        // (a moment that is not a number is never fresh: sky_at refuses it)
        if !self.sky.as_ref().is_some_and(|(at, _)| (moment - at).abs() <= every) {
            self.sky = ephemeris::sky_at(moment).map(|s| (moment, s));
        }
        let Some((_, sky)) = self.sky else {
            self.note(Shown::OutOfRange, stars, None);
            return;
        };
        if w.planets {
            let mut n = 0;
            for (k, p) in sky.planets.iter().enumerate() {
                let fade = glare_fade(p.elongation);
                let [mut pos, colour] = ephemeris::gpu_point(p);
                self.labels.points[k] = [pos[0], pos[1], pos[2], fade];
                if fade <= 0.0 {
                    continue;
                }
                pos[3] *= fade;
                stars.planets[1 + 2 * n] = pos;
                stars.planets[2 + 2 * n] = colour;
                n += 1;
            }
            stars.planets[0][0] = n as f32;
            self.labels.on = w.names;
        }
        if real_discs {
            let axes = stars.axes.map(|a| a.as_dvec3());
            let region = |v: [f64; 3]| axes[0] * v[0] + axes[1] * v[1] + axes[2] * v[2];
            let sun = region(sky.sun).normalize_or(DVec3::Z);
            // seen from the ground: the star field's latitude is in its axes
            let moon = ephemeris::topocentric(region(sky.moon.dir).to_array(), sky.moon.distance_earth_radii);
            stars.discs = [
                [sun.x as f32, sun.y as f32, sun.z as f32, DISC_REAL],
                [moon[0] as f32, moon[1] as f32, moon[2] as f32, DISC_REAL],
            ];
        }
        let state = Shown::On {
            planets: w.planets,
            real: real_discs,
            hidden: w.hidden,
        };
        self.note(state, stars, Some(sky));
    }

    /// A log line when what shows changes: the bodies' altitude and
    /// azimuth then (for a report: where to look).
    fn note(&mut self, now: Shown, stars: &StarFrame, sky: Option<Sky>) {
        if self.logged == Some(now) {
            return;
        }
        self.logged = Some(now);
        match (now, sky) {
            (Shown::Off, _) => log::info!("night sky: no planets, the region's sun and moon discs"),
            (Shown::OutOfRange, _) => log::info!("night sky: the sky's moment is outside 1800 to 2050: no planets, no real sun and moon"),
            (Shown::On { planets, real, hidden }, sky) => {
                let place = |d: Vec3| {
                    let d = d.normalize_or(Vec3::Z);
                    format!(
                        "{:.0}° up, azimuth {:.0}°",
                        d.z.clamp(-1.0, 1.0).asin().to_degrees(),
                        d.x.atan2(d.y).to_degrees().rem_euclid(360.0)
                    )
                };
                let region = |v: [f64; 3]| stars.axes[0] * v[0] as f32 + stars.axes[1] * v[1] as f32 + stars.axes[2] * v[2] as f32;
                let mut parts = Vec::new();
                if let (true, Some(sky)) = (planets, sky) {
                    for p in &sky.planets {
                        parts.push(format!("{} {} (V {:.1})", p.body.id(), place(region(p.dir)), p.magnitude));
                    }
                }
                if real {
                    parts.push(format!("real sun {}", place(Vec3::from_slice(&stars.discs[0][..3]))));
                    parts.push(format!("real moon {}", place(Vec3::from_slice(&stars.discs[1][..3]))));
                }
                if hidden {
                    parts.push("the sun and moon discs hidden".to_owned());
                }
                log::info!("night sky: {}", parts.join(", "));
            }
        }
    }
}

/// `AURORA_DEMO_PLANETS=on|off|names` (names: on, with their names) and
/// `AURORA_DEMO_SKY_DISCS=<items>` (`real`, `hide`, comma separated; an
/// empty value: neither): the planets' and the Sun's and Moon's switches
/// for this launch, in memory only. Unset: as the settings say.
pub fn demo_bodies(settings: Wanted) -> Wanted {
    static BODIES: std::sync::OnceLock<(Option<String>, Option<String>)> = std::sync::OnceLock::new();
    let (planets, discs) = BODIES.get_or_init(|| {
        (
            std::env::var("AURORA_DEMO_PLANETS").ok(),
            std::env::var("AURORA_DEMO_SKY_DISCS").ok(),
        )
    });
    parse_bodies(settings, planets.as_deref(), discs.as_deref())
}

fn parse_bodies(mut w: Wanted, planets: Option<&str>, discs: Option<&str>) -> Wanted {
    match planets.map(str::trim) {
        Some("on" | "1") => (w.planets, w.names) = (true, false),
        Some("names") => (w.planets, w.names) = (true, true),
        Some("off" | "0") => (w.planets, w.names) = (false, false),
        _ => {}
    }
    if let Some(items) = discs {
        let items: Vec<&str> = items.split(',').map(str::trim).collect();
        w.real = items.contains(&"real");
        w.hidden = items.contains(&"hide");
    }
    w
}

/// A body's built-in name, when the data names none: its catalogue key
/// (English, French and Japanese; the other languages show the English).
pub fn builtin_name(body: Body) -> &'static str {
    match body {
        Body::Mercury => "planet-mercury",
        Body::Venus => "planet-venus",
        Body::Mars => "planet-mars",
        Body::Jupiter => "planet-jupiter",
        Body::Saturn => "planet-saturn",
        Body::Uranus => "planet-uranus",
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::world::night_sky::{DEFAULT_GAIN, DEFAULT_MILKY_WAY, NightSky, Observer, star_frame};

    /// 2020-12-21 17:00 UTC: Jupiter and Saturn together, low in the
    /// south-west of Paris, the Sun just set.
    const CONJUNCTION: f64 = 18617.0 + 17.0 / 24.0;

    fn frame(mode: NightSky, moment: f64) -> StarFrame {
        let at = Observer {
            latitude: crate::world::night_sky::LATITUDE as f32,
            longitude: crate::world::night_sky::LONGITUDE as f32,
            hour: crate::world::night_sky::StarHour::RealTime,
        };
        star_frame(mode, at, DEFAULT_GAIN, DEFAULT_MILKY_WAY, Vec3::NEG_Z, moment, 0.0)
    }

    const ALL: Wanted = Wanted {
        planets: true,
        names: true,
        real: true,
        hidden: false,
    };

    #[test]
    fn the_planets_and_discs_variables_override_the_settings_only_when_set() {
        let s = Wanted {
            planets: true,
            names: false,
            real: false,
            hidden: true,
        };
        assert_eq!(parse_bodies(s, None, None), s);
        assert_eq!(parse_bodies(s, Some("off"), None), Wanted { planets: false, ..s });
        assert_eq!(parse_bodies(s, Some("names"), None), Wanted { names: true, ..s });
        assert_eq!(parse_bodies(s, Some("stars"), None), s);
        let both = parse_bodies(s, None, Some("real, hide"));
        assert!(both.real && both.hidden);
        let real = parse_bodies(s, None, Some("real"));
        assert!(real.real && !real.hidden);
        let none = parse_bodies(s, None, Some(""));
        assert!(!none.real && !none.hidden);
    }

    #[test]
    fn off_or_legacy_or_out_of_range_leaves_the_frame_as_before() {
        let before = frame(NightSky::Real, CONJUNCTION);
        let mut c = Cache::default();
        let mut f = before;
        c.fill(&mut f, Wanted::default(), CONJUNCTION, false);
        assert_eq!(f, before, "all off: the frame of before");
        assert_eq!(f.planets[0][0], 0.0);
        assert!(f.discs.iter().all(|d| d[3] == 0.0));
        assert!(!c.labels().on);
        // the Legacy Viewer's stars: nothing, whatever is wanted
        let mut f = frame(NightSky::Legacy, CONJUNCTION);
        c.fill(&mut f, Wanted { hidden: true, ..ALL }, CONJUNCTION, false);
        assert_eq!(f, frame(NightSky::Legacy, CONJUNCTION));
        // outside 1800 to 2050: no planets, no real discs, but hidden hides
        let far = 30_000.0; // 2052
        let mut f = frame(NightSky::Real, far);
        c.fill(&mut f, ALL, far, false);
        assert_eq!(f.planets[0][0], 0.0);
        assert!(f.discs.iter().all(|d| d[3] == 0.0));
        assert!(!c.labels().on);
        c.fill(&mut f, Wanted { hidden: true, ..ALL }, far, false);
        assert!(f.discs.iter().all(|d| d[3] == DISC_HIDDEN && d[2] < -0.9));
        assert_eq!(f.planets[0][0], 0.0);
        let mut f = frame(NightSky::Real, CONJUNCTION);
        c.fill(&mut f, ALL, f64::NAN, false);
        assert_eq!(f.planets[0][0], 0.0);
    }

    #[test]
    fn the_planets_stand_where_the_star_field_puts_them() {
        let mut c = Cache::default();
        let mut f = frame(NightSky::Real, CONJUNCTION);
        c.fill(
            &mut f,
            Wanted {
                planets: true,
                ..Default::default()
            },
            CONJUNCTION,
            false,
        );
        let n = f.planets[0][0] as usize;
        assert!((1..=6).contains(&n), "{n}");
        let sky = ephemeris::sky_at(CONJUNCTION).expect("in range");
        let region = |v: [f32; 3]| f.axes[0] * v[0] + f.axes[1] * v[1] + f.axes[2] * v[2];
        // Jupiter (index 3) and Saturn (4): about 0.1° apart, low in the south-west
        let jupiter = region([
            sky.planets[3].dir[0] as f32,
            sky.planets[3].dir[1] as f32,
            sky.planets[3].dir[2] as f32,
        ]);
        let saturn = region([
            sky.planets[4].dir[0] as f32,
            sky.planets[4].dir[1] as f32,
            sky.planets[4].dir[2] as f32,
        ]);
        assert!(jupiter.angle_between(saturn).to_degrees() < 0.3);
        let altitude = jupiter.z.asin().to_degrees();
        let azimuth = jupiter.x.atan2(jupiter.y).to_degrees().rem_euclid(360.0);
        assert!((5.0..15.0).contains(&altitude), "{altitude}");
        assert!((210.0..235.0).contains(&azimuth), "{azimuth}");
        // the points are the shader's: unit catalogue directions, the
        // stars' brightness law, each planet's colour
        for k in 0..n {
            let pos = f.planets[1 + 2 * k];
            let col = f.planets[2 + 2 * k];
            assert!((Vec3::from_slice(&pos[..3]).length() - 1.0).abs() < 1e-4);
            assert!(pos[3] > 0.0 && pos[3] <= 3.0 && col[3] >= 0.6);
        }
        // the names: only with their switch
        assert!(!c.labels().on);
        c.fill(
            &mut f,
            Wanted {
                planets: true,
                names: true,
                ..Default::default()
            },
            CONJUNCTION,
            false,
        );
        assert!(c.labels().on);
        assert_eq!(c.labels().points[3][3], 1.0, "Jupiter, 30° from the Sun: full");
    }

    #[test]
    fn a_planet_fades_into_the_suns_glare() {
        assert_eq!(glare_fade(5.0), 0.0);
        assert_eq!(glare_fade(10.0), 0.0);
        assert!((glare_fade(14.0) - 0.5).abs() < 1e-6);
        assert_eq!(glare_fade(18.0), 1.0);
        assert_eq!(glare_fade(90.0), 1.0);
        // 2020-06-03: Venus at inferior conjunction, 0.3° from the Sun: left out
        let moment = 18416.0;
        let mut c = Cache::default();
        let mut f = frame(NightSky::Real, moment);
        c.fill(
            &mut f,
            Wanted {
                planets: true,
                names: true,
                ..Default::default()
            },
            moment,
            false,
        );
        assert_eq!(c.labels().points[1][3], 0.0);
        let n = f.planets[0][0] as usize;
        let venus = ephemeris::gpu_point(&ephemeris::sky_at(moment).expect("in range").planets[1])[0];
        assert!((0..n).all(|k| f.planets[1 + 2 * k][..3] != venus[..3]));
    }

    #[test]
    fn the_real_sun_and_moon_take_the_star_fields_place_and_hidden_wins() {
        let mut c = Cache::default();
        let mut f = frame(NightSky::Real, CONJUNCTION);
        c.fill(
            &mut f,
            Wanted {
                real: true,
                ..Default::default()
            },
            CONJUNCTION,
            false,
        );
        assert_eq!(f.planets[0][0], 0.0, "no planets asked");
        let [sun, moon] = f.discs;
        assert!(sun[3] == DISC_REAL && moon[3] == DISC_REAL);
        // the real Sun at 17:00 UTC on the solstice, seen from Paris: about
        // 10° under the horizon, south-west
        let sun = Vec3::from_slice(&sun[..3]);
        assert!((sun.length() - 1.0).abs() < 1e-4);
        let altitude = sun.z.asin().to_degrees();
        assert!((-14.0..-6.0).contains(&altitude), "{altitude}");
        assert!(sun.x < 0.0 && sun.y < 0.0, "{sun}");
        // the Moon seen from the ground is lower than from the Earth's centre
        let sky = ephemeris::sky_at(CONJUNCTION).expect("in range");
        let geo = f.axes[0] * sky.moon.dir[0] as f32 + f.axes[1] * sky.moon.dir[1] as f32 + f.axes[2] * sky.moon.dir[2] as f32;
        let topo = Vec3::from_slice(&moon[..3]);
        assert!(topo.z < geo.z && topo.angle_between(geo).to_degrees() < 1.1);
        // hidden wins over real
        c.fill(
            &mut f,
            Wanted {
                real: true,
                hidden: true,
                ..Default::default()
            },
            CONJUNCTION,
            false,
        );
        assert!(f.discs.iter().all(|d| d[3] == DISC_HIDDEN));
    }

    #[test]
    fn the_positions_are_kept_for_five_minutes_or_a_second_with_real_discs() {
        let mut c = Cache::default();
        let mut f = frame(NightSky::Real, CONJUNCTION);
        let planets = Wanted {
            planets: true,
            ..Default::default()
        };
        c.fill(&mut f, planets, CONJUNCTION, false);
        let first = c.sky.expect("computed").0;
        c.fill(&mut f, planets, CONJUNCTION + 200.0 / 86_400.0, false);
        assert_eq!(c.sky.expect("kept").0, first, "under 5 minutes: kept");
        c.fill(&mut f, planets, CONJUNCTION + 400.0 / 86_400.0, false);
        assert_ne!(c.sky.expect("again").0, first, "past 5 minutes: again");
        let at = c.sky.expect("computed").0;
        c.fill(&mut f, Wanted { real: true, ..planets }, at + 2.0 / 86_400.0, false);
        assert_ne!(c.sky.expect("again").0, at, "a real disc: every second");
    }

    #[test]
    fn the_switches_round_trip_through_the_settings() {
        let d = crate::settings::Settings::default();
        // all off by default: the real sky as before
        assert_eq!(Wanted::of(&d), Wanted::default());
        let mut s = d.clone();
        let w = Wanted {
            planets: false,
            names: true,
            real: true,
            hidden: true,
        };
        w.store(&mut s);
        assert_eq!(Wanted::of(&s), w);
        // an older file without the keys reads as the defaults
        let old: crate::settings::Settings = serde_json::from_str("{}").expect("an empty file reads");
        assert_eq!(Wanted::of(&old), Wanted::of(&d));
    }
}
