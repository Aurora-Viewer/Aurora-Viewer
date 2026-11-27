//! The constellations over the real night sky (World › Environment ›
//! Constellations): engraved figures drawing themselves like
//! a pen stroke as they come into view, their stick lines from star to
//! star, and their names. A figure no art set draws has its lines and
//! name; figures no longer counted have a switch of their own, off by
//! default. Aurora's own: the Legacy Viewer draws no constellations.
//!
//! The data is the user's, installed in their profile
//! (`constellations/` in the configuration folder, constellations_catalogue.rs)
//! and read at run time, on demand: nothing of it is built into the viewer,
//! nothing is read from the network or written to the grid. The sky shader
//! draws them next to the stars (aurora-render constellations.rs, env.wgsl
//! `constellation_overlay`), so they fade with the stars at twilight and
//! towards the horizon, hide behind clouds, the sun and the moon, and show
//! in photos (names included); not in the water's or the mirrors'
//! reflections nor in the reflection probes.
//!
//! What runs: nothing while the overlay is off, by day (the sun less than
//! the darkness setting under the horizon) or with the Legacy Viewer's
//! stars: a few comparisons a frame, no file read, no texture read and
//! nothing sent to the GPU. The folder's small files (the JSON) are read on
//! a thread of their own the first time the overlay or its settings need
//! them, and again on "Rescan" (only when a file changed). A drawing's
//! picture is read and decoded on the overlay's worker the first time its
//! figure shows, then goes into the atlas in bands of rows over a few
//! frames (constellations_atlas.rs); until it is whole the figure shows its
//! lines and name, and the drawing's pen stroke then begins from nothing.
//! The atlas lets the least recently drawn pictures go when it is full,
//! and all of them `RELEASE_AFTER` seconds after nothing showed: they are
//! read again when needed.
//!
//! Test variables: `AURORA_CONSTELLATIONS=0|1` turns the overlay off or on
//! for a launch (an A/B with the same settings);
//! `AURORA_CONSTELLATION_DIR=<folder>` reads the data from that folder;
//! `AURORA_DEMO_CONSTELLATIONS` (demo only) sets it up and can hold it at
//! a stage of its drawing or fading (docs/TESTING.md).

#[path = "constellations_atlas.rs"]
mod atlas;
#[path = "constellations_catalogue.rs"]
pub mod catalogue;
#[path = "constellations_credits.rs"]
pub mod credits;
#[path = "constellations_names.rs"]
mod names;
#[path = "constellations_planets.rs"]
pub mod planet_names;
#[path = "constellations_readings.rs"]
mod readings;
#[path = "constellations_state.rs"]
mod state;

use aurora_render::constellations::{
    Bases, DrawingGeom, FigureGeom, FrameState, Item, LINE_REACH, MAX_FIGURES, MAX_LAYERS, MAX_REFS, NAME_REACH, Overlay, Upload,
    build_tables_of, pack_place, tangent_frame,
};
use catalogue::{Catalogue, Kind, chosen};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

/// Seconds with nothing shown before the atlas's memory is let go.
const RELEASE_AFTER: f32 = 30.0;
/// Names put into the atlas per frame.
const NAMES_PER_FRAME: usize = 8;
/// Pictures given their room in the atlas per frame.
const PLACED_PER_FRAME: usize = 4;
/// Seconds between the demo's readings of the data folder (`rescan`).
const DEMO_RESCAN: f64 = 3.0;
/// Picture bytes (all mip levels) sent to the atlas per frame: a large
/// drawing goes up over a few frames, never in one long write.
const BAND_BYTES: usize = 1 << 20;
/// Names drawn into pictures per frame (the interface's fonts, on this
/// thread): a culture of hundreds of names never takes one long frame.
const NAMES_DRAWN_PER_FRAME: usize = 24;
/// Figures listed beyond which the atlas may take more than
/// `atlas::FEW_LAYERS` (one culture of the data holds fewer).
const FEW_FIGURES: usize = 128;
/// The names waiting for room in the atlas, last frame: the window says it.
static NAMES_WAITING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub const DRAW_RANGE: std::ops::RangeInclusive<f32> = 0.5..=20.0;
pub const FADE_RANGE: std::ops::RangeInclusive<f32> = 0.5..=10.0;
pub const FIELD_RANGE: std::ops::RangeInclusive<f32> = 10.0..=180.0;
pub const DARK_RANGE: std::ops::RangeInclusive<f32> = 0.0..=18.0;
pub const FONT_RANGE: std::ops::RangeInclusive<f32> = 5.0..=36.0;
pub const OPACITY_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;
pub const WIDTH_RANGE: std::ops::RangeInclusive<f32> = 0.1..=3.0;
pub const BRIGHTNESS_RANGE: std::ops::RangeInclusive<f32> = 1.0..=2.0;
pub const SPARKLE_SIZE_RANGE: std::ops::RangeInclusive<f32> = 1.0..=5.0;
pub const SPARKLE_SPIN_RANGE: std::ops::RangeInclusive<f32> = 0.0..=5.0;

/// The sparkles' shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SparkleShape {
    /// A soft round glow with a cross star through it.
    #[default]
    CircleCross,
    /// The cross star alone.
    Cross,
    /// A five-pointed star.
    Star,
}

impl SparkleShape {
    pub const ALL: [SparkleShape; 3] = [SparkleShape::CircleCross, SparkleShape::Cross, SparkleShape::Star];

    /// The shader's number (aurora-render constellations.rs `SHAPE_*`).
    fn code(self) -> f32 {
        use aurora_render::constellations::{SHAPE_CIRCLE_CROSS, SHAPE_CROSS, SHAPE_STAR};
        match self {
            SparkleShape::CircleCross => SHAPE_CIRCLE_CROSS,
            SparkleShape::Cross => SHAPE_CROSS,
            SparkleShape::Star => SHAPE_STAR,
        }
    }
}

/// The overlay's settings (`Settings::constellations`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ConstellationSettings {
    /// The overlay at all (lines, and what the switches below add).
    pub enabled: bool,
    /// The engraved figures' opacity, 0 to 1 (0: their pictures are
    /// neither read nor drawn).
    pub drawing_opacity: f32,
    /// The constellation set (culture) shown, one at a time: a list of one
    /// id once picked (`catalogue::shown_culture`: the first id the data
    /// has). `None`: the western one. Every art set of the culture shown is
    /// used, in their order.
    pub cultures: Option<Vec<String>>,
    /// The stick lines.
    pub lines: bool,
    /// The names.
    pub names: bool,
    /// The figures no longer counted.
    pub obsolete: bool,
    /// Seconds a figure takes to draw itself.
    pub draw_seconds: f32,
    /// Seconds a figure takes to fade out once it leaves the field.
    pub fade_seconds: f32,
    /// Half-angle (degrees) of the field around the view where figures show.
    pub field_degrees: f32,
    /// Degrees the sun must be under the horizon before they show.
    pub sun_below_degrees: f32,
    /// The names' size (points of the interface).
    pub font_size: f32,
    /// The lines' width, times the default (1.4 pixels at the view's
    /// centre); below a pixel they fade instead of thinning ([`line_look`]).
    pub line_width: f32,
    /// While a figure draws itself, its lines and picture are this much
    /// brighter, easing back once each is drawn (state::drawing_gain).
    pub draw_brightness: f32,
    /// A picture, a figure's lines and a name each give one soft pulse of
    /// light when they finish appearing (state::pulse).
    pub highlight: bool,
    /// Each constellation in a light pastel colour of its own, the same
    /// every time ([`pastel`]), for its lines, picture and name.
    pub colour_variation: bool,
    /// A small star riding the pen tip of each line while it draws, fading
    /// once the line is drawn; its size (x1: four times the line's width, the
    /// width setting included), shape and turns while a line draws from
    /// its start to its end.
    pub sparkles: bool,
    pub sparkle_size: f32,
    pub sparkle_shape: SparkleShape,
    pub sparkle_spin: f32,
}

impl Default for ConstellationSettings {
    fn default() -> Self {
        ConstellationSettings {
            enabled: true,
            drawing_opacity: 1.0,
            cultures: None,
            lines: true,
            names: true,
            obsolete: false,
            draw_seconds: 4.0,
            fade_seconds: 3.0,
            field_degrees: 50.0,
            sun_below_degrees: 8.0,
            font_size: 16.0,
            line_width: 1.0,
            draw_brightness: 1.0,
            highlight: true,
            colour_variation: false,
            sparkles: false,
            sparkle_size: 1.0,
            sparkle_shape: SparkleShape::CircleCross,
            sparkle_spin: 1.0,
        }
    }
}

impl ConstellationSettings {
    /// Within their ranges (a hand-edited settings file).
    pub fn sane(self) -> Self {
        use crate::world::night_sky::within;
        let d = ConstellationSettings::default();
        ConstellationSettings {
            drawing_opacity: within(self.drawing_opacity, OPACITY_RANGE, d.drawing_opacity),
            draw_seconds: within(self.draw_seconds, DRAW_RANGE, d.draw_seconds),
            fade_seconds: within(self.fade_seconds, FADE_RANGE, d.fade_seconds),
            field_degrees: within(self.field_degrees, FIELD_RANGE, d.field_degrees),
            sun_below_degrees: within(self.sun_below_degrees, DARK_RANGE, d.sun_below_degrees),
            font_size: within(self.font_size, FONT_RANGE, d.font_size),
            line_width: within(self.line_width, WIDTH_RANGE, d.line_width),
            draw_brightness: within(self.draw_brightness, BRIGHTNESS_RANGE, d.draw_brightness),
            sparkle_size: within(self.sparkle_size, SPARKLE_SIZE_RANGE, d.sparkle_size),
            sparkle_spin: within(self.sparkle_spin, SPARKLE_SPIN_RANGE, d.sparkle_spin),
            ..self
        }
    }

    /// The engraved figures show at all (their pictures are read).
    pub fn drawings_on(&self) -> bool {
        self.drawing_opacity > 0.0
    }
}

/// The lines' half-width (radians) and opacity for a pixel of `pixel`
/// radians and the width setting: 0.7 pixel times the setting, as before
/// it; never under half a pixel, where a thinner line would break into
/// dashes between pixel centres: its light goes down instead, in step with
/// the width it would have had.
pub fn line_look(pixel: f32, width: f32) -> (f32, f32) {
    let half = 0.7 * pixel * width.max(0.0);
    let floor = 0.5 * pixel;
    if half >= floor {
        (half, 1.0)
    } else {
        (floor, half / floor.max(f32::MIN_POSITIVE))
    }
}

/// A sparkle's radius (radians) and light: its size x1 is four times the
/// line's width (the width setting included), so its radius is twice the
/// line's whole width times the size; never under a pixel (it fades instead, as
/// thin lines do), never past the cells its lines are listed in
/// (aurora-render SPARKLE_REACH).
pub fn sparkle_look(pixel: f32, width: f32, size: f32) -> (f32, f32) {
    let radius = 4.0 * 0.7 * pixel * width.max(0.0) * size.max(0.0);
    let (radius, light) = if radius >= pixel {
        (radius, 1.0)
    } else {
        (pixel, radius / pixel.max(f32::MIN_POSITIVE))
    };
    (radius.min(0.9 * aurora_render::constellations::SPARKLE_REACH), light)
}

/// 32-bit FNV-1a: a fixed hash, the same on every build and machine (the
/// standard library's hasher is seeded per process).
pub fn fnv1a(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(*b)).wrapping_mul(0x0100_0193))
}

/// MurmurHash3's 32-bit finalizer: every bit of `h` reaches every bit of
/// the result. FNV-1a alone, on ids of three letters, leaves its top bits
/// clustered (half the 88 western hues fell in a third of the circle).
fn mix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

/// A constellation's own light pastel colour (linear RGB, its brightest
/// channel 1, as the overlay's colours): a hue from the hash of its
/// culture and id (FNV-1a, then [`mix32`]), uniform round the circle, at a
/// high lightness (0.84) and a low saturation (0.55) in HSL.
pub fn pastel(culture: &str, id: &str) -> [f32; 3] {
    let mut key = Vec::with_capacity(culture.len() + id.len() + 1);
    key.extend_from_slice(culture.as_bytes());
    key.push(0);
    key.extend_from_slice(id.as_bytes());
    let hue = f64::from(mix32(fnv1a(&key))) / 4_294_967_296.0 * 6.0;
    let (l, s) = (0.84f64, 0.55f64);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (hue % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let rgb = [r + m, g + m, b + m];
    let top = rgb.iter().copied().fold(f64::MIN, f64::max).max(1e-6);
    rgb.map(|v| (v / top) as f32)
}

/// A demo's hold of every figure in the field at one stage.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hold {
    /// Drawn this far (0 to 1 of the stroke).
    Reveal(f32),
    /// Fully drawn, faded to this opacity.
    Fade(f32),
    /// Fully drawn, this many seconds after each part finished: the
    /// completion pulse held at one phase (its peak at half its length).
    Pulse(f32),
}

/// The test variables of this launch (never written to the settings).
#[derive(Clone, Debug, Default, PartialEq)]
struct Overrides {
    enabled: Option<bool>,
    opacity: Option<f32>,
    /// The culture shown for the launch; the language the names are
    /// read in (the demo's check of the fonts on any script).
    cultures: Option<Vec<String>>,
    name_language: Option<String>,
    lines: Option<bool>,
    zenith: Option<String>,
    names: Option<bool>,
    obsolete: Option<bool>,
    field: Option<f32>,
    font: Option<f32>,
    width: Option<f32>,
    brightness: Option<f32>,
    highlight: Option<bool>,
    colours: Option<bool>,
    sparkles: Option<SparkleShape>,
    sparkle_size: Option<f32>,
    spin: Option<f32>,
    /// The demo's comparison of the shapes: each figure one of the three.
    sparkle_mix: bool,
    hold: Option<Hold>,
    /// The demo reads the data folder again every few seconds (a set
    /// dropped in while it runs, without the Rescan button).
    rescan: bool,
}

impl Overrides {
    fn from_env(demo: bool) -> Self {
        let ab = std::env::var("AURORA_CONSTELLATIONS").ok();
        let scenario = demo.then(|| std::env::var("AURORA_DEMO_CONSTELLATIONS").ok()).flatten();
        parse_overrides(ab.as_deref(), scenario.as_deref())
    }

