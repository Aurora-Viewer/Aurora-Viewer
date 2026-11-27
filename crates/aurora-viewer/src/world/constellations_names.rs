//! The constellations' names: the 88's catalogue keys (`constellation-<id>`,
//! in English, French and Japanese, community languages showing English;
//! any other figure shows the name its data gives),
//! and their pictures for the sky shader, drawn with the interface's own
//! fonts (their fallbacks included, so Japanese names show) into a
//! two-channel image: the text's coverage (red) and a soft glow around it
//! (green), both faded to zero before the picture's edges. A name has a fixed
//! size in the sky (about 1.2° at the default 16 points, centred on its
//! figure's label point, or hung under a figure smaller than itself:
//! constellations.rs `name_hang`), drawn once per font
//! size and language at 0.02° a texel, half the engravings' texel, with
//! two halved levels below: the window and photos read the level of their
//! own pixel's size, so a name is as sharp as its drawing at any zoom.

use std::sync::Arc;

/// Figure id -> its name's key, sorted by id.
const KEYS: &[(&str, &str)] = &[
    ("And", "constellation-and"),
    ("Ant", "constellation-ant"),
    ("Aps", "constellation-aps"),
    ("Aql", "constellation-aql"),
    ("Aqr", "constellation-aqr"),
    ("Ara", "constellation-ara"),
    ("Ari", "constellation-ari"),
    ("Aur", "constellation-aur"),
    ("Boo", "constellation-boo"),
    ("CMa", "constellation-cma"),
    ("CMi", "constellation-cmi"),
    ("CVn", "constellation-cvn"),
    ("Cae", "constellation-cae"),
    ("Cam", "constellation-cam"),
    ("Cap", "constellation-cap"),
    ("Car", "constellation-car"),
    ("Cas", "constellation-cas"),
    ("Cen", "constellation-cen"),
    ("Cep", "constellation-cep"),
    ("Cet", "constellation-cet"),
    ("Cha", "constellation-cha"),
    ("Cir", "constellation-cir"),
    ("Cnc", "constellation-cnc"),
    ("Col", "constellation-col"),
    ("Com", "constellation-com"),
    ("CrA", "constellation-cra"),
    ("CrB", "constellation-crb"),
    ("Crt", "constellation-crt"),
    ("Cru", "constellation-cru"),
    ("Crv", "constellation-crv"),
    ("Cyg", "constellation-cyg"),
    ("Del", "constellation-del"),
    ("Dor", "constellation-dor"),
    ("Dra", "constellation-dra"),
    ("Equ", "constellation-equ"),
    ("Eri", "constellation-eri"),
    ("For", "constellation-for"),
    ("Gem", "constellation-gem"),
    ("Gru", "constellation-gru"),
    ("Her", "constellation-her"),
    ("Hor", "constellation-hor"),
    ("Hya", "constellation-hya"),
    ("Hyi", "constellation-hyi"),
    ("Ind", "constellation-ind"),
    ("LMi", "constellation-lmi"),
    ("Lac", "constellation-lac"),
    ("Leo", "constellation-leo"),
    ("Lep", "constellation-lep"),
    ("Lib", "constellation-lib"),
    ("Lup", "constellation-lup"),
    ("Lyn", "constellation-lyn"),
    ("Lyr", "constellation-lyr"),
    ("Men", "constellation-men"),
    ("Mic", "constellation-mic"),
    ("Mon", "constellation-mon"),
    ("Mus", "constellation-mus"),
    ("Nor", "constellation-nor"),
    ("Oct", "constellation-oct"),
    ("Oph", "constellation-oph"),
    ("Ori", "constellation-ori"),
    ("Pav", "constellation-pav"),
    ("Peg", "constellation-peg"),
    ("Per", "constellation-per"),
    ("Phe", "constellation-phe"),
    ("Pic", "constellation-pic"),
    ("PsA", "constellation-psa"),
    ("Psc", "constellation-psc"),
    ("Pup", "constellation-pup"),
    ("Pyx", "constellation-pyx"),
    ("Ret", "constellation-ret"),
    ("Scl", "constellation-scl"),
    ("Sco", "constellation-sco"),
    ("Sct", "constellation-sct"),
    ("Ser", "constellation-ser"),
    ("Sex", "constellation-sex"),
    ("Sge", "constellation-sge"),
    ("Sgr", "constellation-sgr"),
    ("Tau", "constellation-tau"),
    ("Tel", "constellation-tel"),
    ("TrA", "constellation-tra"),
    ("Tri", "constellation-tri"),
    ("Tuc", "constellation-tuc"),
    ("UMa", "constellation-uma"),
    ("UMi", "constellation-umi"),
    ("Vel", "constellation-vel"),
    ("Vir", "constellation-vir"),
    ("Vol", "constellation-vol"),
    ("Vul", "constellation-vul"),
];

