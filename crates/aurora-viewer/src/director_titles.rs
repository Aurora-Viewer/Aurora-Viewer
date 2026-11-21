//! The Director's fades and title cards: a fade in from black at the start
//! of the track, a fade out to black at its end, and titles (a text of one
//! or more lines, shown at the start, at the end or over a time range, in
//! the centre or as a lower third, with a fade of their own).
//!
//! Aurora's own (the Legacy Viewer has no camera path tool). They are drawn
//! into the 3D picture, not over it by the interface: the film's frames are
//! taken before the interface is drawn (a live film) or drawn without it
//! (an offline render). Both are frame overlays of the renderer
//! (`aurora_render::OverlayEffect`, composed after the world's labels and
//! before the HUDs, as RLVa's are): a tint of the whole frame for a fade
//! (`Tint`, the step a crossfade between shots can use later), and a
//! rectangle of the frame showing a title card's picture, drawn on a thread
//! from the viewer's own fonts (`rasterize`).
//!
//! They are saved with the track (`SavedTrack`): the track's file as it was,
//! with a `finish` beside it that an older viewer ignores and an older file
//! does not have; the file's version does not change.

use crate::camera::track::TrackFile;
use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;

/// The seconds a fade can last (0: none).
pub const FADE_RANGE: RangeInclusive<f32> = 0.0..=10.0;
/// A title's size: its letters' height, a fraction of the picture's.
pub const SIZE_RANGE: RangeInclusive<f32> = 0.02..=0.2;
pub const DEFAULT_SIZE: f32 = 0.06;
/// A title's own fade, seconds.
pub const DEFAULT_TITLE_FADE: f32 = 0.5;
/// How long a title at the start or the end stays, by default, seconds.
pub const DEFAULT_TITLE_LENGTH: f32 = 3.0;
/// The titles a track can have.
pub const MAX_TITLES: usize = 16;
/// The characters a title can have.
pub const MAX_TEXT: usize = 400;

/// When a title shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum When {
    /// From the start of the track, `length` seconds.
    #[default]
    Start,
    /// The last `length` seconds of the track.
    End,
    /// From `from` to `to`, seconds of the track.
    Range,
}

/// Where a title stands in the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Place {
    #[default]
    Centre,
    /// Left, in the lower third: a name, a place.
    LowerThird,
}

/// A title card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Title {
    /// One or more lines.
    pub text: String,
    pub when: When,
    /// `Start` and `End`: seconds shown.
    pub length: f32,
    /// `Range`: seconds of the track it shows from and to.
    pub from: f32,
    pub to: f32,
    pub place: Place,
    /// The letters' height, a fraction of the picture's (`SIZE_RANGE`).
    pub size: f32,
    /// sRGB.
    pub color: [u8; 3],
    /// Seconds it takes to appear and to go (0: at once).
    pub fade: f32,
}

impl Default for Title {
    fn default() -> Self {
        Title {
            text: String::new(),
            when: When::Start,
            length: DEFAULT_TITLE_LENGTH,
            from: 0.0,
            to: DEFAULT_TITLE_LENGTH,
            place: Place::Centre,
            size: DEFAULT_SIZE,
            color: [255, 255, 255],
            fade: DEFAULT_TITLE_FADE,
        }
    }
}

/// The smooth ramp of a fade: 0 at `t` = 0, 1 at 1, flat at both ends.
fn ramp(t: f32) -> f32 {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 1.0 };
    t * t * (3.0 - 2.0 * t)
}

/// A seconds value from a file or a field: finite, within `range`.
fn seconds(value: f32, range: RangeInclusive<f32>) -> f32 {
    if value.is_finite() {
        value.clamp(*range.start(), *range.end())
    } else {
        *range.start()
    }
}

impl Title {
    /// Seconds of a track of `duration` it shows from and to.
    pub fn span(&self, duration: f32) -> (f32, f32) {
        let duration = duration.max(0.0);
        let length = seconds(self.length, 0.0..=crate::camera::track::MAX_TIME);
        match self.when {
            When::Start => (0.0, length.min(duration)),
            When::End => ((duration - length).max(0.0), duration),
            When::Range => {
                let from = seconds(self.from, 0.0..=crate::camera::track::MAX_TIME);
                let to = seconds(self.to, 0.0..=crate::camera::track::MAX_TIME);
                (from.min(to), from.max(to))
            }
        }
    }