    fn apply(&self, s: &ConstellationSettings) -> ConstellationSettings {
        let mut s = ConstellationSettings {
            enabled: self.enabled.unwrap_or(s.enabled),
            drawing_opacity: self.opacity.unwrap_or(s.drawing_opacity),
            cultures: self.cultures.clone().map(Some).unwrap_or_else(|| s.cultures.clone()),
            lines: self.lines.unwrap_or(s.lines),
            names: self.names.unwrap_or(s.names),
            obsolete: self.obsolete.unwrap_or(s.obsolete),
            field_degrees: self.field.unwrap_or(s.field_degrees),
            font_size: self.font.unwrap_or(s.font_size),
            line_width: self.width.unwrap_or(s.line_width),
            draw_brightness: self.brightness.unwrap_or(s.draw_brightness),
            highlight: self.highlight.unwrap_or(s.highlight),
            colour_variation: self.colours.unwrap_or(s.colour_variation),
            sparkles: self.sparkles.is_some() || self.sparkle_mix || s.sparkles,
            sparkle_shape: self.sparkles.unwrap_or(s.sparkle_shape),
            sparkle_size: self.sparkle_size.unwrap_or(s.sparkle_size),
            sparkle_spin: self.spin.unwrap_or(s.sparkle_spin),
            ..s.clone()
        };
        s = s.sane();
        s
    }
}

/// `AURORA_CONSTELLATIONS=0|1`, then `AURORA_DEMO_CONSTELLATIONS=<items>`,
/// comma separated: `on`, `reveal=<0..1>`, `fade=<0..1>`, `obsolete`,
/// `nodrawing`, `nolines`, `nonames`, `field=<degrees>`, `font=<points>`,
/// `zenith=<figure id>` (the real sky turned
/// so that this figure's label point is overhead: [`demo_zenith`]),
/// `rescan` (the data folder read again every `DEMO_RESCAN` seconds),
/// `figures=<0..1>` (the engraved figures' opacity), `width=<x>`,
/// `bright=<x>` (the drawing brightness), `highlight=0|1`, `pulse=<s>`
/// (every part held that long after it finished: the pulse's phase),
/// `colours`, `sparkles=circle|cross|star|mix` (mix: each figure one of
/// the three), `sparkle_size=<x>`, `spin=<turns>`,
/// `cultures=<id>` (the culture shown; an older `<id>+<id>` shows the
/// first the data has, as a stored list),
/// `name_language=<code>` (the figures' names read in that language: `ko`
/// shows Hangul, which no interface language picks). The
/// demo's scenario turns the overlay on unless the A/B variable says 0.
fn parse_overrides(ab: Option<&str>, scenario: Option<&str>) -> Overrides {
    let mut o = Overrides::default();
    if let Some(v) = scenario {
        o.enabled = Some(true);
        for item in v.split(',').map(str::trim) {
            let (k, value) = item.split_once('=').unwrap_or((item, ""));
            let number = value.trim().parse::<f32>().ok().filter(|v| v.is_finite());
            match k.trim() {
                "reveal" => o.hold = number.map(|p| Hold::Reveal(p.clamp(0.0, 1.0))),
                "fade" => o.hold = number.map(|a| Hold::Fade(a.clamp(0.0, 1.0))),
                "pulse" => o.hold = number.map(|t| Hold::Pulse(t.max(0.0))),
                "figures" => o.opacity = number,
                "width" => o.width = number,
                "bright" => o.brightness = number,
                "highlight" => o.highlight = Some(value.trim() != "0"),
                "colours" => o.colours = Some(true),
                "sparkles" => match value.trim() {
                    "cross" => o.sparkles = Some(SparkleShape::Cross),
                    "star" => o.sparkles = Some(SparkleShape::Star),
                    "mix" => o.sparkle_mix = true,
                    _ => o.sparkles = Some(SparkleShape::CircleCross),
                },
                "sparkle_size" => o.sparkle_size = number,
                "spin" => o.spin = number,
                "obsolete" => o.obsolete = Some(true),
                "nodrawing" => o.opacity = Some(0.0),
                "nonames" => o.names = Some(false),
                "field" => o.field = number,
                "font" => o.font = number,
                "cultures" => {
                    o.cultures = Some(
                        value
                            .split('+')
                            .map(str::trim)
                            .filter(|v| !v.is_empty())
                            .map(str::to_owned)
                            .collect(),
                    )
                }
                "name_language" => o.name_language = Some(value.trim().to_owned()).filter(|v| !v.is_empty()),
                "nolines" => o.lines = Some(false),
                "zenith" => o.zenith = Some(value.trim().to_owned()),
                "rescan" => o.rescan = true,
                _ => {}
            }
        }
    }
    match ab.map(str::trim) {
        Some("0") => o.enabled = Some(false),
        Some("1") => o.enabled = Some(true),
        _ => {}
    }
    o
}

/// What a frame tells the overlay.
pub struct Input<'a> {
    pub settings: &'a ConstellationSettings,
    /// The night sky shows the real stars.
    pub real_sky: bool,
    /// The stars' frame (its catalogue-to-region axes).
    pub stars: &'a aurora_render::stars::StarFrame,
    /// The region's sun (z up).
    pub sun: Vec3,
    /// Where the camera looks (region frame), its vertical field (radians)
    /// and the window's height (pixels).
    pub look: Vec3,
    pub fov_y: f32,
    pub window_height: u32,
    /// The frame's time (0 while a photo holds its instant).
    pub dt: f32,
    /// The planets' names wanted, and where (world/planets.rs).
    pub planets: &'a crate::world::planets::Labels,
}

/// The catalogue as the overlay uses it, made on the scan's thread.
pub struct Loaded {
    catalogue: Catalogue,
    /// The tables' input, for each choice of cultures (`View`): every
    /// figure's shared geometry and every drawing's, by their numbers.
    geoms: Vec<FigureGeom>,
    drawing_geoms: Vec<DrawingGeom>,
    /// Per culture, its figures' numbers (the catalogue holds them culture
    /// by culture), up to `MAX_FIGURES`.
    ranges: Vec<std::ops::Range<usize>>,
    /// Per figure, the points the field is measured to (catalogue frame):
    /// its centre and its stars.
    points: Vec<Vec<Vec3>>,
    /// Per figure, the number of its first drawing in the tables (its
    /// drawings follow in its order); per drawing, its figure.
    first: Vec<u32>,
    figure_of: Vec<u32>,
    /// Per figure, its colour with the colour variation (`pack_tint`).
    tints: Vec<u32>,
}

/// A picture to read and decode: its drawing, file and size, and the
/// catalogue it belongs to (an answer for an older one is dropped).
struct Job {
    drawing: u32,
    path: PathBuf,
    size: [u32; 2],
    generation: u64,
}

struct Done {
    drawing: u32,
    generation: u64,
    result: Result<Vec<Vec<u8>>, String>,
    /// Reading and decoding (milliseconds).
    millis: f64,
}

struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

impl Worker {
    fn start() -> Option<Worker> {
        let (jobs, rx) = std::sync::mpsc::channel::<Job>();
        let (tx, done) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("constellations".into()).spawn(move || {
            while let Ok(job) = rx.recv() {
                let t = std::time::Instant::now();
                let result = catalogue::read_capped(&job.path, catalogue::MAX_WEBP).and_then(|webp| atlas::decode(&webp, job.size));
                let out = Done {
                    drawing: job.drawing,
                    generation: job.generation,
                    result,
                    millis: t.elapsed().as_secs_f64() * 1000.0,
                };
                if tx.send(out).is_err() {
                    break;
                }
            }
        });
        match spawned {
            Ok(_) => Some(Worker { jobs, done }),
            Err(e) => {
                log::error!("constellations: no worker thread: {e}");
                None
            }
        }
    }
}

fn load(catalogue: Catalogue) -> Loaded {
    let geoms: Vec<_> = catalogue.figures.iter().map(|f| f.geom.clone()).collect();
    let mut drawings = Vec::new();
    let mut first = Vec::with_capacity(catalogue.figures.len());
    let mut figure_of = Vec::new();
    for (i, f) in catalogue.figures.iter().enumerate() {
        first.push(drawings.len() as u32);
        for d in &f.drawings {
            figure_of.push(i as u32);
            drawings.push(DrawingGeom {
                figure: i as u32,
                centre: d.centre,
                picture: d.picture,
                label: d.label,
            });
        }
    }
    // the tables wait for the cultures to draw (`View`)
    let mut ranges = vec![0..0; catalogue.cultures.len()];
    for (i, f) in catalogue.figures.iter().enumerate().take(MAX_FIGURES) {
        if let Some(r) = ranges.get_mut(f.culture) {
            *r = if r.start == r.end { i..i + 1 } else { r.start..i + 1 };
        }
    }
    let points = geoms
        .iter()
        .map(|g| {
            let mut p = vec![tangent_frame(g.centre[0], g.centre[1])[0]];
            for l in &g.lines {
                p.push(aurora_render::stars::equatorial(l.a[0], l.a[1]));
                p.push(aurora_render::stars::equatorial(l.b[0], l.b[1]));
            }
            p
        })
        .collect();
    let tints = catalogue
        .figures
        .iter()
        .map(|f| {
            let culture = catalogue.cultures.get(f.culture).map_or("", |c| c.id.as_str());
            aurora_render::constellations::pack_tint(pastel(culture, &f.id))
        })
        .collect();
    Loaded {
        tints,
        catalogue,
        geoms,
        drawing_geoms: drawings,
        ranges,
        points,
        first,
        figure_of,
    }
}

/// The tables of one choice of cultures (`listed`, per culture): what the
/// GPU holds. Made on a thread of its own when the ticked cultures change;
/// a culture unticked stays listed until its figures have faded out. The
/// figures keep their catalogue numbers in every view, so what shows goes
/// on showing when another culture comes or goes.
pub struct View {
    listed: Vec<bool>,
    tables: Arc<Vec<u8>>,
    bases: Bases,
    /// The listed figures' numbers, and one past the last (the uniform's
    /// part in use).
    active: Vec<usize>,
    slots: u32,
}

fn view(l: &Loaded, listed: &[bool]) -> View {
    let on = |k: usize| listed.get(k).copied().unwrap_or(false);
    let active: Vec<usize> = l
        .ranges
        .iter()
        .enumerate()
        .filter(|(k, _)| on(*k))
        .flat_map(|(_, r)| r.clone())
        .collect();
    let figures: Vec<bool> = (0..l.geoms.len())
        .map(|i| l.catalogue.figures.get(i).is_some_and(|f| on(f.culture)))
        .collect();
    let (tables, bases) = build_tables_of(&l.geoms, &l.drawing_geoms, Some(&figures));
    View {
        listed: listed.to_vec(),
        tables: Arc::new(tables),
        bases,
        slots: active.iter().max().map_or(0, |&i| i as u32 + 1),
        active,
    }
}

/// The folder the data is read from: `AURORA_CONSTELLATION_DIR`, else
/// `constellations` in the configuration folder, beside the photo LUTs
/// (settings.rs `config_dir`; the demo's own folder in the demo).
pub fn data_dir() -> PathBuf {
    match std::env::var_os("AURORA_CONSTELLATION_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => crate::settings::config_dir().join("constellations"),
    }
}

/// The data as last read: the folder and the overlay's catalogue (`None`:
/// nothing usable in the folder).
pub struct Scanned {
    pub dir: PathBuf,
    pub loaded: Option<Arc<Loaded>>,
    pub planet_names: Arc<planet_names::Texts>,
}

/// Reading the folder, off the frame's thread: one read at a time, a new
/// catalogue only when a file changed, each problem logged once.
mod library {
    use super::{Scanned, catalogue};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};

    /// Bumped with each new catalogue (0: never read).
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static STATE: Mutex<State> = Mutex::new(State {
        scanning: false,
        latest: None,
        signature: None,
        said: std::collections::BTreeSet::new(),
    });

    struct State {
        scanning: bool,
        latest: Option<Arc<Scanned>>,
        signature: Option<(std::path::PathBuf, catalogue::Signature)>,
        said: std::collections::BTreeSet<String>,
    }

    fn lock() -> MutexGuard<'static, State> {
        STATE.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn generation() -> u64 {
        GENERATION.load(Ordering::Acquire)
    }

    pub fn latest() -> (u64, Option<Arc<Scanned>>, bool) {
        let st = lock();
        (generation(), st.latest.clone(), st.scanning)
    }

    /// A warning, once a session.
    pub fn say_once(text: String) {
        if lock().said.insert(text.clone()) {
            log::warn!("{text}");
        }
    }

    /// Reads the folder again on a thread of its own (unless one is
    /// reading).
    pub fn rescan() {
        {
            let mut st = lock();
            if st.scanning {
                return;
            }
            st.scanning = true;
        }
        let dir = super::data_dir();
        let spawned = std::thread::Builder::new().name("constellation-data".into()).spawn(move || {
            struct Finish;
            impl Drop for Finish {
                fn drop(&mut self) {
                    lock().scanning = false;
                }
            }
            let _finish = Finish;
            let t = std::time::Instant::now();
            let signature = catalogue::signature(&dir);
            {
                let st = lock();
                if st.latest.is_some() && st.signature.as_ref().is_some_and(|(d, s)| *d == dir && *s == signature) {
                    return;
                }
            }
            let mut problems = Vec::new();
            let c = catalogue::scan(&dir, &mut problems);
            let planet_names = Arc::new(super::planet_names::read(&dir, &mut problems));
            for p in problems {
                say_once(format!("constellations: {}: {p}", dir.display()));
            }
            let millis = t.elapsed().as_secs_f64() * 1000.0;
            let loaded = if c.is_empty() {
                log::info!("constellations: no data in {} ({millis:.1} ms)", dir.display());
                None
            } else {
                log::info!(
                    "constellations: {} in {}: {} ({millis:.1} ms)",
                    "read",
                    dir.display(),
                    catalogue::summary(&c)
                );
                // (the tables are made per choice of cultures: `View`)
                Some(Arc::new(super::load(c)))
            };
            let mut st = lock();
            st.signature = Some((dir.clone(), signature));
            st.latest = Some(Arc::new(Scanned { dir, loaded, planet_names }));
            GENERATION.fetch_add(1, Ordering::AcqRel);
        });
        if let Err(e) = spawned {
            log::error!("constellations: no thread to read the data: {e}");
            lock().scanning = false;
        }
    }
}

/// Reads the data folder again (the window's Rescan): a set dropped into
/// it while the viewer runs appears without a restart. Off the frame's
/// thread; a folder that did not change is not read again.
pub fn rescan() {
    library::rescan();
}