/// The catalogue key of a figure's name.
pub fn key(id: &str) -> Option<&'static str> {
    KEYS.binary_search_by(|(k, _)| (*k).cmp(id)).ok().map(|i| KEYS[i].1)
}

/// A name's picture: `Rg8` rows, the full level then its halves.
#[derive(Clone, Debug)]
pub struct NameImage {
    pub size: [u32; 2],
    pub levels: Vec<Arc<Vec<u8>>>,
    /// Rows the picture's centre lies above its base text's centre,
    /// doubled: the rows of a reading over the base (0 without one). The
    /// base's centre sits on the label point, so that names with and
    /// without a reading line up.
    pub lift: u32,
    /// Texels of room around its ink on every side (`pad`): its glow and
    /// halved levels fade to zero within it.
    pub pad: u32,
}

// the figures' size word (`NameImage::dims`): 12 bits of width, 10 of
// height, 10 of lift; a picture is at most MAX_W x 512 (`compose`) and its
// lift no more than its height
const _: () = assert!(MAX_W < 1 << 12 && 512 < 1 << 10);

impl NameImage {
    /// Its width, height and lift in one word, for the figures' names
    /// (constellations.wgsl `constellation_overlay` reads it back).
    pub fn dims(&self) -> u32 {
        (self.size[0] & 0xfff) | (self.size[1] & 0x3ff) << 12 | (self.lift & 0x3ff) << 22
    }

    /// The size of a picture centred on its label point that reaches as far
    /// as this one does around its base's centre (its `NAME_REACH`).
    pub fn reach(&self) -> [u32; 2] {
        [self.size[0], self.size[1] + self.lift]
    }

    /// Texels from its base's centre (the label point) up to the top of its
    /// ink and `SPACING`: where a name hung under a star or a planet starts.
    pub fn top(&self) -> f32 {
        (self.size[1] + self.lift) as f32 * 0.5 - self.pad as f32 + SPACING as f32
    }

    /// Texels from its base's centre down to the bottom of its ink and
    /// `SPACING`.
    pub fn bottom(&self) -> f32 {
        self.size[1].saturating_sub(self.lift) as f32 * 0.5 - self.pad as f32 + SPACING as f32
    }

    /// Texels across its ink and `SPACING` on both sides.
    pub fn spaced(&self) -> [u32; 2] {
        let room = 2 * self.pad.saturating_sub(SPACING);
        [self.size[0].saturating_sub(room), self.size[1].saturating_sub(room)]
    }
}

/// Texels a name's place keeps around its ink: the bright part of its glow.
/// The planets' names' gap and stacking and a figure's name hung under it
/// count from it, so that they stay where they were whatever room the
/// picture itself needs (`pad`, wider).
pub const SPACING: u32 = 6;
/// The gap between a star or a planet and the top of the name hung under
/// it (radians, 0.35°, to its `SPACING`).
pub const HANG_GAP: f32 = 0.006;

/// The glow's radius (texels) about a base text `base_h` texels tall: the
/// two box blurs of `blur`.
fn glow_radius(base_h: usize) -> usize {
    base_h / 8 + 1
}

/// The room around a name's ink (texels) for a base text `base_h` texels
/// tall. Two box blurs of radius r reach 2r: the glow is zero past it, never
/// cut at the picture's edge (a cut glow drew the picture's rectangle round
/// a bright name). Then as many texels as the last halved level's outermost
/// texel averages (`1 << (ATLAS_MIPS - 1)`), so that it is zero too, and
/// the shader, which reads no nearer the edge than half of it, never takes
/// what lies outside the picture in the atlas (a gutter, or what an earlier
/// item left there).
fn pad(base_h: usize) -> usize {
    2 * glow_radius(base_h) + (1 << (aurora_render::constellations::ATLAS_MIPS - 1))
}