    /// How much of it shows at `time` of a track of `duration`: 0 to 1,
    /// fading in and out over its own fade (at most half its span each).
    pub fn opacity(&self, time: f32, duration: f32) -> f32 {
        if self.text.trim().is_empty() || !time.is_finite() {
            return 0.0;
        }
        let (start, end) = self.span(duration);
        if time < start || time > end || end <= start {
            return 0.0;
        }
        let fade = seconds(self.fade, FADE_RANGE).min((end - start) * 0.5);
        if fade <= 0.0 {
            return 1.0;
        }
        // (a title at the very start or end of the track stays whole there:
        // the track's own fade, if any, does that part)
        let rise = if start <= 0.0 { 1.0 } else { ramp((time - start) / fade) };
        let fall = if end >= duration { 1.0 } else { ramp((end - time) / fade) };
        rise.min(fall)
    }

    /// The size, within its range.
    pub fn size(&self) -> f32 {
        if self.size.is_finite() {
            self.size.clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end())
        } else {
            DEFAULT_SIZE
        }
    }

    /// Its text as it is drawn: the lines, without the blanks around them,
    /// within `MAX_TEXT` characters.
    pub fn lines(&self) -> Vec<String> {
        let text: String = self.text.chars().take(MAX_TEXT).collect();
        text.lines().map(|line| line.trim().to_owned()).collect()
    }
}

/// The fades and titles of a track.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Finish {
    /// Seconds of the fade from black at the start (0: none).
    pub fade_in: f32,
    /// Seconds of the fade to black at the end (0: none).
    pub fade_out: f32,
    pub titles: Vec<Title>,
}

impl Finish {
    pub fn is_empty(&self) -> bool {
        *self == Finish::default()
    }

    /// How black the picture is at `time` of a track of `duration`: 1 at
    /// the start of a fade in, 1 at the end of a fade out, 0 in between.
    pub fn black(&self, time: f32, duration: f32) -> f32 {
        if !time.is_finite() {
            return 0.0;
        }
        let fade_in = seconds(self.fade_in, FADE_RANGE);
        let fade_out = seconds(self.fade_out, FADE_RANGE);
        let into = if fade_in > 0.0 { 1.0 - ramp(time / fade_in) } else { 0.0 };
        let out = if fade_out > 0.0 && duration > 0.0 {
            ramp((time - (duration - fade_out)) / fade_out)
        } else {
            0.0
        };
        into.max(out)
    }

    /// A finish read from a file: its values within their ranges, as many
    /// titles as a track can have.
    pub fn sanitized(mut self) -> Finish {
        self.fade_in = seconds(self.fade_in, FADE_RANGE);
        self.fade_out = seconds(self.fade_out, FADE_RANGE);
        self.titles.truncate(MAX_TITLES);
        for title in &mut self.titles {
            title.text = title.text.chars().take(MAX_TEXT).collect();
            title.size = title.size();
            title.fade = seconds(title.fade, FADE_RANGE);
        }
        self
    }
}

/// A tint of the whole frame: a color laid over it at an opacity. A fade is
/// one (black); a crossfade between two shots can be made of the same step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tint {
    /// Linear RGB.
    pub color: [f32; 3],
    /// 0 (nothing) to 1 (the color alone).
    pub alpha: f32,
}

impl Tint {
    pub fn black(alpha: f32) -> Tint {
        Tint { color: [0.0; 3], alpha }
    }

    /// The renderer's overlay for it (the renderer's white texture tinted),
    /// `None` when it shows nothing.
    pub fn effect(self) -> Option<aurora_render::OverlayEffect> {
        let alpha = if self.alpha.is_finite() { self.alpha.clamp(0.0, 1.0) } else { 0.0 };
        (alpha > 1e-3).then(|| aurora_render::OverlayEffect {
            color: [self.color[0], self.color[1], self.color[2], alpha],
            texture: [aurora_render::textures::WHITE, 0, 0, 0],
        })
    }
}