/// The credits window's view of the data folder's `credits.json` files
/// (constellations_credits.rs): read on a thread of its own the first time
/// it asks, after a new reading of the folder and after a Rescan; their
/// groups take the catalogue's cultures and sets, by their shown names.
pub fn credits_view() -> credits::View {
    let (generation, latest, reading) = library::latest();
    if generation == 0 && !reading {
        library::rescan();
    }
    let language = aurora_i18n::language();
    credits::view(generation, move || {
        let dir = latest.as_ref().map_or_else(data_dir, |s| s.dir.clone());
        let (cultures, sets): (Vec<(String, String)>, Vec<(String, String)>) = latest
            .as_ref()
            .and_then(|s| s.loaded.as_ref())
            .map(|l| {
                (
                    l.catalogue
                        .cultures
                        .iter()
                        .map(|c| (c.id.clone(), c.display_name(language)))
                        .collect(),
                    l.catalogue
                        .sets
                        .iter()
                        .map(|s| (s.id.clone(), s.display_name(language).to_owned()))
                        .collect(),
                )
            })
            .unwrap_or_default();
        let mut problems = Vec::new();
        let groups = credits::read(&dir, &cultures, &sets, &mut problems);
        for p in problems {
            library::say_once(format!("constellations: {}: {p}", dir.display()));
        }
        groups
    })
}

/// One culture (constellation set) as the window lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct CultureRow {
    pub id: String,
    /// Its name in the interface's language (the western one's is the
    /// catalogues').
    pub name: String,
    /// Its constellations (the figures no longer counted left out).
    pub constellations: usize,
    /// Its figures past the GPU's room (aurora-render `MAX_FIGURES`, every
    /// culture together, in the catalogue's order): never drawn.
    pub left_out: usize,
}

/// What the Constellations window shows of the data: whether it is being
/// read, the cultures (their names in the interface's language) and how
/// many art sets it has (the folder and its absence are in `Status`). The
/// first call reads the folder.
pub struct DataStatus {
    pub reading: bool,
    pub cultures: Vec<CultureRow>,
    pub sets: usize,
    /// Names of figures in the field the atlas has no room for (the last
    /// frame's): they show as room frees.
    pub names_waiting: usize,
}

pub fn data_status() -> DataStatus {
    let (generation, latest, reading) = library::latest();
    if generation == 0 && !reading {
        library::rescan();
    }
    let language = aurora_i18n::language();
    let Some(s) = latest else {
        return DataStatus {
            reading: true,
            cultures: Vec::new(),
            sets: 0,
            names_waiting: 0,
        };
    };
    let (cultures, sets) = match &s.loaded {
        Some(l) => {
            let constellations = catalogue::culture_counts(&l.catalogue);
            let mut left_out = vec![0; l.catalogue.cultures.len()];
            for f in l.catalogue.figures.iter().skip(MAX_FIGURES) {
                if let Some(n) = left_out.get_mut(f.culture) {
                    *n += 1;
                }
            }
            (
                l.catalogue
                    .cultures
                    .iter()
                    .enumerate()
                    .map(|(k, c)| CultureRow {
                        id: c.id.clone(),
                        name: c.display_name(language),
                        constellations: constellations.get(k).copied().unwrap_or(0),
                        left_out: left_out.get(k).copied().unwrap_or(0),
                    })
                    .collect(),
                l.catalogue.sets.len(),
            )
        }
        None => (Vec::new(), 0),
    };
    DataStatus {
        reading,
        cultures,
        sets,
        names_waiting: NAMES_WAITING.load(std::sync::atomic::Ordering::Relaxed),
    }
}

/// Why nothing shows, for the window's status line (`Showing` when
/// something can).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    /// The folder is being read for the first time.
    #[default]
    Reading,
    Showing,
    /// The overlay's switch is off.
    Off,
    /// The night sky is the Legacy Viewer's, not the real one.
    NotRealSky,
    /// The sun is less than `below` degrees under the horizon.
    Daylight {
        below: f32,
    },
    /// The folder holds no usable data.
    NoData(PathBuf),
}

/// The status line's reason, the first that applies: the switch, the night
/// sky, the data, then the daylight. `data`: `None` while the folder was
/// never read, else whether it holds data and where it is.
pub fn status(s: &ConstellationSettings, real_sky: bool, dark: bool, data: Option<(bool, &std::path::Path)>) -> Status {
    if !s.enabled {
        Status::Off
    } else if !real_sky {
        Status::NotRealSky
    } else {
        match data {
            None => Status::Reading,
            Some((false, dir)) => Status::NoData(dir.to_path_buf()),
            Some((true, _)) if !dark => Status::Daylight {
                below: s.sun_below_degrees,
            },
            Some((true, _)) => Status::Showing,
        }
    }
}

/// The settings with `AURORA_DEMO_CONSTELLATIONS`'s items in them, for the
/// window the demo opens (`AURORA_DEMO_CONSTELLATIONS_WINDOW`), so that it
/// shows what the scenario turned on; the demo only, never saved.
pub fn demo_settings(s: &ConstellationSettings) -> ConstellationSettings {
    Overrides::from_env(true).apply(s)
}

/// `zenith=<figure id>` of `AURORA_DEMO_CONSTELLATIONS` (the demo only):
/// the real sky's place (latitude, and longitude for the real time at
/// `unix_days`) that puts the figure's label point overhead, whatever the
/// figure's declination: a far southern figure at the zenith, with its
/// name, to look at the names' upright rule there. Reads the data folder
/// once, at the demo's start.
pub fn demo_zenith(unix_days: f64) -> Option<(f32, f32)> {
    let v = std::env::var("AURORA_DEMO_CONSTELLATIONS").ok()?;
    let id = parse_overrides(None, Some(&v)).zenith?;
    let catalogue = catalogue::scan(&data_dir(), &mut Vec::new());
    let f = catalogue.figures.iter().find(|f| f.id.eq_ignore_ascii_case(&id))?;
    let [ra, dec] = f.geom.label;
    Some(zenith_place(ra, dec, unix_days))
}

/// Latitude and longitude (degrees) where `ra`, `dec` (radians) is
/// overhead at `unix_days` (real time): the latitude is the declination,
/// the local sidereal time the right ascension (night_sky.rs star_frame).
fn zenith_place(ra: f32, dec: f32, unix_days: f64) -> (f32, f32) {
    use crate::world::night_sky::{J2000_UNIX_DAYS, greenwich_sidereal};
    let gmst = greenwich_sidereal(unix_days - J2000_UNIX_DAYS);
    let longitude = (f64::from(ra) - gmst).to_degrees().rem_euclid(360.0);
    let longitude = if longitude > 180.0 { longitude - 360.0 } else { longitude };
    (dec.to_degrees(), longitude as f32)
}

/// A name's letters, at the default size, are this tall in the sky
/// (degrees): its font's em, 16 points. The art keeps a box this tall
/// clear around each label point.
pub const NAME_EM_DEGREES: f32 = 1.2;
const DEFAULT_FONT: f32 = 16.0;
/// The names' texels in the sky (degrees): half the engravings', so that a
/// name is as sharp as its drawing, or sharper, at any zoom, window or
/// photo size, without drawing it again.
pub const NAME_TEXEL_DEGREES: f32 = 0.02;

/// The names' em in texels of their pictures for a font size (points).
pub fn name_em_texels(font: f32) -> f32 {
    NAME_EM_DEGREES * font / DEFAULT_FONT / NAME_TEXEL_DEGREES
}

/// The names drawn for one font size, scale and language.
struct NameSet {
    key: (u32, u32, String),
    images: Vec<Option<names::NameImage>>,
    /// Per figure, how far under its label point its name hangs (`name_hang`,
    /// radians; 0 centred), decided with its picture: its spot near the
    /// zenith, where the name turns with the view.
    hang: Vec<f32>,
    /// Per figure, its stars (`figure_points`), and where its name goes off
    /// them as the sky turns (`Spot`).
    stars: Vec<Vec<Vec3>>,
    spots: Vec<Spot>,
    /// Per figure, its name was drawn (into `images`, `None` when it has
    /// no glyph): a figure's name is drawn the first time it is wanted.
    drawn: Vec<bool>,
}

/// Where a shown name's centre is, off its label point (its own frame:
/// x right, y up, radians): `at` slides toward `goal` (`name_goal`),
/// chosen again once the sky has turned it (`right`, the name's right when
/// it was last chosen; zero: never). `lp`: its figure's label point (unit,
/// catalogue frame), kept with it so a frame that moves nothing costs a
/// cross product and a comparison per name.
#[derive(Clone, Copy, Debug, Default)]
struct Spot {
    goal: Option<Vec2>,
    at: Vec2,
    right: Vec3,
    lp: Vec3,
}

/// A name's texel in the sky (radians): `NAME_TEXEL_DEGREES`, or smaller
/// for a name too long to stay within `NAME_REACH` of its point. Each name
/// its own: a culture's long name never shrinks the others.
pub fn name_texel(size: [u32; 2]) -> f32 {
    let half = (size[0] as f32).hypot(size[1] as f32) * 0.5;
    NAME_TEXEL_DEGREES.to_radians().min(NAME_REACH / half.max(1.0))
}

/// How far each of a figure's stars (its lines' ends) is from its label
/// point (radians), nearest first: 0 for a figure of one star. The same
/// whichever way the sky turns.
pub fn figure_stars(geom: &FigureGeom) -> Vec<f32> {
    let label = aurora_render::stars::equatorial(geom.label[0], geom.label[1]);
    let mut out: Vec<f32> = geom
        .lines
        .iter()
        .flat_map(|l| [l.a, l.b])
        .map(|p| label.angle_between(aurora_render::stars::equatorial(p[0], p[1])))
        .collect();
    out.sort_by(f32::total_cmp);
    out
}

/// How far its name's top may go under its label point, in the name's own
/// heights: past it, a figure is larger than its name and keeps it centred.
const HANG_CAP: f32 = 2.0;

/// How far under its figure's label point a name's base centre goes
/// (radians; 0: centred on it), from its figure's `stars` (`figure_stars`):
/// the rule within `ZENITH_LEVEL` of the zenith, where a name turns with
/// the view; elsewhere `name_spots` moves it off its stars at each turn.
///
/// A name is level with the horizon, so its box turns about the label
/// point as the sky turns: centred, it may come over any star nearer than
/// its farthest corner (its ink, reading and `names::SPACING`). With none
/// so near, it stays centred. Else it hangs below, by the smallest drop
/// that clears its stars whatever the turn: every star either inside the
/// circle its top leaves clear (a `names::HANG_GAP` over its top) or past
/// its farthest corner; a figure of one star the plain case, its name under
/// it like a planet's. Lines may cross under a name, stars never. A figure
/// with no such drop before its top is `HANG_CAP` of its height down is
/// larger than its name and keeps it centred, as does a name that would
/// leave its NAME_REACH at its texel (`name_texel`). Decided once with the
/// picture: static, so the name never jumps as the view turns.
pub fn name_hang(img: &names::NameImage, stars: &[f32]) -> f32 {
    let texel = name_texel(img.reach());
    let half = 0.5 * img.spaced()[0] as f32 * texel;
    let (top, bottom) = (img.top() * texel, img.bottom() * texel);
    if stars.iter().all(|&d| d >= half.hypot(top.max(bottom))) {
        return 0.0;
    }
    for &d in stars {
        // the circle its top leaves clear: past this star, with the gap
        let clear = d + names::HANG_GAP;
        if clear > HANG_CAP * (top + bottom) {
            break;
        }
        let drop = clear + top;
        let far = half.hypot(drop + bottom);
        if !stars.iter().all(|&e| e < clear || e > far) {
            continue;
        }
        // the hung picture reaches as far under its label point as a
        // centred one this tall would
        let tall = img.size[1] as f32 - img.lift as f32 + 2.0 * drop / texel;
        return if name_texel([img.size[0], tall.ceil() as u32]) < texel {
            0.0
        } else {
            drop
        };
    }
    0.0
}

/// A figure's stars (its lines' ends, each once) as unit vectors of the
/// catalogue frame: what a name's spot keeps clear (`name_spot`).
pub fn figure_points(geom: &FigureGeom) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = Vec::new();
    for p in geom.lines.iter().flat_map(|l| [l.a, l.b]) {
        let q = aurora_render::stars::equatorial(p[0], p[1]);
        if !out.iter().any(|w| w.distance_squared(q) < 1e-12) {
            out.push(q);
        }
    }
    out
}

/// A name's box about its text's centre (radians, at its texel,
/// `name_texel`): half its width with its spacing, and how far its ink,
/// reading and spacing reach above and below it (`NameImage::top`,
/// `bottom`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NameBox {
    pub half: f32,
    pub top: f32,
    pub bottom: f32,
}

impl NameBox {
    pub fn of(img: &names::NameImage) -> Self {
        let texel = name_texel(img.reach());
        Self {
            half: 0.5 * img.spaced()[0] as f32 * texel,
            top: img.top() * texel,
            bottom: img.bottom() * texel,
        }
    }

    pub fn height(&self) -> f32 {
        self.top + self.bottom
    }

    /// How many of `stars` (the name's frame: x right, y up, from its label
    /// point) lie under the box centred at `o`, grown by `pad` all round.
    pub fn covered(&self, o: Vec2, stars: &[Vec2], pad: f32) -> usize {
        stars
            .iter()
            .filter(|s| {
                let d = **s - o;
                d.x.abs() < self.half + pad && d.y < self.top + pad && d.y > -(self.bottom + pad)
            })
            .count()
    }

    /// The distances along `u` (a unit direction of the name's frame) at
    /// which the box, grown by `pad`, lies over star `s`: an open span, or
    /// none when the box never comes over it that way.
    fn span(&self, s: Vec2, u: Vec2, pad: f32) -> Option<(f32, f32)> {
        // lo < s - d u < hi on one axis
        let axis = |sc: f32, uc: f32, lo: f32, hi: f32| {
            if uc.abs() < 1e-7 {
                return (lo < sc && sc < hi).then_some((f32::NEG_INFINITY, f32::INFINITY));
            }
            let (a, b) = ((sc - hi) / uc, (sc - lo) / uc);
            Some((a.min(b), a.max(b)))
        };
        let x = axis(s.x, u.x, -(self.half + pad), self.half + pad)?;
        let y = axis(s.y, u.y, -(self.bottom + pad), self.top + pad)?;
        let (lo, hi) = (x.0.max(y.0), x.1.min(y.1));
        (hi > lo && hi > 0.0).then_some((lo, hi))
    }
}