/// The width (points) a laid-out text whose base is `base` may take: the
/// widest picture less its room (`pad`, from the base's ink, with a
/// texel's rounding to spare; a text laid out smaller to fit needs less).
fn room(base: &egui::Galley, ppp: f32) -> f32 {
    let tall = ink_rows(base).map_or(0, |[top, bottom]| ((bottom - top) * ppp).ceil().max(0.0) as usize + 1);
    MAX_W.saturating_sub(2 * pad(tall)) as f32 / ppp
}
/// A name's picture is this wide at most (`rasterize` lays a longer name
/// out smaller).
const MAX_W: usize = 2048;

/// The pictures of `texts`, their font's em `em` texels high. `None` for
/// a text with no glyph (an empty name).
pub fn rasterize(ctx: &egui::Context, texts: &[&str], em: f32) -> Vec<Option<NameImage>> {
    let ppp = ctx.pixels_per_point().max(0.25);
    let font = egui::FontId::proportional((em / ppp).clamp(4.0, 200.0));
    ctx.fonts_mut(|fonts| {
        // a name wider than a picture may be is laid out again smaller, so
        // that it fits (it is drawn smaller in the sky anyway: its reach)
        // a name that ends with its kana reading: its base on the line and
        // the reading over its kanji (`lay_ruby`); any other name, or one
        // whose reading cannot be laid out, as before
        let rubies: Vec<Option<RubyLayout>> = texts
            .iter()
            .map(|t| super::readings::ruby(t).and_then(|r| lay_ruby(fonts, &r, &font, ppp)))
            .collect();
        let galleys: Vec<_> = texts
            .iter()
            .zip(&rubies)
            .map(|(t, ruby)| {
                if let Some(r) = ruby {
                    return r.base.clone();
                }
                let g = fonts.layout_no_wrap((*t).to_owned(), font.clone(), egui::Color32::WHITE);
                let room = room(&g, ppp);
                if g.size().x <= room {
                    return g;
                }
                let smaller = egui::FontId::proportional((font.size * room / g.size().x * 0.98).max(1.0));
                fonts.layout_no_wrap((*t).to_owned(), smaller, egui::Color32::WHITE)
            })
            .collect();
        // after the layouts: the atlas holds every glyph they use
        let atlas = fonts.image();
        galleys
            .iter()
            .zip(&rubies)
            .map(|(g, ruby)| {
                let glyphs = glyphs_of(g, egui::Vec2::ZERO, ppp);
                let coverage = |x: usize, y: usize| atlas.pixels.get(y * atlas.size[0] + x).map_or(0, |c| c.a());
                match ruby {
                    None => compose(&glyphs, coverage),
                    Some(r) => {
                        let over: Vec<Glyph> = r.readings.iter().flat_map(|(rg, at)| glyphs_of(rg, *at, ppp)).collect();
                        compose_parts(&glyphs, &over, coverage)
                    }
                }
            })
            .collect()
    })
}

/// The glyphs of a laid-out text moved by `at` (points), in pixels.
fn glyphs_of(g: &egui::Galley, at: egui::Vec2, ppp: f32) -> Vec<Glyph> {
    g.rows
        .iter()
        .flat_map(|placed| {
            placed.row.glyphs.iter().filter(|gl| !gl.uv_rect.is_nothing()).map(move |gl| {
                let p = placed.pos + gl.pos.to_vec2() + gl.uv_rect.offset + at;
                Glyph {
                    x: (p.x * ppp).round() as i32,
                    y: (p.y * ppp).round() as i32,
                    min: gl.uv_rect.min,
                    max: gl.uv_rect.max,
                }
            })
        })
        .collect()
}

/// The top and bottom of a laid-out text's ink (points); `None` without.
fn ink_rows(g: &egui::Galley) -> Option<[f32; 2]> {
    g.rows
        .iter()
        .flat_map(|placed| {
            placed.row.glyphs.iter().filter(|gl| !gl.uv_rect.is_nothing()).map(move |gl| {
                let top = placed.pos.y + gl.pos.y + gl.uv_rect.offset.y;
                [top, top + gl.uv_rect.size.y]
            })
        })
        .reduce(|a, b| [a[0].min(b[0]), a[1].max(b[1])])
}