/// The part of a frame of `frame` (width, height) a title's card covers,
/// from its top left, 0 to 1: `card` is the card's picture size, drawn for
/// a picture `drawn_for` pixels high; `picture` is where the picture is in
/// the frame (left, top, width, height, pixels: the format's part of it).
pub fn card_rect(place: Place, card: (u32, u32), drawn_for: u32, picture: [f32; 4], frame: (f32, f32)) -> ([f32; 2], [f32; 2]) {
    let [left, top, width, height] = picture;
    let scale = height / drawn_for.max(1) as f32;
    let (w, h) = (card.0 as f32 * scale, card.1 as f32 * scale);
    let (x, y) = match place {
        Place::Centre => (left + (width - w) * 0.5, top + (height - h) * 0.5),
        Place::LowerThird => (left + width * 0.07, top + height * 0.72 - h * 0.5),
    };
    let (fw, fh) = (frame.0.max(1.0), frame.1.max(1.0));
    ([x / fw, y / fh], [(x + w) / fw, (y + h) / fh])
}

/// The picture of a title card: its size and RGBA (white letters, their
/// coverage in the alpha; the title's color is the overlay's).
pub type Card = (u32, u32, Vec<u8>);

/// The longest side of a card's picture (a page of the texture table).
const CARD_MAX: u32 = 4096;
/// Transparent pixels around the letters (the sampler's filter at the edges).
const CARD_PAD: f32 = 4.0;

/// The fonts a title is drawn with: the interface's own (Inter, Noto Sans,
/// bundled), then the system's for the scripts they lack (Japanese…).
fn fonts() -> std::sync::Arc<resvg::usvg::fontdb::Database> {
    static DB: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> = std::sync::OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_font_data(crate::ui::fonts::INTER_SEMIBOLD_TTF.to_vec());
        db.load_font_data(crate::ui::fonts::NOTO_SEMIBOLD_TTF.to_vec());
        db.load_system_fonts();
        std::sync::Arc::new(db)
    })
    .clone()
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The SVG of a title's lines at `font_px` (letters' height in pixels),
/// centred on x = 0 for the centre, from x = 0 for a lower third.
pub fn svg(lines: &[String], place: Place, font_px: f32) -> String {
    let anchor = match place {
        Place::Centre => "middle",
        Place::LowerThird => "start",
    };
    let line_height = font_px * 1.25;
    let mut body = String::new();
    for (i, line) in lines.iter().enumerate() {
        body.push_str(&format!(
            "<tspan x=\"0\" y=\"{:.1}\">{}</tspan>",
            font_px + i as f32 * line_height,
            escape(if line.is_empty() { " " } else { line })
        ));
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><text font-family=\"Inter, Noto Sans, sans-serif\" \
         font-weight=\"600\" font-size=\"{font_px:.1}\" text-anchor=\"{anchor}\" fill=\"#ffffff\">{body}</text></svg>"
    )
}

/// Draw a title's card for a picture `picture_height` pixels high: CPU
/// work, on a thread. `None` for an empty title, or text that draws nothing.
pub fn rasterize(title: &Title, picture_height: u32) -> Option<Card> {
    let lines = title.lines();
    if lines.iter().all(String::is_empty) {
        return None;
    }
    let mut font_px = (title.size() * picture_height as f32).max(4.0);
    let options = resvg::usvg::Options {
        fontdb: fonts(),
        font_family: "Inter".to_owned(),
        ..Default::default()
    };
    for _ in 0..3 {
        let source = svg(&lines, title.place, font_px);
        let tree = resvg::usvg::Tree::from_str(&source, &options).ok()?;
        let bounds = tree.root().abs_stroke_bounding_box();
        let (w, h) = (bounds.width() + 2.0 * CARD_PAD, bounds.height() + 2.0 * CARD_PAD);
        if !(w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 {
            return None;
        }
        // too wide for a texture: smaller letters
        let longest = w.max(h);
        if longest > CARD_MAX as f32 {
            font_px *= CARD_MAX as f32 / longest * 0.98;
            continue;
        }
        let (pw, ph) = (w.ceil() as u32, h.ceil() as u32);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(pw, ph)?;
        let transform = resvg::tiny_skia::Transform::from_translate(CARD_PAD - bounds.x(), CARD_PAD - bounds.y());
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        // premultiplied white: the coverage is the alpha, the color white
        let rgba = pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| [255, 255, 255, px[3]])
            .collect();
        return Some((pw, ph, rgba));
    }
    None
}