/// The directions a name may move in (`name_spot`), every 15°, straight
/// down first, the shortest clear move winning: over a sweep of the sky's
/// turns this placement leaves Japanese names on their own stars at 0 to
/// 0.6% of the turns, the Norse ones 3 to 6% (the drop of `name_hang`: 6
/// to 58%), English ones at 0 to 9% at 16 points (10 to 35%); most of the
/// rest are long names already shrunk to their reach, which cannot move.
/// Of 4, 8 and 24 directions, 24 moved names least far.
const SPOT_DIRECTIONS: usize = 24;
/// How far a name's centre may move from its label point, in its heights.
const SPOT_BOUND: f32 = 2.0;
/// The room a newly chosen spot keeps round its stars, in name heights: the
/// turning sky does not bring the name straight back over one.
const SPOT_PAD: f32 = 0.15;
/// A clear spot is kept until another lies nearer the label point by this
/// much, in name heights: no flip-flop between two spots alike.
const SPOT_MARGIN: f32 = 0.5;
/// How fast a name slides to a new spot, in its heights per second.
const SPOT_SPEED: f32 = 2.0;
/// A name's spot is chosen again once the sky has turned it this much
/// (cos 0.25°): nothing runs for a sky that does not move.
const SPOT_TURN_COS: f32 = 0.999_990_5;
/// `length(cross(label, zenith))` under which env.wgsl leans a name on the
/// camera's right (its 0.14, and a little room): within about 8.6° of the
/// zenith a name turns with the view, so it keeps the fixed drop there
/// (`name_hang`), clear whichever way it turns.
const ZENITH_LEVEL: f32 = 0.15;

/// How far a name's centre may move (radians): `SPOT_BOUND` of its
/// heights, and never so far that its picture leaves the `NAME_REACH`
/// round its label point the cell lists hold it in.
pub fn spot_bound(img: &names::NameImage, b: &NameBox) -> f32 {
    let texel = name_texel(img.reach());
    let [w, h] = img.reach();
    let half = (w as f32).hypot(h as f32) * 0.5 * texel;
    (NAME_REACH - half).max(0.0).min(SPOT_BOUND * b.height())
}

/// The nearest spot for a name's centre (its frame: x right, y up, from its
/// label point) that leaves its figure's `stars` clear by `pad`, at this
/// turn of the sky: centred when nothing is under it, else the shortest
/// move along any of `SPOT_DIRECTIONS`, `bound` at most. With none within
/// the bound, the spot that covers the fewest. Returns the offset and how
/// many stars it still covers (grown by `pad`).
pub fn name_spot(b: &NameBox, stars: &[Vec2], bound: f32, pad: f32) -> (Vec2, usize) {
    if b.covered(Vec2::ZERO, stars, pad) == 0 {
        return (Vec2::ZERO, 0);
    }
    let mut best: Option<(usize, f32, Vec2)> = None;
    for k in 0..SPOT_DIRECTIONS {
        let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::TAU / SPOT_DIRECTIONS as f32;
        let u = Vec2::new(a.cos(), a.sin());
        let mut spans: Vec<(f32, f32)> = stars.iter().filter_map(|&s| b.span(s, u, pad)).collect();
        spans.sort_by(|p, q| p.0.total_cmp(&q.0));
        // past every star the box would still lie over, in order
        let mut d = 0.0f32;
        for &(lo, hi) in &spans {
            if lo <= d && hi > d {
                d = hi + 1e-6;
            }
        }
        let (left, d) = if d <= bound {
            (0, d)
        } else {
            // none clear this way: the start, the bound, or just past a star
            std::iter::once(0.0)
                .chain(std::iter::once(bound))
                .chain(spans.iter().map(|sp| sp.1 + 1e-6).filter(|x| *x <= bound))
                .map(|x| (b.covered(u * x, stars, pad), x))
                .min_by(|p, q| p.0.cmp(&q.0).then(p.1.total_cmp(&q.1)))
                .unwrap_or((usize::MAX, 0.0))
        };
        if best.is_none_or(|(bl, bd, _)| left < bl || (left == bl && d < bd - 1e-7)) {
            best = Some((left, d, u * d));
        }
    }
    best.map_or((Vec2::ZERO, b.covered(Vec2::ZERO, stars, pad)), |(left, _, o)| (o, left))
}

/// The spot a shown name goes to at this turn of the sky: the one it has
/// (`goal`) while it covers no more of its stars than the best spot and
/// none as good lies nearer the label point by `SPOT_MARGIN` of its
/// height; else the nearest clear one, with `SPOT_PAD` of room (or without
/// it, when only that is clear); with none within `bound`, the one that
/// covers the fewest. Kept that way, a name with no clear spot does not
/// wander between spots alike either.
pub fn name_goal(b: &NameBox, stars: &[Vec2], goal: Option<Vec2>, bound: f32) -> Vec2 {
    let h = b.height();
    let (mut best, mut left) = name_spot(b, stars, bound, SPOT_PAD * h);
    if left > 0 {
        (best, left) = name_spot(b, stars, bound, 0.0);
    }
    match goal {
        Some(g) if g.length() <= bound + 1e-6 && b.covered(g, stars, 0.0) <= left && g.length() <= best.length() + SPOT_MARGIN * h => g,
        _ => best,
    }
}

/// `at` moved toward `goal` by `step` at most: a name slides, never jumps.
pub fn slide(at: Vec2, goal: Vec2, step: f32) -> Vec2 {
    let d = goal - at;
    let len = d.length();
    if len <= step { goal } else { at + d * (step / len) }
}

/// A name's right and up at its label point `lp` (catalogue frame) as
/// env.wgsl turns it, level with the horizon for `zenith` (the region's
/// up in the catalogue frame); `None` near the zenith (`ZENITH_LEVEL`),
/// where the shader turns it with the camera.
pub fn name_axes(lp: Vec3, zenith: Vec3) -> Option<(Vec3, Vec3)> {
    let level = lp.cross(zenith);
    (level.length() >= ZENITH_LEVEL).then(|| {
        let right = level.normalize();
        (right, right.cross(lp))
    })
}

/// A name's centre word for the shader (`FrameState::names` z): `from`, its
/// label point (0 the figure's, d + 1 drawing d's own), as bits; on the
/// figure's own point, its offset `spot` (radians, its own frame: x right,
/// y up) behind the top bit, in quarters of its `texel`: x in bits 16..29,
/// y in bits 0..15, signed (env.wgsl). Bit 30 stays clear, so the word is
/// never a NaN a driver might rewrite. A figure whose drawing shows, its
/// set having placed that point clear of the art (`drawn`:
/// `Figure::label_clear_of_drawing`), keeps its name centred.
pub fn name_spot_word(from: u32, spot: Vec2, texel: f32, drawn: bool) -> f32 {
    if from != 0 || drawn || spot == Vec2::ZERO || texel <= 0.0 || !spot.is_finite() {
        return f32::from_bits(from);
    }
    let q = spot * (4.0 / texel);
    let x = (q.x.round().clamp(-8191.0, 8191.0) as i32 as u32) & 0x3fff;
    let y = (q.y.round().clamp(-32767.0, 32767.0) as i32 as u32) & 0xffff;
    f32::from_bits(1 << 31 | x << 16 | y)
}

/// The overlay's state on the main thread (`App::constellations`).
#[derive(Default)]
pub struct Constellations {
    overrides: Option<Overrides>,
    worker: Option<Worker>,
    /// The catalogue in use and its library generation; a newer one waits
    /// (`swapping`) until what shows has faded out.
    loaded: Option<Arc<Loaded>>,
    generation: u64,
    swapping: bool,
    figures: Vec<state::Figure>,
    /// The tables of the cultures drawn, and the next ones being made.
    view: Option<Arc<View>>,
    building: Option<Receiver<View>>,
    atlas: atlas::Atlas,
    /// Per drawing (all sets): its stroke and opacity; its picture's load.
    drawings: Vec<state::Drawing>,
    loads: Vec<state::Load>,
    /// Per figure: where its name stands (0 its label point, d + 1 drawing
    /// d's own); its lines' opacity (the lines switch fades them).
    label_from: Vec<u32>,
    lines_alpha: Vec<f32>,
    /// What the window's status line says (the last frame's), and the
    /// folder the catalogue in use was read from.
    status: Status,
    data_dir: PathBuf,
    names: Option<NameSet>,
    /// The renderer the atlas was filled for: a new one holds nothing.
    device: Option<aurora_render::wgpu::Device>,
    tables_id: u64,
    idle: f32,
    /// The clock of the loads' timings; the demo's last rescan on it.
    epoch: Option<std::time::Instant>,
    rescanned: f64,
    /// This frame's overlay for the renderer; the last idle one is kept.
    frame: Option<Arc<Overlay>>,
    /// Profile: figures shown, atlas bytes sent (AURORA_PROFILE, the log).
    pub shown: usize,
    pub uploaded: u64,
    logged: (usize, u32),
    /// The demo's mixed sparkle shapes (`sparkles=mix`).
    sparkle_mix: bool,
    /// Names of figures wanted that the atlas has no room for yet, and the
    /// count the log said last.
    pub names_waiting: usize,
    waiting_logged: usize,
    planet_names: planet_names::PlanetNames,
}

impl Constellations {
    /// The overlay for this frame's parameters (`FrameParams::constellations`).
    pub fn frame(&self) -> Option<Arc<Overlay>> {
        self.frame.clone()
    }

    /// Why nothing shows (the last frame's), for the window's status line.
    pub fn status(&self) -> &Status {
        &self.status
    }

    fn now(&mut self) -> f64 {
        self.epoch.get_or_insert_with(std::time::Instant::now).elapsed().as_secs_f64()
    }