/// A name laid out with its readings: its base, and each reading with where
/// it goes (points, from the base's origin).
struct RubyLayout {
    base: Arc<egui::Galley>,
    readings: Vec<(Arc<egui::Galley>, egui::Vec2)>,
}

/// `ruby` laid out at `font`, or smaller when it is wider than its `room`
/// (`ppp` pixels a point), its readings by the same factor. `None` when its
/// runs or its ink cannot be found: the name is then laid out as written.
fn lay_ruby(fonts: &mut egui::epaint::FontsView<'_>, ruby: &super::readings::Ruby, font: &egui::FontId, ppp: f32) -> Option<RubyLayout> {
    let (laid, wide) = lay_ruby_at(fonts, ruby, font.size)?;
    let room = room(&laid.base, ppp);
    if wide <= room {
        return Some(laid);
    }
    lay_ruby_at(fonts, ruby, (font.size * room / wide * 0.98).max(1.0)).map(|(laid, _)| laid)
}

/// `ruby` at the font size `size`, and the width a picture centred on its
/// base needs.
fn lay_ruby_at(fonts: &mut egui::epaint::FontsView<'_>, ruby: &super::readings::Ruby, size: f32) -> Option<(RubyLayout, f32)> {
    use super::readings::{READING_GAP, READING_SCALE, place};
    let white = egui::Color32::WHITE;
    let base = fonts.layout_no_wrap(ruby.base.clone(), egui::FontId::proportional(size), white);
    // the runs' edges where the cursor would stand: shaped text counts its
    // glyphs by chars, checked (a shaper that merged some would move them)
    let glyphs: usize = base.rows.iter().map(|r| r.row.glyphs.len()).sum();
    if glyphs != ruby.base.chars().count() {
        return None;
    }
    let edge = |i: usize| base.pos_from_cursor(egui::text::CCursor::new(egui::text::CharIndex(i))).min.x;
    let runs: Vec<[f32; 2]> = ruby.spans.iter().map(|(r, _)| [edge(r.start), edge(r.end)]).collect();
    let readings: Vec<_> = ruby
        .spans
        .iter()
        .map(|(_, text)| fonts.layout_no_wrap(text.clone(), egui::FontId::proportional(size * READING_SCALE), white))
        .collect();
    let widths: Vec<f32> = readings.iter().map(|g| g.size().x).collect();
    let (lefts, wide) = place(&runs, &widths, base.size().x);
    // every reading on one line, its ink a gap above the base's
    let top = ink_rows(&base)?[0];
    let bottom = readings.iter().filter_map(|g| ink_rows(g)).map(|r| r[1]).reduce(f32::max)?;
    let y = top - READING_GAP * size - bottom;
    let readings = readings.into_iter().zip(lefts).map(|(g, x)| (g, egui::vec2(x, y))).collect();
    Some((RubyLayout { base, readings }, wide))
}

/// A glyph: where its top left lands (pixels) and its rectangle in the
/// font atlas (texels, max exclusive).
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub x: i32,
    pub y: i32,
    pub min: [u16; 2],
    pub max: [u16; 2],
}

/// Lays the glyphs' coverage (`coverage(x, y)` of the atlas) into one
/// picture with its glow and levels.
pub fn compose(glyphs: &[Glyph], coverage: impl Fn(usize, usize) -> u8) -> Option<NameImage> {
    compose_parts(glyphs, &[], coverage)
}

/// The frame of the glyphs' ink (`[x0, y0, x1, y1]`, pixels, max
/// exclusive); `None` without ink.
fn bounds(glyphs: &[Glyph]) -> Option<[i32; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for g in glyphs {
        let (w, h) = (i32::from(g.max[0]) - i32::from(g.min[0]), i32::from(g.max[1]) - i32::from(g.min[1]));
        if w <= 0 || h <= 0 {
            continue;
        }
        x0 = x0.min(g.x);
        y0 = y0.min(g.y);
        x1 = x1.max(g.x + w);
        y1 = y1.max(g.y + h);
    }
    (x0 < x1 && y0 < y1).then_some([x0, y0, x1, y1])
}