/// A track's file with its fades and titles beside it (`finish`): the
/// track's own fields as they were, read and written by `TrackFile`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTrack {
    #[serde(flatten)]
    pub track: TrackFile,
    #[serde(default, skip_serializing_if = "Finish::is_empty")]
    pub finish: Finish,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_goes_from_black_and_to_black() {
        let finish = Finish {
            fade_in: 2.0,
            fade_out: 1.0,
            ..Default::default()
        };
        assert_eq!(finish.black(0.0, 10.0), 1.0);
        assert!((finish.black(1.0, 10.0) - 0.5).abs() < 1e-6, "half way");
        assert_eq!(finish.black(2.0, 10.0), 0.0);
        assert_eq!(finish.black(5.0, 10.0), 0.0);
        assert!((finish.black(9.5, 10.0) - 0.5).abs() < 1e-6);
        assert_eq!(finish.black(10.0, 10.0), 1.0);
        // the hold past the end stays black
        assert_eq!(finish.black(11.0, 10.0), 1.0);
        // off: nothing
        assert_eq!(Finish::default().black(0.0, 10.0), 0.0);
        assert_eq!(Finish::default().black(10.0, 10.0), 0.0);
        // a short track: both fades meet, never more than black
        let short = Finish {
            fade_in: 3.0,
            fade_out: 3.0,
            ..Default::default()
        };
        for t in [0.0, 0.5, 1.0, 1.5, 2.0] {
            let b = short.black(t, 2.0);
            assert!((0.0..=1.0).contains(&b));
        }
        assert_eq!(finish.black(f32::NAN, 10.0), 0.0);
        // the curve is smooth: monotonic, flat at its ends
        let mut last = 1.0;
        for i in 0..=20 {
            let b = finish.black(i as f32 * 0.1, 10.0);
            assert!(b <= last + 1e-6);
            last = b;
        }
        assert!(finish.black(0.02, 10.0) > 0.999);
    }

    #[test]
    fn a_title_shows_at_its_time_and_fades_in_and_out() {
        let mut title = Title {
            text: "Aurora".to_owned(),
            ..Default::default()
        };
        // at the start, three seconds: whole at once at the start
        assert_eq!(title.span(10.0), (0.0, 3.0));
        assert_eq!(title.opacity(0.0, 10.0), 1.0);
        assert!((title.opacity(2.75, 10.0) - 0.5).abs() < 1e-6, "fading out");
        assert_eq!(title.opacity(3.5, 10.0), 0.0);
        // at the end: fades in, whole at the very end
        title.when = When::End;
        assert_eq!(title.span(10.0), (7.0, 10.0));
        assert_eq!(title.opacity(6.9, 10.0), 0.0);
        assert!((title.opacity(7.25, 10.0) - 0.5).abs() < 1e-6);
        assert_eq!(title.opacity(10.0, 10.0), 1.0);
        // a range, in either order, its fade at most half of it
        title.when = When::Range;
        title.from = 6.0;
        title.to = 4.0;
        title.fade = 5.0;
        assert_eq!(title.span(10.0), (4.0, 6.0));
        assert_eq!(title.opacity(5.0, 10.0), 1.0);
        assert!(title.opacity(4.5, 10.0) > 0.0 && title.opacity(4.5, 10.0) < 1.0);
        // no fade: at once
        title.fade = 0.0;
        assert_eq!(title.opacity(4.0, 10.0), 1.0);
        // longer than the track: the whole track
        title.when = When::Start;
        title.length = 30.0;
        assert_eq!(title.span(10.0), (0.0, 10.0));
        // an empty title never shows
        title.text = "  ".to_owned();
        assert_eq!(title.opacity(1.0, 10.0), 0.0);
        // a file's odd values
        title.size = f32::NAN;
        assert_eq!(title.size(), DEFAULT_SIZE);
        title.size = 9.0;
        assert_eq!(title.size(), *SIZE_RANGE.end());
    }

    #[test]
    fn a_title_card_stands_in_the_picture() {
        // a 2.39:1 picture in a 1920x1080 frame, a card drawn for 803 px
        let picture = [0.0, 138.5, 1920.0, 803.0];
        let (min, max) = card_rect(Place::Centre, (400, 100), 803, picture, (1920.0, 1080.0));
        assert!((min[0] + max[0] - 1.0).abs() < 1e-5, "centred across");
        assert!(((min[1] + max[1]) * 0.5 - 0.5).abs() < 1e-5, "and down");
        assert!((max[0] - min[0] - 400.0 / 1920.0).abs() < 1e-5);
        // drawn for a taller picture: scaled down
        let (min, max) = card_rect(Place::Centre, (400, 100), 1606, picture, (1920.0, 1080.0));
        assert!((max[0] - min[0] - 200.0 / 1920.0).abs() < 1e-5);
        // a lower third: on the left, below the middle
        let (min, max) = card_rect(Place::LowerThird, (400, 100), 803, picture, (1920.0, 1080.0));
        assert!(min[0] < 0.1 && max[0] < 0.5);
        assert!(min[1] > 0.5);
    }

    #[test]
    fn a_fade_is_a_tint_of_the_frame() {
        assert!(Tint::black(0.0).effect().is_none());
        let effect = Tint::black(0.5).effect().expect("shown");
        assert_eq!(effect.color, [0.0, 0.0, 0.0, 0.5]);
        assert_eq!(effect.texture[0], aurora_render::textures::WHITE);
        assert_eq!(effect.texture[3], 0, "the whole frame");
        assert!(Tint::black(f32::NAN).effect().is_none());
        assert_eq!(Tint::black(7.0).effect().expect("shown").color[3], 1.0);
    }

    #[test]
    fn a_title_is_written_as_svg_with_its_text_escaped() {
        let lines = vec!["A & B".to_owned(), "<C>".to_owned(), String::new()];
        let svg = svg(&lines, Place::Centre, 40.0);
        assert!(svg.contains("A &amp; B") && svg.contains("&lt;C&gt;"));
        assert!(svg.contains("text-anchor=\"middle\""));
        assert_eq!(svg.matches("<tspan").count(), 3);
        assert!(super::svg(&lines, Place::LowerThird, 40.0).contains("text-anchor=\"start\""));
        let title = Title {
            text: "  first  \nsecond".to_owned(),
            ..Default::default()
        };
        assert_eq!(title.lines(), ["first", "second"]);
    }

    #[test]
    fn a_title_card_is_drawn_from_the_viewer_fonts() {
        let title = Title {
            text: "Aurora\nViewer".to_owned(),
            ..Default::default()
        };
        let (w, h, rgba) = rasterize(&title, 1080).expect("a card");
        assert_eq!(rgba.len(), w as usize * h as usize * 4);
        // two lines of 65 px letters: taller than one, letters drawn
        assert!(h > 120 && w > 100, "{w}x{h}");
        assert!(rgba.as_chunks::<4>().0.iter().any(|px| px[3] == 255));
        assert!(rgba.as_chunks::<4>().0.iter().all(|px| px[0] == 255));
        // twice the picture, twice the card
        let (w2, ..) = rasterize(&title, 2160).expect("a card");
        assert!((f64::from(w2) / f64::from(w) - 2.0).abs() < 0.1);
        assert_eq!(rasterize(&Title::default(), 1080), None);
    }

    #[test]
    fn fades_and_titles_are_saved_beside_the_track_and_older_files_load() {
        let old = r#"{"version":2,"linear":false,"ease_ends":true,"keys":[]}"#;
        let saved: SavedTrack = serde_json::from_str(old).expect("an older file");
        assert!(saved.finish.is_empty());
        assert_eq!(saved.track.version, 2);
        // without fades and titles the file is written as before
        assert_eq!(serde_json::to_string(&saved).expect("json"), old);
        let with = SavedTrack {
            finish: Finish {
                fade_in: 1.5,
                fade_out: 2.0,
                titles: vec![Title {
                    text: "The end".to_owned(),
                    when: When::End,
                    place: Place::LowerThird,
                    color: [250, 200, 10],
                    ..Default::default()
                }],
            },
            ..saved
        };
        let json = serde_json::to_string(&with).expect("json");
        let back: SavedTrack = serde_json::from_str(&json).expect("read back");
        assert_eq!(back, with);
        // the track's own reader still reads it (an older viewer ignores it)
        let track: TrackFile = serde_json::from_str(&json).expect("the track's fields");
        assert_eq!(track, with.track);
        // a file's odd values are brought within their ranges
        let odd: Finish = serde_json::from_str(r#"{"fade_in":-3,"fade_out":99,"titles":[{"size":5}]}"#).expect("finish");
        let odd = odd.sanitized();
        assert_eq!((odd.fade_in, odd.fade_out), (0.0, *FADE_RANGE.end()));
        assert_eq!(odd.titles[0].size, *SIZE_RANGE.end());
        assert_eq!(odd.titles[0].color, [255, 255, 255], "missing fields take their defaults");
    }
}