    /// One frame. `ctx`: the interface's fonts (the names).
    pub fn update(&mut self, input: &Input, demo: bool, ctx: &egui::Context, device: &aurora_render::wgpu::Device) {
        let overrides = self.overrides.get_or_insert_with(|| Overrides::from_env(demo)).clone();
        self.sparkle_mix = overrides.sparkle_mix;
        let s = overrides.apply(input.settings);
        if self.device.as_ref() != Some(device) {
            if self.device.is_some() {
                // a new renderer (the GPU was recovered): its atlas is empty
                self.atlas.clear();
                self.tables_id += 1;
                self.reset_all();
            }
            self.device = Some(device.clone());
        }
        if !input.real_sky {
            // the Legacy Viewer's stars: the figures have nothing to stand on
            self.reset_all();
        }
        if overrides.rescan {
            let now = self.now();
            if now - self.rescanned >= DEMO_RESCAN {
                self.rescanned = now;
                library::rescan();
            }
        }
        let any_shown = self.figures.iter().any(state::Figure::shown) || self.drawings.iter().any(state::Drawing::shown);
        // a newer reading of the folder: taken once nothing of the old shows
        // (what shows fades out first: a rescan never blinks)
        if library::generation() != self.generation {
            if any_shown {
                self.swapping = true;
            } else {
                self.adopt();
            }
        }
        let dark = state::dark_enough(input.sun, s.sun_below_degrees);
        let gate = s.enabled && input.real_sky && dark;
        let data_dir = self.data_dir.clone();
        self.status = status(
            &s,
            input.real_sky,
            dark,
            (self.generation > 0).then_some((self.loaded.is_some(), data_dir.as_path())),
        );
        // the planets' names (constellations_planets.rs): drawn alone when
        // the figures show nothing
        let (planets_busy, mut planet_uploads) = self.planet_names_step(input, &s, dark, ctx);
        if !gate && !any_shown {
            self.idle_or_planet_names(input.dt, planets_busy, planet_uploads);
            return;
        }
        self.idle = 0.0;
        if self.generation == 0 {
            // the folder's first reading (its own thread); nothing to draw
            // until it is done
            library::rescan();
            self.idle_or_planet_names(0.0, planets_busy, planet_uploads);
            return;
        }
        let Some(loaded) = self.loaded.clone() else {
            self.idle_or_planet_names(0.0, planets_busy, planet_uploads);
            return;
        };
        if self.worker.is_none() {
            self.worker = Worker::start();
        }
        let gate = gate && !self.swapping;
        let n = loaded.catalogue.figures.len().min(MAX_FIGURES);
        let nd = loaded.figure_of.len();
        self.figures.resize(n, state::Figure::default());
        self.label_from.resize(n, 0);
        self.lines_alpha.resize(n, 1.0);
        self.drawings.resize(nd, state::Drawing::default());
        self.loads.resize(nd, state::Load::default());
        self.receive();
        self.atlas.tick();
        // the culture shown; the tables list it and those still fading
        // out, made again off this thread when that changes (a culture
        // shows once its tables are there)
        let ids: Vec<String> = loaded.catalogue.cultures.iter().map(|c| c.id.clone()).collect();
        let shown = catalogue::shown_culture(&ids, s.cultures.as_deref());
        let ticked: Vec<bool> = (0..ids.len()).map(|k| Some(k) == shown).collect();
        self.take_view();
        let listed = |k: usize| self.view.as_ref().is_some_and(|v| v.listed.get(k).copied().unwrap_or(false));
        let to_list: Vec<bool> = (0..ticked.len())
            .map(|k| {
                ticked[k]
                    || (listed(k)
                        && loaded
                            .ranges
                            .get(k)
                            .is_some_and(|r| r.clone().any(|i| self.figures.get(i).is_some_and(state::Figure::shown))))
            })
            .collect();
        if self.building.is_none() && self.view.as_ref().is_none_or(|v| v.listed != to_list) {
            self.build_view(&loaded, to_list);
        }
        let view = self.view.clone();
        let active: &[usize] = view.as_ref().map_or(&[], |v| v.active.as_slice());
        // more atlas room than one culture's only for more figures than its
        self.atlas.set_room(if active.len() > FEW_FIGURES {
            MAX_LAYERS
        } else {
            atlas::FEW_LAYERS
        });
        let language: &str = match overrides.name_language.as_deref() {
            Some(code) => code,
            None => aurora_i18n::language(),
        };
        let now = self.now();
        let mut uploads = Vec::new();
        uploads.append(&mut planet_uploads);
        let axes = input.stars.axes;
        // the stars' own rotation (night_sky.rs star_frame): catalogue to
        // region, whatever turns the sky
        let region = |p: Vec3| axes[0] * p.x + axes[1] * p.y + axes[2] * p.z;
        let look = input.look.normalize_or(Vec3::Y);
        // which figures are wanted, nearest the view first: the camera and
        // the horizon only, never where a figure sits on the celestial sphere
        let mut want = vec![false; n];
        let mut nearness = vec![-2.0f32; n];
        // the listed cultures' figures only
        for &i in active {
            let f = &loaded.catalogue.figures[i];
            let allowed = (f.kind != Kind::Obsolete || s.obsolete) && ticked.get(f.culture).copied().unwrap_or(false);
            let best = loaded.points[i]
                .iter()
                .map(|p| region(*p))
                .filter(|r| r.z > -0.05)
                .map(|r| r.dot(look))
                .fold(-2.0f32, f32::max);
            nearness[i] = best;
            let fig = &mut self.figures[i];
            fig.in_field = best > -1.5 && state::in_field(fig.in_field, best, s.field_degrees);
            want[i] = gate && allowed && fig.in_field;
        }
        // each figure's drawing to show: the first set's that has one, every
        // set of its culture used; its names (the first time a figure is
        // wanted)
        let mut choice: Vec<Option<u32>> = vec![None; n];
        for &i in active {
            let f = &loaded.catalogue.figures[i];
            choice[i] = s
                .drawings_on()
                .then(|| chosen(&f.drawings))
                .flatten()
                .map(|k| loaded.first[i] + k as u32);
        }
        self.ensure_names(&s, ctx, language, &want, active);
        let figure_of = |d: u32| loaded.figure_of.get(d as usize).map_or(usize::MAX, |&f| f as usize);
        // a drawing is wanted: its figure is, and it is the one to show
        let wanted = |d: u32| {
            let f = figure_of(d);
            f < n && want[f] && choice[f] == Some(d)
        };
        let drawing_of = |d: u32| {
            let f = figure_of(d);
            loaded
                .catalogue
                .figures
                .get(f)
                .and_then(|fig| fig.drawings.get((d - loaded.first[f]) as usize))
        };
        // the atlas holds what was evicted no more
        for (d, load) in self.loads.iter_mut().enumerate() {
            if load.in_atlas() && self.atlas.place_of(Item::Picture(d as u32)).is_none() {
                load.lost();
            }
        }
        let busy: Vec<bool> = (0..n).map(|i| want[i] || self.figures[i].shown()).collect();
        let keep: Vec<bool> = self
            .drawings
            .iter()
            .zip(&self.loads)
            .enumerate()
            .map(|(d, (dw, load))| dw.shown() || matches!(load, state::Load::Uploading { .. }) || wanted(d as u32))
            .collect();
        let in_use = |item: Item| match item {
            Item::Picture(d) => keep.get(d as usize).copied().unwrap_or(false),
            Item::Name(f) => busy.get(f as usize).copied().unwrap_or(false),
            Item::PlanetName(_) => true,
        };
        // the pictures: asked of the worker when wanted, given their room,
        // sent in bands (the nearest figures first), let go when nobody
        // waits for them any more
        let mut order: Vec<u32> = (0..nd as u32).collect();
        order.sort_by(|a, b| {
            let near = |d: u32| nearness.get(figure_of(d)).copied().unwrap_or(-2.0);
            near(*b).total_cmp(&near(*a))
        });
        let mut placed = 0;
        let mut budget = BAND_BYTES;
        for &d in &order {
            let w = wanted(d);
            let Some(dr) = drawing_of(d) else { continue };
            let load = &mut self.loads[d as usize];
            if w && load.request(now) {
                let sent = self.worker.as_ref().is_some_and(|wk| {
                    wk.jobs
                        .send(Job {
                            drawing: d,
                            path: dr.image.clone(),
                            size: dr.picture.size,
                            generation: self.generation,
                        })
                        .is_ok()
                });
                if !sent {
                    load.unsent();
                    library::say_once("constellations: no worker to read the drawings: lines and names only".into());
                }
            }
            if !w {
                load.unwanted();
            }
            if matches!(load, state::Load::Decoded { .. })
                && placed < PLACED_PER_FRAME
                && self.atlas.place(Item::Picture(d), dr.picture.size, in_use).is_some()
            {
                load.placed();
                placed += 1;
            }
            if budget > 0
                && let Some(p) = self.atlas.place_of(Item::Picture(d))
            {
                let size = dr.picture.size;
                // the bytes of one level-0 row and its mips' share
                let row_bytes = (size[0] as usize * 2 * 4).div_ceil(3).max(1);
                if let Some(band) = load.next_band(size[1], (budget / row_bytes).max(1) as u32) {
                    let sent: usize = band.rows[1].saturating_sub(band.rows[0]) as usize * row_bytes;
                    budget = budget.saturating_sub(sent.max(1));
                    if let Some(since) = band.done_since {
                        let figure = &loaded.catalogue.figures[figure_of(d)];
                        log::info!(
                            "constellations: {} ({}) ready {:.0} ms after it was asked, {} x {} texels",
                            figure.id,
                            loaded.catalogue.sets.get(dr.set).map_or("?", |s| s.id.as_str()),
                            (now - since) * 1000.0,
                            size[0],
                            size[1]
                        );
                    }
                    uploads.push(Upload {
                        item: Item::Picture(d),
                        layer: p.layer,
                        block: p.block,
                        size,
                        rows: band.rows,
                        levels: band.levels,
                    });
                }
            }
        }
        // the region's up in the catalogue frame: what levels the names
        let zenith = Vec3::new(axes[0].z, axes[1].z, axes[2].z);
        self.name_spots(active, &want, &s, input.dt, zenith);
        let mut names_sent = 0;
        let (mut names_full, mut names_waiting) = (false, 0usize);
        for &i in active {
            let figure = &loaded.catalogue.figures[i];
            // its lines begin at once, paced for its drawing when it has one
            // to come; the drawing draws itself in once its picture is whole
            let ready = Some(choice[i].is_some_and(|d| !self.loads[d as usize].failed()));
            // the name's centre: the shown drawing's set may move it; it
            // moves only while the name is faded out
            let label_to = choice[i]
                .filter(|&d| figure.drawings[(d - loaded.first[i]) as usize].label.is_some())
                .map_or(0, |d| d + 1);
            // the name in the atlas once its figure is wanted
            let mut name_on = false;
            if s.names
                && let Some(img) = self.names.as_ref().and_then(|set| set.images.get(i)).and_then(Option::as_ref)
            {
                let item = Item::Name(i as u32);
                if self.atlas.place_of(item).is_some() {
                    name_on = true;
                } else if want[i] {
                    // once one finds no room, the others wait for the next
                    // frames (each try searches the whole atlas); counted
                    let placed = (names_sent < NAMES_PER_FRAME && !names_full)
                        .then(|| self.atlas.place(item, img.size, in_use))
                        .flatten();
                    match placed {
                        Some(p) => {
                            uploads.push(Upload {
                                item,
                                layer: p.layer,
                                block: p.block,
                                size: img.size,
                                rows: [0, img.size[1]],
                                levels: img.levels.clone(),
                            });
                            names_sent += 1;
                        }
                        None => {
                            // (not counted while it only waits for its turn)
                            names_full |= names_sent < NAMES_PER_FRAME;
                            names_waiting += usize::from(names_full);
                        }
                    }
                }
            }
            let fig = &mut self.figures[i];
            fig.step(state::Step {
                want: want[i],
                ready,
                name_on: name_on && self.label_from[i] == label_to,
                dt: input.dt,
                draw_seconds: s.draw_seconds,
                fade_seconds: s.fade_seconds,
            });
            if fig.name_alpha <= 0.0 {
                self.label_from[i] = label_to;
            }
            // the lines switch fades them like the others
            let fade = input.dt.max(0.0) / s.fade_seconds.max(0.1);
            let la = &mut self.lines_alpha[i];
            *la = if s.lines { (*la + fade).min(1.0) } else { (*la - fade).max(0.0) };
            // its drawings: the chosen one draws itself in with the figure
            // (or once its picture is whole), the others fade out
            let fig_on = fig.started && want[i];
            for k in 0..figure.drawings.len() as u32 {
                let d = loaded.first[i] + k;
                let dw = &mut self.drawings[d as usize];
                let ready = self.loads[d as usize].ready();
                dw.step(fig_on && choice[i] == Some(d), ready, input.dt, s.draw_seconds, s.fade_seconds);
                // the demo's hold of a stage
                if dw.started && fig_on {
                    match overrides.hold {
                        Some(Hold::Reveal(p)) => (dw.reveal, dw.alpha, dw.done) = (p, 1.0, (p >= 1.0).then_some(0.0)),
                        Some(Hold::Fade(_)) => (dw.reveal, dw.alpha) = (state::REVEAL_END, 1.0),
                        Some(Hold::Pulse(t)) => (dw.reveal, dw.alpha, dw.done) = (state::REVEAL_END, 1.0, Some(t)),
                        None => {}
                    }
                }
                if dw.shown() {
                    self.atlas.touch(Item::Picture(d));
                }
            }
            // the demo's hold of a stage
            if fig.started && want[i] {
                match overrides.hold {
                    Some(Hold::Reveal(p)) => {
                        fig.reveal = p;
                        fig.alpha = 1.0;
                        fig.name_alpha = if name_on { smooth_name(p) } else { 0.0 };
                    }
                    Some(Hold::Fade(a)) => {
                        fig.reveal = state::REVEAL_END;
                        fig.alpha = a;
                        fig.name_alpha = if name_on { 1.0 } else { 0.0 };
                    }
                    Some(Hold::Pulse(t)) => {
                        fig.reveal = state::REVEAL_END;
                        fig.alpha = 1.0;
                        fig.name_alpha = if name_on { 1.0 } else { 0.0 };
                        fig.lines_done = Some(t);
                        fig.name_done = name_on.then_some(t);
                    }
                    None => {}
                }
            }
            if fig.shown() {
                self.atlas.touch(Item::Name(i as u32));
            }
        }
        self.uploaded += uploads
            .iter()
            .map(|u| {
                u.levels
                    .iter()
                    .enumerate()
                    .map(|(k, l)| {
                        let [a, b] = aurora_render::constellations::band_rows(u.rows, u.size[1], k as u32);
                        let w = aurora_render::constellations::level_len(u.size[0], k as u32);
                        (u64::from(b - a) * u64::from(w) * 2).min(l.len() as u64)
                    })
                    .sum::<u64>()
            })
            .sum::<u64>();
        self.shown = self.figures.iter().filter(|f| f.shown()).count();
        self.names_waiting = names_waiting;
        NAMES_WAITING.store(names_waiting, std::sync::atomic::Ordering::Relaxed);
        self.frame = Some(Arc::new(self.overlay(input, &s, uploads)));
        self.log();
    }

    /// Makes the tables of the cultures `listed` on a thread of their own
    /// (on this one if no thread can start).
    fn build_view(&mut self, loaded: &Arc<Loaded>, listed: Vec<bool>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (l, here) = (loaded.clone(), listed.clone());
        let spawned = std::thread::Builder::new().name("constellation-tables".into()).spawn(move || {
            let t = std::time::Instant::now();
            let v = view(&l, &listed);
            let names: Vec<&str> = l
                .catalogue
                .cultures
                .iter()
                .zip(&v.listed)
                .filter(|(_, on)| **on)
                .map(|(c, _)| c.id.as_str())
                .collect();
            log::info!(
                "constellations: tables of {} figures ({}): {} KiB, cell lists of {} and {} references at most ({:.1} ms)",
                v.active.len(),
                names.join(", "),
                v.tables.len() / 1024,
                v.bases.longest,
                v.bases.wide_longest,
                t.elapsed().as_secs_f64() * 1000.0
            );
            let _ = tx.send(v);
        });
        match spawned {
            Ok(_) => self.building = Some(rx),
            Err(e) => {
                log::error!("constellations: no thread for the tables: {e}");
                self.view = Some(Arc::new(view(loaded, &here)));
                self.tables_id += 1;
            }
        }
    }