/// `compose` with readings: `over`, the readings' glyphs, laid with the
/// base `glyphs`. The picture keeps the base centred across and says how
/// far its centre is above the base's (`NameImage::lift`); the glow's
/// radius and the room around the ink (`pad`) come from the base's height,
/// so that a name with a reading glows like the others. Without readings:
/// `compose`'s picture.
fn compose_parts(glyphs: &[Glyph], over: &[Glyph], coverage: impl Fn(usize, usize) -> u8) -> Option<NameImage> {
    let base = bounds(glyphs)?;
    let all = bounds(over).map_or(base, |o| {
        [base[0].min(o[0]), base[1].min(o[1]), base[2].max(o[2]), base[3].max(o[3])]
    });
    let ([x0, y0, x1, y1], lift) = super::readings::frame(base, all);
    let base_h = (base[3] - base[1]) as usize;
    let pad = pad(base_h);
    let (w, h) = ((x1 - x0) as usize + 2 * pad, (y1 - y0) as usize + 2 * pad);
    if w > MAX_W || h > 512 {
        return None;
    }
    let mut text = vec![0u8; w * h];
    for g in glyphs.iter().chain(over) {
        for ty in g.min[1]..g.max[1] {
            for tx in g.min[0]..g.max[0] {
                let x = (g.x - x0) as usize + pad + usize::from(tx - g.min[0]);
                let y = (g.y - y0) as usize + pad + usize::from(ty - g.min[1]);
                let c = coverage(usize::from(tx), usize::from(ty));
                if let Some(t) = text.get_mut(y * w + x) {
                    *t = (*t).max(c);
                }
            }
        }
    }
    // the base's height: a reading over it does not thicken the glow
    let glow = blur(&text, w, h, glow_radius(base_h));
    let mut rg = Vec::with_capacity(w * h * 2);
    for (t, g) in text.iter().zip(&glow) {
        rg.push(*t);
        rg.push(((u32::from(*g) * 2).min(255)) as u8);
    }
    let levels = super::atlas::levels(&rg, w as u32, h as u32, aurora_render::constellations::ATLAS_MIPS);
    Some(NameImage {
        size: [w as u32, h as u32],
        lift,
        pad: pad as u32,
        levels: levels.into_iter().map(Arc::new).collect(),
    })
}

