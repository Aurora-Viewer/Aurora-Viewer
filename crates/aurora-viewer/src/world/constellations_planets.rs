//! The planets' names over the real night sky (world/planets.rs draws the
//! planets): a name under each planet drawn, when the Constellations
//! window's "Planet names" is on, in the constellations' atlas and their names' look and
//! size (constellations.wgsl `constellation_planet_names`, a pass of its own:
//! the figures may be off, or the folder hold none).
//!
//! The text is data: `planets.json` of the constellation data, per
//! planet id (`mercury`…`uranus`) the form of a culture's name
//! (`{"name": …, "names": {"fr": …, "ja": …}}`). A missing file or planet,
//! or an entry left empty (`{"name": "", "names": {}}`), is its built-in
//! name (the catalogue's `planet-*`, in the interface's language); a
//! language the entry does not have, the entry's own `name` (the form's
//! default name). The file is read with the
//! rest of the folder, on its thread, and again on Rescan when it changed.
//!
//! The names show in the dark only, by the constellations' darkness
//! setting (the region's sun, whatever the planets' own switches), fade in
//! and out with the constellations' fade time, and follow their planet's
//! own fade against the Sun's glare. Off: a few comparisons a frame,
//! nothing rasterized, placed nor sent.

use super::{ConstellationSettings, Constellations, Input, NAME_REACH, NAME_TEXEL_DEGREES, catalogue, library, name_em_texels, names};
use crate::world::planets::{builtin_name, ephemeris::Body};
use aurora_render::constellations::{FrameState, HEAD_PLANET_NAMES, Item, Overlay, PLANET_NAMES, Upload, pack_place};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Between two names stacked under planets in conjunction (radians,
/// 0.1°), and the lowest a name goes (the shader's reach, constellations.wgsl).
const STACK_GAP: f32 = 0.0017;
const MAX_DROP: f32 = 0.09;

/// The data file, at the top of the constellation data folder.
pub const FILE: &str = "planets.json";

/// Per planet (`Body::ALL`'s order), its names from the file (`None`: the
/// built-in one).
pub type Texts = [Option<catalogue::Names>; PLANET_NAMES];

/// Reads the data's `planets.json` (the user's when their folder has one,
/// else the built-in one). Missing: every name built in, nothing said;
/// unreadable, or an entry that does not read: said in `problems` (logged
/// once), those names built in.
pub fn read(data: &super::builtin::Data, problems: &mut Vec<String>) -> Texts {
    match data.read(FILE, catalogue::MAX_SET_FILE) {
        None => Texts::default(),
        Some(Ok(bytes)) => parse(&bytes, problems),
        Some(Err(e)) => {
            problems.push(format!("{FILE}: {e}"));
            Texts::default()
        }
    }
}

/// The file's text: each planet's entry read as a culture's name
/// (catalogue.rs `parse_culture`: the same cleaning and limits).
pub fn parse(bytes: &[u8], problems: &mut Vec<String>) -> Texts {
    let mut out = Texts::default();
    let map: BTreeMap<String, serde_json::Value> = match serde_json::from_slice(bytes) {
        Ok(m) => m,
        Err(e) => {
            problems.push(format!("{FILE}: {e}"));
            return out;
        }
    };
    for (k, body) in Body::ALL.iter().enumerate() {
        let Some(entry) = map.get(body.id()) else { continue };
        match serde_json::to_vec(entry)
            .map_err(|e| e.to_string())
            .and_then(|b| catalogue::parse_culture(&b))
        {
            Ok(n) => out[k] = n,
            Err(e) => problems.push(format!("{FILE}: {}: {e}", body.id())),
        }
    }
    out
}

/// Planet `k`'s name in `language` (the interface's code).
pub fn text(texts: &Texts, k: usize, language: &str) -> String {
    match texts.get(k).and_then(Option::as_ref) {
        Some(n) => n.display(language).to_owned(),
        None => Body::ALL
            .get(k)
            .map_or("", |b| aurora_i18n::text_in(language, builtin_name(*b)))
            .to_owned(),
    }
}

/// The names' state on the main thread (`Constellations::planet_names`).
#[derive(Default)]
pub struct PlanetNames {
    /// The folder's names, from its last reading.
    texts: Arc<Texts>,
    /// Their pictures, for one font size, scale and language.
    images: Option<((u32, u32, &'static str), Vec<Option<names::NameImage>>)>,
    /// Per planet: its name's fade, its last direction (catalogue frame)
    /// and its planet's own opacity.
    alpha: [f32; PLANET_NAMES],
    at: [[f32; 4]; PLANET_NAMES],
}

impl PlanetNames {
    /// A new reading of the folder (`Constellations::adopt`, which clears
    /// the atlas): the pictures are made again.
    pub fn adopt(&mut self, texts: Option<Arc<Texts>>) {
        self.texts = texts.unwrap_or_default();
        self.images = None;
    }

    fn image(&self, k: usize) -> Option<&names::NameImage> {
        self.images.as_ref().and_then(|(_, i)| i.get(k)).and_then(Option::as_ref)
    }
}

impl Constellations {
    /// One frame of the planets' names: their pictures, places and fades.
    /// Returns whether one shows or waits to, and this frame's uploads.
    pub(super) fn planet_names_step(
        &mut self,
        input: &Input,
        s: &ConstellationSettings,
        dark: bool,
        ctx: &egui::Context,
    ) -> (bool, Vec<Upload>) {
        let wanted = input.planets.on && input.real_sky && dark;
        let pn = &mut self.planet_names;
        if !wanted && pn.alpha.iter().all(|a| *a <= 0.0) {
            return (false, Vec::new());
        }
        if self.generation == 0 {
            // the folder's first reading (its own thread): planets.json
            // comes with it
            library::rescan();
            return (wanted, Vec::new());
        }
        // the pictures, made again when their size, the scale or the
        // language changes (the fonts exist once the interface ran a pass)
        let key = (s.font_size.to_bits(), ctx.pixels_per_point().to_bits(), aurora_i18n::language());
        if wanted && ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT) > 0 && pn.images.as_ref().is_none_or(|(k, _)| *k != key) {
            let texts: Vec<String> = (0..PLANET_NAMES).map(|k| text(&pn.texts, k, key.2)).collect();
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            pn.images = Some((key, names::rasterize(ctx, &refs, name_em_texels(s.font_size))));
            for k in 0..PLANET_NAMES {
                self.atlas.release(Item::PlanetName(k as u32));
            }
        }
        let mut uploads = Vec::new();
        let mut busy = false;
        let fade = input.dt.max(0.0) / s.fade_seconds.max(0.1);
        for k in 0..PLANET_NAMES {
            let point = input.planets.points[k];
            if point[..3].iter().any(|v| *v != 0.0) {
                pn.at[k] = point;
            }
            let want = wanted && point[3] > 0.0;
            let item = Item::PlanetName(k as u32);
            let mut placed = self.atlas.place_of(item).is_some();
            if want
                && !placed
                && let Some(img) = pn.image(k)
                && let Some(p) = self.atlas.place(item, img.size, |_| true)
            {
                uploads.push(Upload {
                    item,
                    layer: p.layer,
                    block: p.block,
                    size: img.size,
                    rows: [0, img.size[1]],
                    levels: img.levels.clone(),
                });
                placed = true;
            }
            let a = &mut pn.alpha[k];
            *a = if want && placed {
                (*a + fade).min(1.0)
            } else {
                (*a - fade).max(0.0)
            };
            if *a > 0.0 {
                self.atlas.touch(item);
            }
            busy |= want || *a > 0.0;
        }
        (busy, uploads)
    }

    /// The planets' names into this frame's uniform: their words and the
    /// head's bit (none shown: nothing written, the shader skips them). A
    /// name that would cover an earlier planet's (two planets in
    /// conjunction) goes under it.
    pub(super) fn planet_names_fill(&self, st: &mut FrameState) {
        let pn = &self.planet_names;
        // per name shown: its planet, direction, half width and height
        // (radians, its ink and spacing: `names::SPACING`, whatever room
        // its picture has around them)
        let mut shown: Vec<(usize, glam::Vec3, f32, f32)> = Vec::new();
        for k in 0..PLANET_NAMES {
            let (Some(p), Some(img)) = (self.atlas.place_of(Item::PlanetName(k as u32)), pn.image(k)) else {
                continue;
            };
            let opacity = pn.alpha[k] * pn.at[k][3];
            if opacity <= 0.0 {
                continue;
            }
            // the names' texel, kept within the shader's reach of its planet
            let half = (img.size[0] as f32).hypot(img.size[1] as f32) * 0.5;
            let texel = NAME_TEXEL_DEGREES.to_radians().min(0.85 * NAME_REACH / half.max(1.0));
            let [x, y, z, _] = pn.at[k];
            let dir = glam::Vec3::new(x, y, z);
            let [wide, tall] = img.spaced();
            let drop: f32 = shown
                .iter()
                .filter(|(_, d, w, _)| d.angle_between(dir) < w + 0.5 * wide as f32 * texel)
                .map(|(_, _, _, h)| h + STACK_GAP)
                .sum();
            shown.push((k, dir, 0.5 * wide as f32 * texel, tall as f32 * texel));
            st.planets[2 * k] = [x, y, z, opacity];
            // its centre under the planet: the top of its ink and spacing
            // a gap under it, past the names stacked before it
            st.planets[2 * k + 1] = [
                f32::from_bits(pack_place(p.layer, p.block[0], p.block[1])),
                f32::from_bits(img.size[0] | img.size[1] << 16),
                texel,
                names::HANG_GAP + img.top() * texel + drop.min(MAX_DROP),
            ];
            st.head[0] |= HEAD_PLANET_NAMES;
        }
    }

    /// The figures show nothing this frame: the planets' names alone when
    /// one shows or waits, else the overlay's idle.
    pub(super) fn idle_or_planet_names(&mut self, dt: f32, busy: bool, uploads: Vec<Upload>) {
        if !busy {
            self.idle(dt);
            return;
        }
        self.idle = 0.0;
        self.shown = 0;
        let mut st = FrameState::default();
        self.planet_names_fill(&mut st);
        let mut o = Overlay {
            state: Box::new(st),
            layers: self.atlas.layers(),
            uploads,
            ..Overlay::default()
        };
        if let Some(v) = &self.view {
            // the tables of the cultures drawn
            o.tables = Some(v.tables.clone());
            o.tables_id = self.tables_id;
        }
        self.frame = Some(Arc::new(o));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAGED: &str = r#"{
      "mercury": { "name": "Mercury", "names": { "fr": "Mercure", "ja": "水星" } },
      "venus": { "name": "Venus", "names": { "fr": "Vénus", "ja": "金星" } },
      "jupiter": { "name": "Jupiter", "names": { "fr": "Jupiter", "ja": "木星" } },
      "saturn": { "names": { "fr": "Saturne" } },
      "pluto": { "name": "Pluto" }
    }"#;

    #[test]
    fn the_planets_names_come_from_the_file_else_built_in() {
        let mut problems = Vec::new();
        let t = parse(STAGED.as_bytes(), &mut problems);
        assert_eq!(text(&t, 0, "fr"), "Mercure");
        assert_eq!(text(&t, 1, "ja"), "金星");
        assert_eq!(text(&t, 1, "fr-CA"), "Vénus", "a region falls back to its language");
        assert_eq!(text(&t, 3, "de"), "Jupiter", "a missing language: the entry's own name");
        assert_eq!(text(&t, 2, "fr"), "Mars", "a missing planet: built in");
        assert_eq!(text(&t, 5, "ja"), "天王星", "built in, in the interface's language");
        assert_eq!(text(&t, 5, "en"), "Uranus");
        // an entry without its name is said and built in; an unknown id is
        // no planet of ours and nothing to say
        assert_eq!(text(&t, 4, "fr"), "Saturne");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("saturn"), "{problems:?}");
    }

    #[test]
    fn a_missing_or_broken_file_leaves_every_name_built_in() {
        let mut problems = Vec::new();
        let dir = std::env::temp_dir().join("aurora-planet-names-none");
        let t = read(&super::super::builtin::Data::folder(&dir), &mut problems);
        assert!(problems.is_empty());
        let english: Vec<String> = (0..PLANET_NAMES).map(|k| text(&t, k, "en")).collect();
        assert_eq!(english, ["Mercury", "Venus", "Mars", "Jupiter", "Saturn", "Uranus"]);
        let french: Vec<String> = (0..PLANET_NAMES).map(|k| text(&t, k, "fr")).collect();
        assert_eq!(french, ["Mercure", "Vénus", "Mars", "Jupiter", "Saturne", "Uranus"]);
        let t = parse(b"[1, 2]", &mut problems);
        assert_eq!(problems.len(), 1);
        assert_eq!(text(&t, 0, "ja"), "水星");
        assert_eq!(text(&t, PLANET_NAMES, "en"), "", "no seventh planet");
    }

    #[test]
    fn a_shown_name_fills_its_words_and_the_head_bit_only() {
        let mut c = Constellations::default();
        let mut st = FrameState::default();
        c.planet_names_fill(&mut st);
        assert_eq!(st, FrameState::default(), "nothing shown: nothing written");
        let img = names::NameImage {
            size: [120, 40],
            levels: Vec::new(),
            lift: 0,
            // the spacing as its room: the picture's top is its spacing's
            pad: names::SPACING,
        };
        let mut images = vec![None; PLANET_NAMES];
        images[3] = Some(img);
        c.planet_names.images = Some(((0, 0, "en"), images));
        c.planet_names.at[3] = [0.0, 0.6, 0.8, 0.5];
        c.planet_names.alpha[3] = 1.0;
        c.planet_names_fill(&mut st);
        assert_eq!(st, FrameState::default(), "not in the atlas yet: nothing written");
        let p = c.atlas.place(Item::PlanetName(3), [120, 40], |_| true).expect("room");
        c.planet_names_fill(&mut st);
        assert_eq!(st.head, [HEAD_PLANET_NAMES, 0, 0, 0], "the figures' bit stays theirs");
        assert_eq!(st.planets[6], [0.0, 0.6, 0.8, 0.5], "direction, opacity with its planet's");
        assert_eq!(st.planets[7][0].to_bits(), pack_place(p.layer, p.block[0], p.block[1]));
        assert_eq!(st.planets[7][1].to_bits(), 120 | 40 << 16);
        assert!((st.planets[7][2] - NAME_TEXEL_DEGREES.to_radians()).abs() < 1e-9);
        // alone: right under its planet, its picture's top a gap under it
        let t = NAME_TEXEL_DEGREES.to_radians();
        let under = names::HANG_GAP + 20.0 * t;
        assert!((st.planets[7][3] - under).abs() < 1e-7, "{}", st.planets[7][3]);
        assert!(st.planets.iter().enumerate().all(|(i, w)| i == 6 || i == 7 || *w == [0.0; 4]));
        // Saturn in conjunction with Jupiter: its name under Jupiter's
        let img = c.planet_names.image(3).cloned();
        if let Some((_, images)) = c.planet_names.images.as_mut() {
            images[4] = img;
        }
        c.planet_names.at[4] = [0.0, 0.601, 0.799, 1.0];
        c.planet_names.alpha[4] = 1.0;
        c.atlas.place(Item::PlanetName(4), [120, 40], |_| true).expect("room");
        c.planet_names_fill(&mut st);
        let height = 40.0 * t;
        assert!((st.planets[7][3] - under).abs() < 1e-7, "the first stays");
        assert!(
            (st.planets[9][3] - (under + height + STACK_GAP)).abs() < 1e-6,
            "{}",
            st.planets[9][3]
        );
        // far apart: each under its own
        c.planet_names.at[4] = [0.0, 0.8, 0.6, 1.0];
        c.planet_names_fill(&mut st);
        assert!((st.planets[9][3] - under).abs() < 1e-7);
        // the same ink in a picture with more room around it (its glow's):
        // the text stays where it was, its centre as far under the planet
        if let Some((_, images)) = c.planet_names.images.as_mut() {
            images[3] = Some(names::NameImage {
                size: [140, 60],
                levels: Vec::new(),
                lift: 0,
                pad: names::SPACING + 10,
            });
        }
        c.planet_names_fill(&mut st);
        assert!((st.planets[7][3] - under).abs() < 1e-7, "{}", st.planets[7][3]);
    }

    #[test]
    fn off_the_names_cost_nothing() {
        let mut c = Constellations::default();
        let settings = ConstellationSettings::default();
        let stars = aurora_render::stars::StarFrame::default();
        let labels = crate::world::planets::Labels::default();
        let input = Input {
            settings: &settings,
            real_sky: true,
            stars: &stars,
            sun: glam::Vec3::NEG_Z,
            look: glam::Vec3::Y,
            fov_y: 1.0,
            window_height: 1000,
            dt: 0.1,
            planets: &labels,
        };
        let ctx = egui::Context::default();
        let (busy, uploads) = c.planet_names_step(&input, &settings, true, &ctx);
        assert!(!busy && uploads.is_empty());
        assert!(c.planet_names.images.is_none(), "nothing rasterized");
        assert_eq!(c.atlas.layers(), 0, "nothing placed");
    }
}