    /// The tables made last, once ready: the renderer takes them with the
    /// next frame. A cell list longer than the shader reads is said once.
    fn take_view(&mut self) {
        let Some(rx) = &self.building else { return };
        match rx.try_recv() {
            Ok(v) => {
                if v.bases.longest.max(v.bases.wide_longest) > MAX_REFS {
                    library::say_once(format!(
                        "constellations: a part of the sky lists {} lines and names, more than the {MAX_REFS} drawn there: some are left out",
                        v.bases.longest.max(v.bases.wide_longest)
                    ));
                }
                self.view = Some(Arc::new(v));
                self.tables_id += 1;
                self.building = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.building = None,
        }
    }

    /// Takes the library's newest catalogue: everything starts again from
    /// nothing shown (called once nothing shows).
    fn adopt(&mut self) {
        let (generation, latest, _) = library::latest();
        self.generation = generation;
        self.swapping = false;
        if let Some(s) = &latest {
            self.data_dir = s.dir.clone();
        }
        self.planet_names.adopt(latest.as_ref().map(|s| s.planet_names.clone()));
        self.loaded = latest.and_then(|s| s.loaded.clone());
        (self.view, self.building) = (None, None);
        self.figures.clear();
        self.drawings.clear();
        self.loads.clear();
        self.label_from.clear();
        self.lines_alpha.clear();
        self.names = None;
        self.atlas.clear();
        self.tables_id += 1;
        if self.frame.is_some() {
            // the renderer drops what it held of the old catalogue
            self.frame = Some(Arc::new(Overlay::default()));
        }
    }

    /// Every figure and drawing back to nothing shown.
    fn reset_all(&mut self) {
        self.figures.iter_mut().for_each(|f| *f = state::Figure::default());
        self.drawings.iter_mut().for_each(|d| *d = state::Drawing::default());
    }

    /// Nothing shows: the idle overlay once, then none once its atlas may go.
    fn idle(&mut self, dt: f32) {
        self.idle += dt.max(0.0);
        self.shown = 0;
        self.names_waiting = 0;
        NAMES_WAITING.store(0, std::sync::atomic::Ordering::Relaxed);
        if self.frame.is_none() {
            return;
        }
        if self.idle >= RELEASE_AFTER {
            self.frame = None;
            self.atlas.clear();
            self.loads.iter_mut().for_each(|l| {
                l.unwanted();
                l.lost();
            });
            log::info!("constellations: nothing shown for {RELEASE_AFTER} s: the atlas is let go");
            return;
        }
        let keeps = self.frame.as_ref().is_some_and(|f| f.state.head[0] == 0 && f.uploads.is_empty());
        if !keeps {
            let mut o = Overlay {
                layers: self.atlas.layers(),
                ..Overlay::default()
            };
            if let Some(v) = &self.view {
                o.tables = Some(v.tables.clone());
                o.tables_id = self.tables_id;
            }
            self.frame = Some(Arc::new(o));
        }
    }

    /// The worker's pictures.
    fn receive(&mut self) {
        let Some(w) = &self.worker else { return };
        while let Ok(done) = w.done.try_recv() {
            if done.generation != self.generation {
                continue;
            }
            let Some(load) = self.loads.get_mut(done.drawing as usize) else {
                continue;
            };
            if let Some(e) = load.answered(done.result) {
                let file = self
                    .loaded
                    .as_ref()
                    .and_then(|l| {
                        let f = *l.figure_of.get(done.drawing as usize)? as usize;
                        let fig = l.catalogue.figures.get(f)?;
                        Some(fig.drawings.get((done.drawing - l.first[f]) as usize)?.image.display().to_string())
                    })
                    .unwrap_or_default();
                library::say_once(format!("constellations: {file}: {e}: its lines and name only"));
            } else {
                log::debug!("constellations: drawing {} read and decoded in {:.1} ms", done.drawing, done.millis);
            }
        }
    }

    /// Each wanted or shown name's spot off its figure's stars at this turn
    /// of the sky (`name_goal`), chosen again only once the sky has turned
    /// it (`SPOT_TURN_COS`); it slides there at `SPOT_SPEED`, and takes its
    /// spot at once while faded out, so it never appears and then moves.
    /// Near the zenith it keeps the fixed drop (`name_hang`). Nothing runs
    /// while the names are off.
    fn name_spots(&mut self, active: &[usize], want: &[bool], s: &ConstellationSettings, dt: f32, zenith: Vec3) {
        let Some(set) = self.names.as_mut().filter(|_| s.names) else {
            return;
        };
        for &i in active {
            let Some(fig) = self.figures.get(i) else { continue };
            if !(want.get(i).copied().unwrap_or(false) || fig.shown()) {
                continue;
            }
            let (Some(Some(img)), Some(stars), Some(hang)) = (set.images.get(i), set.stars.get(i), set.hang.get(i)) else {
                continue;
            };
            let Some(spot) = set.spots.get_mut(i) else { continue };
            let axes = name_axes(spot.lp, zenith);
            // at rest on its spot, and the sky has not turned it: nothing to do
            let turned = axes.map_or(spot.right != Vec3::ZERO, |(right, _)| spot.right.dot(right) <= SPOT_TURN_COS);
            if spot.goal.is_some_and(|g| g == spot.at) && !turned {
                continue;
            }
            let b = NameBox::of(img);
            // its first spot taken at once: a name settles before it shows
            let first = spot.goal.is_none();
            let goal = match (axes, spot.goal) {
                (None, _) => {
                    // chosen again on leaving the zenith
                    spot.right = Vec3::ZERO;
                    Vec2::new(0.0, -hang)
                }
                (Some((right, _)), Some(g)) if spot.right.dot(right) > SPOT_TURN_COS => g,
                (Some((right, up)), g) => {
                    let flat: Vec<Vec2> = stars.iter().map(|q| Vec2::new(q.dot(right), q.dot(up))).collect();
                    spot.right = right;
                    name_goal(&b, &flat, g, spot_bound(img, &b))
                }
            };
            spot.goal = Some(goal);
            spot.at = if first || fig.name_alpha <= 0.0 {
                goal
            } else {
                slide(spot.at, goal, SPOT_SPEED * b.height() * dt.max(0.0))
            };
        }
    }

    /// The names drawn again when their size, the interface's scale or the
    /// language changes; their places in the atlas go with the old ones.
    /// A figure's name is drawn the first time it is wanted (`want`, among
    /// the listed figures `active`), `NAMES_DRAWN_PER_FRAME` a frame at
    /// most: the cultures not ticked draw none.
    fn ensure_names(&mut self, s: &ConstellationSettings, ctx: &egui::Context, language: &str, want: &[bool], active: &[usize]) {
        // (the fonts exist once the interface has run a pass: the viewer's
        // own window's, whatever a window drawn outside it counts)
        if !s.names || ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT) == 0 {
            return;
        }
        let Some(loaded) = &self.loaded else { return };
        let (font, scale) = (s.font_size.to_bits(), ctx.pixels_per_point().to_bits());
        if !self
            .names
            .as_ref()
            .is_some_and(|n| n.key.0 == font && n.key.1 == scale && n.key.2 == language)
        {
            if let Some(old) = &self.names {
                for i in 0..old.images.len() {
                    self.atlas.release(Item::Name(i as u32));
                }
            }
            let n = loaded.catalogue.figures.len().min(MAX_FIGURES);
            self.names = Some(NameSet {
                key: (font, scale, language.to_owned()),
                images: vec![None; n],
                hang: vec![0.0; n],
                stars: vec![Vec::new(); n],
                spots: vec![Spot::default(); n],
                drawn: vec![false; n],
            });
        }
        let Some(set) = self.names.as_mut() else { return };
        let todo: Vec<usize> = active
            .iter()
            .copied()
            .filter(|&i| want.get(i).copied().unwrap_or(false) && !set.drawn.get(i).copied().unwrap_or(true))
            .take(NAMES_DRAWN_PER_FRAME)
            .collect();
        if todo.is_empty() {
            return;
        }
        let texts: Vec<&str> = todo.iter().map(|&i| loaded.catalogue.figures[i].display_name(language)).collect();
        let images = names::rasterize(ctx, &texts, name_em_texels(s.font_size));
        for (&i, img) in todo.iter().zip(images) {
            let geom = &loaded.catalogue.figures[i].geom;
            set.hang[i] = img.as_ref().map_or(0.0, |img| name_hang(img, &figure_stars(geom)));
            set.stars[i] = figure_points(geom);
            set.spots[i] = Spot {
                lp: aurora_render::stars::equatorial(geom.label[0], geom.label[1]),
                ..Spot::default()
            };
            set.images[i] = img;
            set.drawn[i] = true;
        }
    }

    fn overlay(&self, input: &Input, s: &ConstellationSettings, uploads: Vec<Upload>) -> Overlay {
        let mut st = FrameState::default();
        let Some(loaded) = &self.loaded else {
            return Overlay::default();
        };
        // the tables of the cultures drawn (none yet: no figure shows, the
        // renderer keeps what it has, the planets' names go on)
        let view = self.view.as_deref();
        let b = view.map_or_else(Bases::default, |v| v.bases);
        use aurora_render::constellations::{HEAD_DRAWINGS, HEAD_SPARKLES, HEAD_TINTS, SHAPE_MIX};
        let mut bits = HEAD_DRAWINGS;
        if s.colour_variation {
            bits |= HEAD_TINTS;
        }
        if s.sparkles {
            bits |= HEAD_SPARKLES;
        }
        st.head = [u32::from(self.shown > 0), bits, b.labels, b.states];
        // the sparkles read the cell lists that reach around the lines
        st.bases = if s.sparkles {
            [b.wide_refs, b.drawings, b.segments, b.wide_cells]
        } else {
            [b.refs, b.drawings, b.segments, 0]
        };
        // the window's pixel at the view's centre (radians): the lines' width
        // there, in photos too (a photo's larger image keeps them at the
        // share of it the window shows)
        let pixel = 2.0 * (input.fov_y.clamp(0.01, 3.0) * 0.5).tan() / input.window_height.max(1) as f32;
        // names at their fixed size in the sky, each within NAME_REACH
        // (`name_texel`, per figure)
        let texel = NAME_TEXEL_DEGREES.to_radians();
        // the lines' width setting; never wider than the cells they are
        // listed in reach (a wide one would be cut at a cell's border)
        let (half, line_alpha) = line_look(pixel, s.line_width);
        st.look = [half.min(0.8 * LINE_REACH), texel, 0.0, 0.0];
        if s.sparkles {
            let (radius, light) = sparkle_look(pixel, s.line_width, s.sparkle_size);
            let shape = if self.sparkle_mix { SHAPE_MIX } else { s.sparkle_shape.code() };
            // its light against the lines' opacity, which carries the thin
            // lines' own fading
            st.style = [radius, s.sparkle_spin, shape, light / line_alpha.max(1e-3)];
        }
        let mut drawings = vec![[0.0f32; 4]; self.drawings.len()];
        for (d, dw) in self.drawings.iter().enumerate() {
            let f = loaded.figure_of.get(d).map_or(usize::MAX, |&f| f as usize);
            // a picture draws only once it is whole in the atlas
            let ready = self.loads.get(d).is_some_and(state::Load::ready);
            let (Some(fig), Some(p), true) = (self.figures.get(f), self.atlas.place_of(Item::Picture(d as u32)), ready) else {
                continue;
            };
            if dw.shown() && fig.shown() {
                let place = pack_place(p.layer, p.block[0], p.block[1]);
                // its light (ink and pen tip, added to the sky's) carries its
                // opacity setting, drawing brightness and pulse
                let light = state::part_gain(dw.reveal, dw.done, s.draw_brightness, s.highlight);
                drawings[d] = [
                    dw.reveal,
                    dw.alpha * fig.alpha * s.drawing_opacity * light,
                    0.0,
                    f32::from_bits(place),
                ];
            }
        }
        for (i, f) in self.figures.iter().enumerate().take(MAX_FIGURES) {
            if !f.shown() {
                continue;
            }
            // y: the lines' opacity (the shader reads it for the lines only),
            // with the thin lines' light, the drawing brightness and pulse
            let lines = self.lines_alpha.get(i).copied().unwrap_or(1.0);
            let light = state::part_gain(f.line_reveal(), f.lines_done, s.draw_brightness, s.highlight);
            let tint = if s.colour_variation {
                loaded.tints.get(i).copied().unwrap_or(0)
            } else {
                0
            };
            // x: its name's own texel
            let image = self.names.as_ref().and_then(|n| n.images.get(i)).and_then(Option::as_ref);
            // (its reach around its label point: a reading over it counts)
            let own = image.map_or(0.0, |img| name_texel(img.reach()));
            st.figures[i] = [own, f.alpha * lines * line_alpha * light, f.line_reveal(), f32::from_bits(tint)];
            let (place, dims) = match (self.atlas.place_of(Item::Name(i as u32)), image) {
                (Some(p), Some(img)) => (pack_place(p.layer, p.block[0], p.block[1]), img.dims()),
                _ => (0, 0),
            };
            let from = self.label_from.get(i).copied().unwrap_or(0);
            // a name on its figure's own label point goes off its stars
            // (`name_spots`); a figure whose drawing shows keeps it centred
            // when that drawing's set placed the point clear of its art
            // (every set used: the first drawing it has shows)
            let spot = self.names.as_ref().and_then(|n| n.spots.get(i)).map_or(Vec2::ZERO, |sp| sp.at);
            let drawn = s.drawings_on()
                && loaded
                    .catalogue
                    .figures
                    .get(i)
                    .is_some_and(catalogue::Figure::label_clear_of_drawing);
            let at = name_spot_word(from, spot, own, drawn);
            st.names[i] = [
                f32::from_bits(place),
                f32::from_bits(dims),
                at,
                f.name_alpha * f.alpha * (1.0 + if s.highlight { state::pulse(f.name_done) } else { 0.0 }),
            ];
        }
        self.planet_names_fill(&mut st);
        Overlay {
            state: Box::new(st),
            drawings,
            states_at: b.states,
            tables: view.map(|v| v.tables.clone()),
            tables_id: self.tables_id,
            layers: self.atlas.layers(),
            uploads,
            slots: view.map_or(0, |v| v.slots),
        }
    }

    /// A line when what shows changes (AURORA_PROFILE reads the part's
    /// time): the figures shown, the atlas's memory, the decoded pictures
    /// waiting for it. At info level only when something starts or stops
    /// showing or the atlas grows or shrinks; a new count of figures alone
    /// (each one crossing the field's edge as the view turns) is debug.
    fn log(&mut self) {
        // the names waiting for room in the atlas: info when some start or
        // stop waiting, debug for each new count
        if self.names_waiting != self.waiting_logged {
            let level = if (self.names_waiting > 0) != (self.waiting_logged > 0) {
                log::Level::Info
            } else {
                log::Level::Debug
            };
            log::log!(level, "constellations: {} names wait for room in the atlas", self.names_waiting);
            self.waiting_logged = self.names_waiting;
        }
        let now = (self.shown, self.atlas.layers());
        if now != self.logged {
            let level = if log_worthy(self.logged, now) {
                log::Level::Info
            } else {
                log::Level::Debug
            };
            self.logged = now;
            let ready = self.loads.iter().filter(|l| l.ready()).count();
            let waiting: usize = self
                .loads
                .iter()
                .map(|l| match l {
                    state::Load::Decoded { levels, .. } | state::Load::Uploading { levels, .. } => levels.iter().map(|v| v.len()).sum(),
                    _ => 0,
                })
                .sum();
            log::log!(
                level,
                "constellations: {} shown, {ready} pictures in the atlas ({} layer(s), {} MiB), {} KiB decoded waiting, {} KiB sent so far",
                self.shown,
                now.1,
                (now.1 as usize * aurora_render::constellations::ATLAS_LAYER_BYTES) >> 20,
                waiting / 1024,
                self.uploaded / 1024
            );
        }
    }
}

/// The overlay's line is worth the info level: something starts or stops
/// showing, or the atlas's memory changes (figures shown, atlas layers).
fn log_worthy(was: (usize, u32), now: (usize, u32)) -> bool {
    (was.0 == 0) != (now.0 == 0) || was.1 != now.1
}