/// Two box blurs of radius `r` each way: about a Gaussian.
fn blur(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    let pass = |src: &[u16], horizontal: bool| -> Vec<u16> {
        let mut out = vec![0u16; src.len()];
        let (n, lines) = if horizontal { (w, h) } else { (h, w) };
        let at = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
        for line in 0..lines {
            let mut sum = 0u32;
            for i in 0..n.min(r + 1) {
                sum += u32::from(src[at(line, i)]);
            }
            for i in 0..n {
                out[at(line, i)] = (sum / (2 * r as u32 + 1)) as u16;
                if i + r + 1 < n {
                    sum += u32::from(src[at(line, i + r + 1)]);
                }
                if i >= r {
                    sum -= u32::from(src[at(line, i - r)]);
                }
            }
        }
        out
    };
    let mut v: Vec<u16> = src.iter().map(|&c| u16::from(c)).collect();
    for _ in 0..2 {
        v = pass(&v, true);
        v = pass(&v, false);
    }
    v.into_iter().map(|c| c.min(255) as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_has_its_text_in_english_french_and_japanese() {
        assert!(KEYS.windows(2).all(|w| w[0].0 < w[1].0), "sorted by id");
        assert_eq!(key("Ori"), Some("constellation-ori"));
        assert_eq!(key("Orion"), None);
        for (id, k) in KEYS {
            for lang in ["en", "fr", "ja"] {
                let text = aurora_i18n::text_in(lang, k);
                assert_ne!(text, *k, "{id} has no {lang} name");
                assert!(!text.trim().is_empty());
            }
        }
        assert_eq!(aurora_i18n::text_in("en", "constellation-uma"), "Ursa Major");
        assert_eq!(aurora_i18n::text_in("fr", "constellation-uma"), "Grande Ourse");
        assert_eq!(aurora_i18n::text_in("ja", "constellation-uma"), "おおぐま座");
    }

    #[test]
    fn glyphs_are_laid_with_room_for_the_glow_and_levels() {
        // two 4 x 8 glyphs, one 6 px right of the other, a pixel lower
        let glyphs = [
            Glyph {
                x: 0,
                y: 0,
                min: [0, 0],
                max: [4, 8],
            },
            Glyph {
                x: 6,
                y: 1,
                min: [4, 0],
                max: [8, 8],
            },
        ];
        let img = compose(&glyphs, |x, _| if x < 4 { 255 } else { 128 }).expect("a picture");
        let (w, h) = (img.size[0] as usize, img.size[1] as usize);
        // 9 rows of ink: a glow of radius 2, reaching 4, and the levels' 4
        let pad = pad(9);
        assert_eq!((pad, img.pad), (8, 8));
        assert_eq!((w, h), (10 + 2 * pad, 9 + 2 * pad));
        let full = &img.levels[0];
        assert_eq!(full.len(), w * h * 2);
        let at = |x: usize, y: usize| full[(y * w + x) * 2];
        assert_eq!(at(pad, pad), 255);
        assert_eq!(at(pad + 6, pad + 1), 128);
        assert_eq!(at(pad + 5, pad), 0, "the gap between them");
        assert_eq!(at(0, 0), 0, "the pad");
        // the glow reaches into the pad, and fades
        let glow = |x: usize, y: usize| full[(y * w + x) * 2 + 1];
        assert!(glow(pad - 1, pad + 4) > 0);
        assert!(glow(pad + 2, pad + 4) > glow(pad - 2, pad + 4));
        assert!(edges_are_zero(&img));
        // its ink and spacing, and the top of them over the base's centre
        assert_eq!(img.spaced(), [10 + 2 * SPACING, 9 + 2 * SPACING]);
        assert_eq!(img.top(), 4.5 + SPACING as f32);
        assert_eq!(img.bottom(), 4.5 + SPACING as f32);
        assert_eq!(img.levels.len(), 3);
        assert_eq!(img.levels[1].len(), w.div_ceil(2) * h.div_ceil(2) * 2);
        // nothing to draw
        assert!(compose(&[], |_, _| 0).is_none());
        assert!(
            compose(
                &[Glyph {
                    x: 0,
                    y: 0,
                    min: [3, 3],
                    max: [3, 9]
                }],
                |_, _| 255
            )
            .is_none()
        );
    }

    /// Every level's outermost rows and columns are zero, text and glow: no
    /// edge of the picture can show, whatever the brightness or the level
    /// read.
    fn edges_are_zero(img: &NameImage) -> bool {
        let (mut w, mut h) = (img.size[0] as usize, img.size[1] as usize);
        img.levels.iter().all(|l| {
            let zero = |x: usize, y: usize| l.get((y * w + x) * 2..(y * w + x) * 2 + 2) == Some(&[0, 0][..]);
            let edges = (0..w).all(|x| zero(x, 0) && zero(x, h - 1)) && (0..h).all(|y| zero(0, y) && zero(w - 1, y));
            (w, h) = (w.div_ceil(2), h.div_ceil(2));
            edges
        })
    }

    #[test]
    fn the_glow_fades_to_zero_inside_every_level_at_every_size() {
        let ctx = fonts_ready();
        let long = "Celestial Dog ".repeat(30);
        let long_read = format!("{}座(ざ)", "望".repeat(80));
        let texts = [
            "Orion",
            "Ursa Major",
            "北極(ほっきょく)",
            "輔(附北斗)(ほ(ふほくと))",
            long.as_str(),
            long_read.as_str(),
        ];
        let sizes = [*super::super::FONT_RANGE.start(), 16.0, *super::super::FONT_RANGE.end()];
        for font in sizes {
            let em = super::super::name_em_texels(font);
            for (t, img) in texts.iter().zip(rasterize(&ctx, &texts, em)) {
                let img = img.unwrap_or_else(|| panic!("{t} at {font}: a picture"));
                assert!(edges_are_zero(&img), "{t} at {font}: {:?}", img.size);
                assert!(img.size[0] as usize <= MAX_W && img.size[1] <= 512, "{t} at {font}: {:?}", img.size);
                assert!(img.pad >= SPACING, "{t} at {font}: the spacing within the room");
            }
        }
        // the cause: at the default size the glow reaches farther than the
        // fixed 6 texels the pictures had, which cut it into a rectangle
        let em = super::super::name_em_texels(16.0);
        let Some(img) = rasterize(&ctx, &["Ursa Major"], em).pop().flatten() else {
            panic!("a picture")
        };
        let (w, pad) = (img.size[0] as usize, img.pad as usize);
        let glow_at = |y: usize| (0..w).map(|x| img.levels[0][(y * w + x) * 2 + 1]).max().unwrap_or(0);
        assert!(glow_at(pad - 7) > 0, "7 texels over the ink");
        assert_eq!(glow_at(0), 0);
    }

    /// A context whose fonts exist: egui's own, the same on every system
    /// (a kanji or kana they lack draws as a box, with ink).
    fn fonts_ready() -> egui::Context {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        ctx
    }

    /// `rasterize` as it was before readings, for the names without one.
    fn rasterize_before(ctx: &egui::Context, texts: &[&str], em: f32) -> Vec<Option<NameImage>> {
        let ppp = ctx.pixels_per_point().max(0.25);
        let font = egui::FontId::proportional((em / ppp).clamp(4.0, 200.0));
        ctx.fonts_mut(|fonts| {
            let galleys: Vec<_> = texts
                .iter()
                .map(|t| {
                    let g = fonts.layout_no_wrap((*t).to_owned(), font.clone(), egui::Color32::WHITE);
                    let room = room(&g, ppp);
                    if g.size().x <= room {
                        return g;
                    }
                    let smaller = egui::FontId::proportional((font.size * room / g.size().x * 0.98).max(1.0));
                    fonts.layout_no_wrap((*t).to_owned(), smaller, egui::Color32::WHITE)
                })
                .collect();
            let atlas = fonts.image();
            galleys
                .iter()
                .map(|g| {
                    let glyphs: Vec<Glyph> = g
                        .rows
                        .iter()
                        .flat_map(|placed| {
                            placed.row.glyphs.iter().filter(|gl| !gl.uv_rect.is_nothing()).map(move |gl| {
                                let at = placed.pos + gl.pos.to_vec2() + gl.uv_rect.offset;
                                Glyph {
                                    x: (at.x * ppp).round() as i32,
                                    y: (at.y * ppp).round() as i32,
                                    min: gl.uv_rect.min,
                                    max: gl.uv_rect.max,
                                }
                            })
                        })
                        .collect();
                    compose(&glyphs, |x, y| atlas.pixels.get(y * atlas.size[0] + x).map_or(0, |c| c.a()))
                })
                .collect()
        })
    }

    fn same(a: &Option<NameImage>, b: &Option<NameImage>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => a.size == b.size && a.lift == b.lift && a.levels == b.levels,
            (None, None) => true,
            _ => false,
        }
    }

    #[test]
    fn a_name_without_a_reading_is_the_same_picture_as_before() {
        let ctx = fonts_ready();
        let long = "Celestial Dog ".repeat(30);
        let texts = [
            "Orion",
            "杵(箕宿)",
            "太微垣(たいびえん：太微垣)",
            "Celestial Dog (In Dipper Mansion)",
            "北極(ほっきょく",
            long.as_str(),
            "",
        ];
        for em in [60.0, 135.0] {
            let now = rasterize(&ctx, &texts, em);
            let before = rasterize_before(&ctx, &texts, em);
            for ((t, a), b) in texts.iter().zip(&now).zip(&before) {
                assert!(same(a, b), "{t} at {em}");
                assert!(a.as_ref().is_none_or(|a| a.lift == 0 && a.reach() == a.size));
            }
        }
        // the long one was laid out smaller to fit
        assert!(
            rasterize(&ctx, &[long.as_str()], 60.0)[0]
                .as_ref()
                .is_some_and(|i| i.size[0] as usize <= MAX_W)
        );
    }

    #[test]
    fn a_reading_sits_over_its_kanji_and_the_base_stays_as_it_was() {
        let ctx = fonts_ready();
        let em = 60.0;
        let pics = rasterize(&ctx, &["北極(ほっきょく)", "北極", "狼の顎(あご)", "狼の顎"], em);
        let [Some(read), Some(plain), Some(jaw), Some(jaw_plain)] = pics.as_slice() else {
            panic!("pictures: {pics:?}");
        };
        // the reading's rows over the base: about half the name's size and
        // the gap, all lifted
        assert_eq!(plain.lift, 0);
        assert!(read.lift as f32 > 0.2 * em && (read.lift as f32) < 0.8 * em, "{}", read.lift);
        assert_eq!(read.size[1], plain.size[1] + read.lift, "the reading's rows above only");
        assert_eq!(read.reach(), [read.size[0], read.size[1] + read.lift]);
        // the base's text, untouched, centred across (a reading wider than
        // its run widens the picture on both sides)
        let dx = (read.size[0] - plain.size[0]) / 2;
        assert_eq!((read.size[0] - plain.size[0]) % 2, 0);
        let text = |img: &NameImage, x: u32, y: u32| img.levels[0][((y * img.size[0] + x) * 2) as usize];
        let (pw, ph) = (plain.size[0], plain.size[1]);
        // (from the base's ink down: the reading's may reach into the pad)
        assert_eq!(read.pad, plain.pad, "the room is the base's");
        for y in plain.pad..ph {
            for x in 0..pw {
                assert_eq!(text(read, x + dx, y + read.lift), text(plain, x, y), "({x}, {y})");
            }
        }
        // the reading's ink is above the base's, within the lifted rows
        let inked = |img: &NameImage, rows: std::ops::Range<u32>| rows.into_iter().any(|y| (0..img.size[0]).any(|x| text(img, x, y) > 0));
        assert!(inked(read, 0..read.lift));
        // over the last kanji only: the reading's ink is right of the
        // picture's centre, the picture as wide as before or wider
        assert!(jaw.size[0] >= jaw_plain.size[0]);
        let ink_x: Vec<u32> = (0..jaw.size[0]).filter(|&x| (0..jaw.lift).any(|y| text(jaw, x, y) > 0)).collect();
        let mid = ink_x.iter().sum::<u32>() as f32 / ink_x.len().max(1) as f32;
        assert!(mid > jaw.size[0] as f32 * 0.6, "{mid} of {}", jaw.size[0]);
        // the glow's radius is the base's: the glow under the base (the
        // row under its ink) is the same as the plain name's
        let glow = |img: &NameImage, x: u32, y: u32| img.levels[0][((y * img.size[0] + x) * 2 + 1) as usize];
        let under = ph - plain.pad;
        assert!((0..pw).any(|x| glow(plain, x, under) > 0));
        for x in 0..pw {
            assert_eq!(glow(read, x + dx, under + read.lift), glow(plain, x, under), "x {x}");
        }
        assert!(edges_are_zero(read) && edges_are_zero(plain));
        // in the figures' size word
        assert_eq!(read.dims() & 0xfff, read.size[0]);
        assert_eq!((read.dims() >> 12) & 0x3ff, read.size[1]);
        assert_eq!(read.dims() >> 22, read.lift);
        assert_eq!(plain.dims(), plain.size[0] | plain.size[1] << 12);
    }

    #[test]
    fn a_long_name_shrinks_with_its_reading() {
        let ctx = fonts_ready();
        let em = 60.0;
        let long = format!("{}座(ざ)", "望".repeat(80));
        let pics = rasterize(&ctx, &[long.as_str(), "望座(ざ)"], em);
        let [Some(small), Some(full)] = pics.as_slice() else {
            panic!("pictures: {pics:?}");
        };
        assert!(small.size[0] as usize <= MAX_W, "fits");
        assert!(small.lift > 0 && small.lift < full.lift, "{} {}", small.lift, full.lift);
        // the same proportions as at full size (a pixel's rounding apart)
        let ratio = |i: &NameImage| i.lift as f32 / (i.size[1] - i.lift - 2 * i.pad) as f32;
        assert!((ratio(small) - ratio(full)).abs() < 0.15, "{} {}", ratio(small), ratio(full));
    }

    #[test]
    fn a_picture_with_a_reading_fits_at_the_largest_name_size() {
        let ctx = fonts_ready();
        let em = super::super::name_em_texels(*super::super::FONT_RANGE.end());
        let pics = rasterize(&ctx, &["輔(附北斗)(ほ(ふほくと))", "三台(上台)(さんだい(じょうだい))"], em);
        for p in &pics {
            let p = p.as_ref().expect("a picture under the 512 rows cap");
            assert!(p.size[1] <= 512 && p.lift > 0, "{:?} {}", p.size, p.lift);
        }
    }
}