fn smooth_name(reveal: f32) -> f32 {
    let t = ((reveal - state::NAME_FROM) / (state::NAME_TO - state::NAME_FROM)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_variables_set_the_launch_only() {
        assert_eq!(parse_overrides(None, None), Overrides::default());
        assert_eq!(parse_overrides(Some("0"), None).enabled, Some(false));
        assert_eq!(parse_overrides(Some(" 1 "), None).enabled, Some(true));
        assert_eq!(parse_overrides(Some("maybe"), None).enabled, None);
        let o = parse_overrides(None, Some("reveal=0.4, obsolete,nonames,field=70"));
        assert_eq!(o.enabled, Some(true));
        assert_eq!(o.hold, Some(Hold::Reveal(0.4)));
        assert_eq!(
            (o.obsolete, o.names, o.opacity, o.field),
            (Some(true), Some(false), None, Some(70.0))
        );
        assert_eq!(parse_overrides(None, Some("fade=7")).hold, Some(Hold::Fade(1.0)));
        assert_eq!(parse_overrides(None, Some("fade=x")).hold, None);
        // the A/B variable wins over the scenario
        assert_eq!(parse_overrides(Some("0"), Some("on")).enabled, Some(false));
        let s = parse_overrides(None, Some("field=500,font=nan")).apply(&ConstellationSettings::default());
        assert_eq!(s.field_degrees, 180.0);
        assert_eq!(s.font_size, 16.0);
        // the lines, the zenith view, the demo's rescan; the old `sets=`
        // item (every set is used now) changes nothing
        assert!(parse_overrides(None, Some("on,rescan")).rescan);
        let o = parse_overrides(None, Some("sets=second+first, nolines, zenith=Oct"));
        assert_eq!((o.lines, o.zenith.as_deref()), (Some(false), Some("Oct")));
        let s = o.apply(&ConstellationSettings::default());
        assert_eq!(
            s,
            ConstellationSettings {
                lines: false,
                ..ConstellationSettings::default()
            }
            .sane()
        );
        // the culture shown and the names' language, for the launch only;
        // the older `a+b` spelling is read like a stored list (its first id
        // the data has), and an empty one shows the western culture
        let o = parse_overrides(None, Some("cultures=sky-b, name_language=ko"));
        assert_eq!(
            (o.cultures.clone(), o.name_language.as_deref()),
            (Some(vec!["sky-b".to_owned()]), Some("ko"))
        );
        assert_eq!(o.apply(&ConstellationSettings::default()).cultures, o.cultures);
        let ids = ["western".to_owned(), "sky-b".to_owned(), "sky-c".to_owned()];
        let old = parse_overrides(None, Some("cultures=gone+sky-c+sky-b")).cultures;
        assert_eq!(catalogue::shown_culture(&ids, old.as_deref()), Some(2));
        let none = parse_overrides(None, Some("cultures=")).cultures;
        assert_eq!(
            catalogue::shown_culture(&ids, none.as_deref()),
            Some(0),
            "no culture: the western one"
        );
        assert_eq!(
            parse_overrides(None, Some("on")).apply(&ConstellationSettings::default()).cultures,
            None
        );
        // the new settings and the pulse's phase
        let o = parse_overrides(None, Some("figures=0.5,width=3,bright=2,highlight=0,pulse=0.5"));
        assert_eq!(
            (o.opacity, o.width, o.brightness, o.highlight),
            (Some(0.5), Some(3.0), Some(2.0), Some(false))
        );
        assert_eq!(o.hold, Some(Hold::Pulse(0.5)));
        assert_eq!(parse_overrides(None, Some("nodrawing")).opacity, Some(0.0));
        let s = parse_overrides(None, Some("width=9,bright=0.2,figures=-1")).apply(&ConstellationSettings::default());
        assert_eq!((s.line_width, s.draw_brightness, s.drawing_opacity), (3.0, 1.0, 0.0));
    }

    #[test]
    fn the_settings_read_back_what_they_write() {
        let s = ConstellationSettings {
            drawing_opacity: 0.4,
            names: false,
            font_size: 22.0,
            line_width: 2.0,
            ..ConstellationSettings::default()
        };
        let back: ConstellationSettings = serde_json::from_str(&serde_json::to_string(&s).expect("w")).expect("r");
        assert_eq!(back, s);
        // a file without them (or with some): the defaults for the others
        let read: ConstellationSettings = serde_json::from_str(r#"{"names": false}"#).expect("readable");
        assert_eq!(
            read,
            ConstellationSettings {
                names: false,
                ..ConstellationSettings::default()
            }
        );
    }

    #[test]
    fn an_older_file_with_sets_and_several_cultures_still_reads() {
        // the build before one culture at a time: the sets checked and the
        // cultures ticked; read whole, nothing dropped, nothing rewritten
        let json = serde_json::json!({
            "enabled": true,
            "sets": ["urania-1824"],
            "cultures": ["gone", "japanese", "chinese"],
            "font_size": 12.0
        });
        let read = crate::tolerant::from_value::<ConstellationSettings>(json, "test").expect("read");
        assert!(read.dropped.is_empty(), "{:?}", read.dropped);
        let s = read.value;
        assert_eq!(s.font_size, 12.0);
        assert_eq!(
            s.cultures.as_deref(),
            Some(&["gone".to_owned(), "japanese".to_owned(), "chinese".to_owned()][..])
        );
        let ids = ["western".to_owned(), "chinese".to_owned(), "japanese".to_owned()];
        assert_eq!(
            catalogue::shown_culture(&ids, s.cultures.as_deref()),
            Some(2),
            "its first id the data has"
        );
        // written back: no `sets` key, the list as it was until a pick
        let out = serde_json::to_value(&s).expect("written");
        assert!(out.get("sets").is_none());
        assert_eq!(out["cultures"], serde_json::json!(["gone", "japanese", "chinese"]));
        // 5 points now within the range; below, the smallest
        let small = ConstellationSettings {
            font_size: 2.0,
            ..ConstellationSettings::default()
        };
        assert_eq!(small.sane().font_size, *FONT_RANGE.start());
        let five = ConstellationSettings {
            font_size: 5.0,
            ..ConstellationSettings::default()
        };
        assert_eq!(five.sane().font_size, 5.0);
    }

    #[test]
    fn thin_lines_fade_instead_of_thinning() {
        let px = 0.001;
        // the default: as before the setting, at full light
        assert_eq!(line_look(px, 1.0), (0.7 * px, 1.0));
        let (h, a) = line_look(px, 3.0);
        assert!((h - 2.1 * px).abs() < 1e-9 && a == 1.0);
        // down to a pixel wide they thin; below, a pixel wide and dimmer, in
        // step with the width they would have had
        let (h, a) = line_look(px, 0.75);
        assert!((h - 0.525 * px).abs() < 1e-9 && a == 1.0);
        let (h, a) = line_look(px, 0.1);
        assert_eq!(h, 0.5 * px);
        assert!((a - 0.14).abs() < 1e-6, "{a}");
        // continuous where the rule changes
        let w = 0.5 / 0.7;
        let (h0, a0) = line_look(px, w - 1e-4);
        let (h1, a1) = line_look(px, w + 1e-4);
        assert!((h0 - h1).abs() < 1e-6 && (a0 - a1).abs() < 1e-3);
    }

    #[test]
    fn each_constellation_has_a_stable_light_pastel() {
        // FNV-1a's own test values
        assert_eq!(fnv1a(b""), 0x811c_9dc5);
        assert_eq!(fnv1a(b"a"), 0xe40c_292c);
        assert_eq!(fnv1a(b"foobar"), 0xbf9c_f968);
        // the same colour on every call, different ones for different
        // figures and for the same id in another culture
        let ori = pastel("western", "Ori");
        assert_eq!(ori, pastel("western", "Ori"));
        assert_ne!(ori, pastel("western", "UMa"));
        assert_ne!(ori, pastel("other", "Ori"));
        let ids = [
            "And", "Ant", "Aps", "Aql", "Aqr", "Ara", "Ari", "Aur", "Boo", "CMa", "CMi", "CVn", "Cae", "Cam", "Cap", "Car", "Cas", "Cen",
            "Cep", "Cet", "Cha", "Cir", "Cnc", "Col", "Com", "CrA", "CrB", "Crt", "Cru", "Crv", "Cyg", "Del", "Dor", "Dra", "Equ", "Eri",
            "For", "Gem", "Gru", "Her", "Hor", "Hya", "Hyi", "Ind", "LMi", "Lac", "Leo", "Lep", "Lib", "Lup", "Lyn", "Lyr", "Men", "Mic",
            "Mon", "Mus", "Nor", "Oct", "Oph", "Ori", "Pav", "Peg", "Per", "Phe", "Pic", "PsA", "Psc", "Pup", "Pyx", "Ret", "Scl", "Sco",
            "Sct", "Ser", "Sex", "Sge", "Sgr", "Tau", "Tel", "TrA", "Tri", "Tuc", "UMa", "UMi", "Vel", "Vir", "Vol", "Vul",
        ];
        let mut sectors = [0u32; 6];
        for id in ids {
            let c = pastel("western", id);
            let (top, low) = (c.iter().copied().fold(0.0f32, f32::max), c.iter().copied().fold(1.0f32, f32::min));
            // light and pale: the brightest channel the overlay's 1, the
            // others close to it (an HSV saturation under a quarter)
            assert!((top - 1.0).abs() < 1e-6, "{id}: {c:?}");
            assert!((0.75..0.9).contains(&low), "{id}: {c:?}");
            let hue = {
                let [r, g, b] = c;
                let d = top - low;
                let h = if r == top {
                    ((g - b) / d).rem_euclid(6.0)
                } else if g == top {
                    (b - r) / d + 2.0
                } else {
                    (r - g) / d + 4.0
                };
                h * 60.0
            };
            sectors[((hue / 60.0) as usize).min(5)] += 1;
        }
        // the hues spread round the circle: each sixth has its share
        assert!(sectors.iter().all(|n| *n >= 6), "{sectors:?}");
        // pinned: a build that changed the hash would change every colour
        let packed = aurora_render::constellations::pack_tint(pastel("western", "Ori"));
        assert_eq!(packed, PINNED_ORI, "{packed:#010x}");
    }

    /// `pack_tint(pastel("western", "Ori"))`, as computed.
    const PINNED_ORI: u32 = 0x3fcf_e8ff;

    #[test]
    fn a_sparkle_is_four_times_the_line_and_turns_with_it() {
        let px = 0.001;
        // x1: its radius is twice the line's whole width (0.7 px half-width,
        // x4), so it is four lines wide
        let (r, light) = sparkle_look(px, 1.0, 1.0);
        assert!((r - 2.8 * px).abs() < 1e-9 && light == 1.0);
        // the width setting and the size multiply it: x5 at the default width
        assert!((sparkle_look(px, 1.0, 5.0).0 - 14.0 * px).abs() < 1e-7);
        assert!((sparkle_look(px, 2.0, 3.0).0 - 16.8 * px).abs() < 1e-7);
        // never under a pixel: it fades instead
        let (r, light) = sparkle_look(px, 0.1, 1.0);
        assert_eq!(r, px);
        assert!((light - 0.28).abs() < 1e-6);
        // never past the cells its lines are listed in
        assert_eq!(sparkle_look(0.01, 3.0, 5.0).0, 0.9 * aurora_render::constellations::SPARKLE_REACH);
        // the settings: in range, from the demo's items
        let o = parse_overrides(None, Some("sparkles=star,sparkle_size=9,spin=2.5,colours"));
        let s = o.apply(&ConstellationSettings::default());
        assert!(s.sparkles && s.colour_variation);
        assert_eq!((s.sparkle_shape, s.sparkle_size, s.sparkle_spin), (SparkleShape::Star, 5.0, 2.5));
        assert!(parse_overrides(None, Some("sparkles=mix")).sparkle_mix);
        assert_eq!(parse_overrides(None, Some("sparkles")).sparkles, Some(SparkleShape::CircleCross));
        let d = ConstellationSettings::default();
        assert!(!d.sparkles && !d.colour_variation && d.sparkle_shape == SparkleShape::CircleCross);
        assert_eq!((d.sparkle_size, d.sparkle_spin), (1.0, 1.0));
        // stored by name
        let json = serde_json::to_value(ConstellationSettings {
            sparkle_shape: SparkleShape::Star,
            ..d.clone()
        })
        .expect("written");
        assert_eq!(json["sparkle_shape"], serde_json::json!("star"));
    }

    /// Each stored shape reaches the shader as its own number, the same as
    /// the demo's item gives it (the saved file and the test variable take
    /// one road: `SparkleShape::code`).
    #[test]
    fn each_saved_sparkle_shape_reaches_the_shader_as_its_number() {
        use aurora_render::constellations::{SHAPE_CIRCLE_CROSS, SHAPE_CROSS, SHAPE_STAR};
        for (saved, item, code) in [
            ("circle_cross", "circle", SHAPE_CIRCLE_CROSS),
            ("cross", "cross", SHAPE_CROSS),
            ("star", "star", SHAPE_STAR),
        ] {
            let read: ConstellationSettings =
                serde_json::from_value(serde_json::json!({ "sparkles": true, "sparkle_shape": saved })).expect("a saved shape reads");
            assert_eq!(read.sparkle_shape.code(), code, "{saved}");
            let demo = parse_overrides(None, Some(&format!("sparkles={item}"))).apply(&ConstellationSettings::default());
            assert_eq!(demo.sparkle_shape, read.sparkle_shape, "{item}");
            // written back as it was read
            assert_eq!(
                serde_json::to_value(&read).expect("written")["sparkle_shape"],
                serde_json::json!(saved)
            );
        }
    }

    #[test]
    fn only_a_start_a_stop_or_the_atlas_is_logged_at_info() {
        assert!(log_worthy((0, 0), (3, 1)));
        assert!(log_worthy((3, 1), (0, 1)));
        assert!(log_worthy((3, 1), (3, 2)));
        assert!(!log_worthy((3, 1), (4, 1)));
        assert!(!log_worthy((4, 1), (3, 1)));
    }

    #[test]
    fn the_zenith_view_puts_any_point_overhead() {
        use crate::world::night_sky::{NightSky, Observer, StarHour, star_frame};
        // a far southern point, a northern one, the south pole: each is
        // overhead in the real sky of the place found, by the stars' own
        // rotation
        let t = 20_468.75; // 2026-01-15 18:00 UTC
        for (ra, dec) in [(120.0f32, -75.0f32), (300.0, 40.0), (0.0, -89.9), (215.0, 5.0)] {
            let (latitude, longitude) = zenith_place(ra.to_radians(), dec.to_radians(), t);
            let frame = star_frame(
                NightSky::Real,
                Observer {
                    latitude,
                    longitude,
                    hour: StarHour::RealTime,
                },
                1.0,
                0.0,
                Vec3::NEG_Z,
                t,
                0.0,
            );
            let p = aurora_render::stars::equatorial(ra.to_radians(), dec.to_radians());
            let up = frame.axes[0] * p.x + frame.axes[1] * p.y + frame.axes[2] * p.z;
            assert!(up.z > 0.9999, "({ra}, {dec}): {up}");
            assert!((-180.0..=180.0).contains(&longitude));
        }
    }

    #[test]
    fn settings_keep_their_defaults_and_ranges() {
        let d = ConstellationSettings::default();
        assert!(d.enabled && d.drawing_opacity == 1.0 && d.lines && d.names && !d.obsolete);
        assert!(d.cultures.is_none(), "the western culture only");
        assert!(d.highlight && d.line_width == 1.0 && d.draw_brightness == 1.0);
        assert_eq!(
            (d.draw_seconds, d.fade_seconds, d.field_degrees, d.sun_below_degrees),
            (4.0, 3.0, 50.0, 8.0)
        );
        assert_eq!(*FADE_RANGE.end(), 10.0);
        let bad = ConstellationSettings {
            fade_seconds: 60.0,
            draw_seconds: f32::NAN,
            sun_below_degrees: -5.0,
            ..d.clone()
        }
        .sane();
        assert_eq!((bad.fade_seconds, bad.draw_seconds, bad.sun_below_degrees), (10.0, 4.0, 0.0));
        // an older settings file without them reads the defaults
        let read: ConstellationSettings = serde_json::from_str(r#"{"names": false}"#).expect("partial");
        assert_eq!(read, ConstellationSettings { names: false, ..d });
    }

    #[test]
    fn a_folder_loads_with_its_tables() {
        use catalogue::tests::{drawn, folder, set, write};
        let root = folder("tables");
        set(&root, "a", 0);
        set(&root, "b", 1);
        write(
            &root,
            "sets/a/Ori.json",
            drawn("Ori", r#","lines":[{"a":[78.6,-8.2],"b":[81.1,-2.4]}]"#)
                .replace(r#""lines":[],"#, "")
                .as_bytes(),
        );
        write(&root, "sets/a/Ori.webp", b"RIFF");
        write(&root, "sets/b/Ori.json", drawn("Ori", "").as_bytes());
        write(&root, "sets/b/Ori.webp", b"RIFF");
        write(&root, "sets/b/Cet.json", drawn("Cet", "").as_bytes());
        write(&root, "sets/b/Cet.webp", b"RIFF");
        let mut problems = Vec::new();
        let l = load(catalogue::scan(&root, &mut problems));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(l.points.len(), l.catalogue.figures.len());
        let v = view(&l, &[true]);
        assert_eq!((v.active.clone(), v.slots), (vec![0, 1], 2));
        let b = v.bases;
        assert!(b.refs < b.drawings && b.drawings < b.segments && b.segments < b.labels && b.labels < b.states);
        assert_eq!(v.tables.len() as u32, (b.states + b.drawing_count.max(1)) * 16);
        // no culture: the same numbers, nothing listed
        let none = view(&l, &[false]);
        assert!(none.active.is_empty() && none.slots == 0 && none.bases.longest == 0);
        assert_eq!(none.bases.drawing_count, b.drawing_count);
        // every drawing of every set has its number, after its figure's first
        let drawings: usize = l.catalogue.figures.iter().map(|f| f.drawings.len()).sum();
        assert_eq!((l.figure_of.len(), b.drawing_count as usize, drawings), (3, 3, 3));
        for (d, &f) in l.figure_of.iter().enumerate() {
            assert!(l.first[f as usize] as usize <= d);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_long_name_alone_is_smaller_within_its_reach() {
        let full = NAME_TEXEL_DEGREES.to_radians();
        // a short name: the full texel; a long one: its half-diagonal at
        // NAME_REACH, whatever the others
        assert_eq!(name_texel([300, 80]), full);
        let long = [1900, 80];
        let t = name_texel(long);
        assert!(t < full);
        assert!(((long[0] as f32).hypot(long[1] as f32) * 0.5 * t - NAME_REACH).abs() < 1e-5);
    }

    #[test]
    fn a_name_moves_off_its_own_stars_by_the_smallest_drop() {
        use aurora_render::constellations::Line;
        let deg = |d: f32| d.to_radians();
        let full = NAME_TEXEL_DEGREES.to_radians();
        let line = |a: [f32; 2], b: [f32; 2]| Line { a, b, t0: 0.0, t1: 1.0 };
        // one star (the data's line from the star to itself): at the label
        // point; two stars a degree each side: a degree each
        let label = [deg(37.95), deg(89.26)];
        let one = FigureGeom {
            centre: label,
            label,
            lines: vec![line(label, label)],
        };
        assert!(figure_stars(&one).iter().all(|d| *d < 1e-3), "{:?}", figure_stars(&one));
        let two = FigureGeom {
            centre: [deg(120.0), deg(10.0)],
            label: [deg(120.0), deg(10.0)],
            lines: vec![line([deg(119.0), deg(10.0)], [deg(121.0), deg(10.0)])],
        };
        let two = figure_stars(&two);
        assert!(two.iter().all(|d| (d - deg(0.985)).abs() < deg(0.01)), "{two:?}");
        // a name 192 texels wide with its spacing (3.84°) and 72 tall
        // (1.44°): its corner 2.05° from its centre
        let pad = names::SPACING + 4;
        let img = names::NameImage {
            size: [200, 80],
            levels: Vec::new(),
            lift: 0,
            pad,
        };
        let top = 40.0 - pad as f32 + names::SPACING as f32;
        assert_eq!((img.top(), img.bottom()), (top, top));
        let under = |d: f32| d + names::HANG_GAP + top * full;
        let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
        // the one star: the top of the text and spacing a gap under it
        assert!(near(name_hang(&img, &[0.0, 0.0]), under(0.0)));
        // two stars under the centred name: a gap under the nearer one, the
        // other (the same distance, to rounding) within that gap
        assert!(two[1] < two[0] + names::HANG_GAP);
        assert!(near(name_hang(&img, &two), under(two[0])));
        // no star under the box whatever its turn: centred, as before
        assert_eq!(name_hang(&img, &[deg(2.5), deg(3.0)]), 0.0);
        // a nearer star clears, the next one would come under the hung
        // name: the drop clears both
        assert!(near(name_hang(&img, &[deg(0.5), deg(1.5)]), under(deg(1.5))));
        // stars past the hung name's corner do not hold it lower
        assert!(near(name_hang(&img, &[deg(0.3), deg(3.0), deg(6.0)]), under(deg(0.3))));
        // no drop within twice its height clears them: a figure larger than
        // its name keeps it centred
        assert_eq!(name_hang(&img, &[deg(0.3), deg(2.5), deg(3.2), deg(6.0)]), 0.0);
        // a reading over the text: from the reading's top
        let read = names::NameImage {
            size: [200, 120],
            lift: 40,
            ..img.clone()
        };
        let top_read = 80.0 - pad as f32 + names::SPACING as f32;
        assert!(near(name_hang(&read, &[0.0]), names::HANG_GAP + top_read * full));
        // hung, a name stays within its reach at its own texel
        for (i, f) in [(&img, &[0.0][..]), (&img, &two[..]), (&read, &[0.0][..])] {
            let d = name_hang(i, f) / full;
            let tall = i.size[1] as f32 - i.lift as f32 + 2.0 * d;
            assert_eq!(name_texel([i.size[0], tall.ceil() as u32]), name_texel(i.reach()));
        }
        // a long name already at its reach: centred, never out of it
        let long = names::NameImage {
            size: [1900, 80],
            ..img.clone()
        };
        assert!(name_texel(long.reach()) < full);
        assert_eq!(name_hang(&long, &[0.0]), 0.0);
    }

    /// The offset env.wgsl reads from a name's centre word (radians, x right,
    /// y up), its own arithmetic: `None` for a label point's number.
    fn spot_of_word(word: f32, texel: f32) -> Option<Vec2> {
        let w = word.to_bits();
        ((w >> 31) != 0).then(|| Vec2::new(((w << 2) as i32 >> 18) as f32, ((w << 16) as i32 >> 16) as f32) * 0.25 * texel)
    }

    #[test]
    fn a_name_spot_stays_moves_to_the_nearest_clear_place_or_covers_the_fewest() {
        let b = NameBox {
            half: 1.0,
            top: 0.3,
            bottom: 0.3,
        };
        let near = |a: Vec2, c: Vec2| a.distance(c) < 1e-4;
        // nothing under it: centred
        assert_eq!(
            name_spot(&b, &[Vec2::new(0.0, 2.0), Vec2::new(3.0, 0.0)], 1.2, 0.0),
            (Vec2::ZERO, 0)
        );
        // a star on its label point: straight down by its top, the nearest
        let (o, left) = name_spot(&b, &[Vec2::ZERO], 1.2, 0.0);
        assert!(near(o, Vec2::new(0.0, -0.3)) && left == 0, "{o}");
        // a star a little under it: up is nearer than down
        let (o, _) = name_spot(&b, &[Vec2::new(0.0, -0.1)], 1.2, 0.0);
        assert!(near(o, Vec2::new(0.0, 0.2)), "{o}");
        // room kept round the stars
        let (o, _) = name_spot(&b, &[Vec2::ZERO], 1.2, 0.1);
        assert!(near(o, Vec2::new(0.0, -0.4)), "{o}");
        // stars below and above: a side, or a slant, the shortest clear move
        let stars = [Vec2::new(0.0, -0.1), Vec2::new(0.0, 0.1), Vec2::new(0.0, -0.5), Vec2::new(0.0, 0.5)];
        let (o, left) = name_spot(&b, &stars, 1.2, 0.0);
        assert_eq!(left, 0);
        assert_eq!(b.covered(o, &stars, 0.0), 0);
        assert!(o.length() <= 1.0 + 1e-4, "{o}");
        // a field of stars with no clear place within the bound: never
        // farther than the bound, and no more covered than centred
        let field: Vec<Vec2> = (-6..=6)
            .flat_map(|i| (-6..=6).map(move |j| Vec2::new(i as f32 * 0.25, j as f32 * 0.25)))
            .collect();
        let (o, left) = name_spot(&b, &field, 0.6, 0.0);
        assert!(
            o.length() <= 0.6 + 1e-4 && left > 0 && left <= b.covered(Vec2::ZERO, &field, 0.0),
            "{o} {left}"
        );
        assert_eq!(b.covered(o, &field, 0.0), left);
        // the spot kept: a clear one stays unless another is nearer by half
        // a height; a covering one goes
        let one = [Vec2::ZERO];
        assert!(near(name_goal(&b, &one, Some(Vec2::new(0.0, -0.6)), 1.2), Vec2::new(0.0, -0.6)));
        let best = name_goal(&b, &one, Some(Vec2::new(0.0, -1.1)), 1.2);
        assert!(near(best, Vec2::new(0.0, -0.3 - SPOT_PAD * 0.6)), "{best}");
        assert!(near(name_goal(&b, &one, Some(Vec2::new(0.0, -0.1)), 1.2), best));
        assert!(near(name_goal(&b, &one, None, 1.2), best));
        // a slide never jumps
        assert_eq!(slide(Vec2::ZERO, Vec2::new(0.0, -1.0), 0.25), Vec2::new(0.0, -0.25));
        assert_eq!(slide(Vec2::ZERO, Vec2::new(0.0, -0.1), 0.25), Vec2::new(0.0, -0.1));
        // level with the horizon: up is the zenith's side; none near it
        let (right, up) = name_axes(Vec3::X, Vec3::Z).expect("on the horizon");
        assert!(right.distance(-Vec3::Y) < 1e-6 && up.distance(Vec3::Z) < 1e-6, "{right} {up}");
        assert!(name_axes(Vec3::new(0.05, 0.0, 1.0).normalize(), Vec3::Z).is_none());
        // the word: a label point's number as before; an offset as env.wgsl
        // reads it, to a quarter texel; centred for a placed drawing
        let texel = 0.0003;
        assert_eq!(name_spot_word(3, Vec2::new(0.0, -0.01), texel, false).to_bits(), 3);
        assert_eq!(name_spot_word(0, Vec2::new(0.0, -0.01), texel, true).to_bits(), 0);
        assert_eq!(name_spot_word(0, Vec2::ZERO, texel, false).to_bits(), 0);
        assert_eq!(spot_of_word(f32::from_bits(5), texel), None);
        for o in [
            Vec2::new(0.0, -0.01),
            Vec2::new(0.02, 0.005),
            Vec2::new(-0.03, -0.04),
            Vec2::new(-1.0, 2.0),
            Vec2::new(-0.0001, -0.0001),
            Vec2::new(-0.001, 0.0),
        ] {
            let word = name_spot_word(0, o, texel, false);
            // never a NaN or an infinity: its bits reach the shader as they are
            assert!(word.is_finite(), "{o}");
            let back = spot_of_word(word, texel).expect("an offset");
            let clamped = Vec2::new(
                o.x.clamp(-2047.75 * texel, 2047.75 * texel),
                o.y.clamp(-8191.75 * texel, 8191.75 * texel),
            );
            assert!(back.distance(clamped) <= 0.2 * texel, "{o} {back}");
        }
        // env.wgsl reads the word the same way
        let wgsl = include_str!("../../../aurora-render/src/shaders/env.wgsl");
        assert!(wgsl.contains("vec2<f32>(f32(bitcast<i32>(word << 2u) >> 18u), f32(bitcast<i32>(word << 16u) >> 16u)) * 0.25"));
        assert!(wgsl.contains("/ texel + vec2<f32>(-spot.x, spot.y) +"));
    }

    #[test]
    fn a_turning_sky_slides_a_name_off_its_stars_without_flip_flop() {
        // a small figure under a wide name, turned through a full circle by
        // a quarter degree a frame (far faster than the sky)
        let b = NameBox {
            half: 1.2,
            top: 0.35,
            bottom: 0.3,
        };
        let stars = [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5, 0.2),
            Vec2::new(-0.6, -0.15),
            Vec2::new(0.2, -0.45),
        ];
        let bound = 2.0 * b.height();
        let (step, dt) = (0.25f32.to_radians(), 1.0 / 60.0);
        let speed = SPOT_SPEED * b.height() * dt;
        let (mut goal, mut at) = (None, Vec2::ZERO);
        let (mut switches, mut covered, mut since) = (0usize, 0usize, usize::MAX);
        for k in 0..(2.0 * std::f32::consts::PI / step) as usize + 1 {
            let th = k as f32 * step;
            let (r, u) = (Vec2::new(th.cos(), th.sin()), Vec2::new(-th.sin(), th.cos()));
            let flat: Vec<Vec2> = stars.iter().map(|s| Vec2::new(s.dot(r), s.dot(u))).collect();
            let g = name_goal(&b, &flat, goal, bound);
            since = since.saturating_add(1);
            if goal.is_some_and(|old: Vec2| old.distance(g) > 1e-6) {
                switches += 1;
                // a new spot holds for at least a degree of turn: no flip-flop
                assert!(since >= 4, "turn {k}: a new spot {since} steps after the last");
                since = 0;
            }
            goal = Some(g);
            covered += usize::from(b.covered(g, &flat, 0.0) > 0);
            assert!(g.length() <= bound + 1e-4);
            // the shown spot moves at the slide's pace, whatever the goal
            let next = if k == 0 { g } else { slide(at, g, speed) };
            assert!(k == 0 || next.distance(at) <= speed + 1e-6);
            at = next;
        }
        // the goal is always clear, and changes a handful of times a turn
        assert_eq!(covered, 0);
        assert!(switches <= 24, "{switches} switches in a turn");
        eprintln!("name spot sweep: {switches} switches in a turn");
    }

    #[test]
    fn the_status_line_gives_the_first_reason_nothing_shows() {
        let on = ConstellationSettings::default();
        let off = ConstellationSettings {
            enabled: false,
            ..on.clone()
        };
        let dir = std::path::Path::new("/profile/constellations");
        assert_eq!(status(&off, true, true, Some((true, dir))), Status::Off);
        assert_eq!(status(&on, false, true, Some((true, dir))), Status::NotRealSky);
        assert_eq!(status(&on, true, true, None), Status::Reading);
        assert_eq!(status(&on, true, false, Some((false, dir))), Status::NoData(dir.to_path_buf()));
        assert_eq!(status(&on, true, false, Some((true, dir))), Status::Daylight { below: 8.0 });
        assert_eq!(status(&on, true, true, Some((true, dir))), Status::Showing);
    }

    #[test]
    fn the_data_folder_is_the_profile_one_unless_a_launch_names_another() {
        // the configuration folder's (the unit tests' own under cfg(test))
        if std::env::var_os("AURORA_CONSTELLATION_DIR").is_none() {
            assert_eq!(data_dir(), crate::settings::config_dir().join("constellations"));
        }
    }
}
