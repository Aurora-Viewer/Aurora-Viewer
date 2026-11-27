//! The constellation data the user installs in their profile
//! (`constellations/` in the configuration folder, beside `photo-luts`;
//! `AURORA_CONSTELLATION_DIR` for a launch), read at run time. Nothing of
//! it is built into the viewer, and nothing is read from the network.
//!
//! ```text
//! constellations/cultures/<culture>/lines.json   a culture's stick lines, names, label points
//! constellations/cultures/<culture>/culture.json its name (and translations), optional
//! constellations/cultures/<culture>/names.json   its figures' names by language, optional
//! constellations/lines.json                      the western culture's, when it has no folder
//! constellations/names.json                      its figures' names, likewise (before the catalogues')
//! constellations/sets/<set>/set.json             an art set: name, translations, order, culture
//! constellations/sets/<set>/<figure>.json        a figure's metadata in that set
//! constellations/sets/<set>/<figure>.webp        its drawing (lossless, LA: ink, reveal time)
//! ```
//!
//! [`scan`] reads the small files only (the JSON); a drawing's WebP is read
//! when the figure first shows (constellations.rs). The sets are the
//! folders that hold a `set.json`, each of one culture (`"culture"`,
//! `western` by default); a figure is known by its culture and its id (ids
//! are unique within a culture only). Which cultures, sets and figures
//! there are, and which figures a set draws, is the data's business: no
//! list of them here.
//!
//! The files are the user's, so they are read as untrusted input: each has
//! a size limit, ids keep to a safe set of characters (a file's name comes
//! from its id, never from a path inside a JSON), counts are bounded, and a
//! file that does not read is left out with a problem the caller logs
//! once. Nothing here panics on bad data.

use aurora_render::constellations::{ATLAS_H, ATLAS_W, FigureGeom, Line, Picture};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Limits on what a folder may hold (well above the art's own: its
/// largest JSON is 60 KB, its largest WebP 200 KB, its widest drawing 1600
/// texels).
pub const MAX_LINES_FILE: u64 = 4 << 20;
pub const MAX_SET_FILE: u64 = 64 << 10;
pub const MAX_FIGURE_FILE: u64 = 1 << 20;
pub const MAX_WEBP: u64 = 32 << 20;
pub const MAX_SETS: usize = 32;
/// Figure files read from one set's folder.
pub const MAX_SET_FIGURES: usize = 512;
pub const MAX_LINES: usize = 1024;
pub const MAX_NAME_CHARS: usize = 64;
/// Figures read from one `lines.json` (the largest culture of the data has
/// 372), and from the whole folder: every culture is read, whatever is
/// ticked; the GPU holds the ticked ones (aurora-render `MAX_FIGURES` in
/// all, the window says what goes past it).
pub const MAX_FILE_FIGURES: usize = 2048;
pub const MAX_CATALOGUE_FIGURES: usize = 16384;
/// A culture's `names.json`, and the languages it may give a figure.
pub const MAX_NAMES_FILE: u64 = 4 << 20;
const MAX_LANGUAGES: usize = 64;
const MAX_ID: usize = 40;

/// Which group a figure belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One of the 88 constellations.
    Modern,
    /// One of the 88 the art marks as without a drawing. Whether a figure
    /// has a drawing is read from its drawings, never from its kind.
    LinesOnly,
    /// A figure no longer counted: behind its own switch.
    Obsolete,
}

impl Kind {
    fn parse(s: &str) -> Option<Kind> {
        match s {
            "modern" => Some(Kind::Modern),
            "lines-only" => Some(Kind::LinesOnly),
            "obsolete" => Some(Kind::Obsolete),
            _ => None,
        }
    }
}

/// The culture of the sets that do not name one.
pub const WESTERN: &str = "western";

/// A name and its translations by language code (`fr`, `ja`, `zh-Hans`…).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    pub name: String,
    pub names: BTreeMap<String, String>,
}

impl Names {
    /// The name in `language` (the interface's code): that language's, the
    /// language's own without its region (`pt` for `pt-BR`), else the
    /// default name.
    pub fn display(&self, language: &str) -> &str {
        let base = language.split('-').next().unwrap_or(language);
        self.names
            .get(language)
            .or_else(|| self.names.get(base))
            .map_or(self.name.as_str(), String::as_str)
    }
}

/// A culture's own sky: its stick lines and names, and the sets drawing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CultureInfo {
    pub id: String,
    /// From its `culture.json`; the western culture's name is the
    /// catalogues' (`constellation-culture-western`).
    pub names: Option<Names>,
}

impl CultureInfo {
    pub fn display_name(&self, language: &str) -> String {
        match &self.names {
            _ if self.id == WESTERN => aurora_i18n::text("constellation-culture-western").to_owned(),
            Some(n) => n.display(language).to_owned(),
            None => self.id.clone(),
        }
    }
}

/// An art set (`sets/<id>/set.json`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetInfo {
    /// Its folder's name: what the setting stores.
    pub id: String,
    pub names: Names,
    pub order: i64,
    /// Its culture's index in `Catalogue::cultures`.
    pub culture: usize,
    pub culture_id: String,
}

impl SetInfo {
    pub fn display_name(&self, language: &str) -> &str {
        self.names.display(language)
    }
}

/// One drawing of a figure.
#[derive(Clone, Debug)]
pub struct Drawing {
    /// Its set's index in `Catalogue::sets`.
    pub set: usize,
    /// The centre of its stereographic plane (radians).
    pub centre: [f32; 2],
    pub picture: Picture,
    /// The set's own label point for the name, when it moves it: only a
    /// point the set placed the name's box at (`label_set_box_deg`).
    pub label: Option<[f32; 2]>,
    /// Its file placed the figure's own label point clear of the art
    /// (`label_box_deg`, the name's box there): the name stays centred on it
    /// while the drawing shows (`Figure::label_clear_of_drawing`).
    pub label_placed: bool,
    /// Its WebP file (`sets/<set>/<figure id>.webp`), read when it shows.
    pub image: PathBuf,
}

/// One figure.
#[derive(Clone, Debug)]
pub struct Figure {
    /// Its culture's index in `Catalogue::cultures`; its id within it.
    pub culture: usize,
    pub id: String,
    pub kind: Kind,
    /// Its name's catalogue key (`constellation-<id>`, the 88), else
    /// `name`, the data's.
    pub name_key: Option<&'static str>,
    pub name: String,
    pub geom: FigureGeom,
    /// Its drawings, in the sets' order.
    pub drawings: Vec<Drawing>,
    /// Its names by language code, from its culture's `names.json`.
    pub names: BTreeMap<String, String>,
}

impl Figure {
    /// Its name in `language` (the interface's code): the catalogues' for
    /// the 88; else its culture's `names.json` in that language (or the
    /// language without its region, `zh` for `zh-Hans`), then in English,
    /// then the name its lines give, then its id.
    /// A western figure's `names.json` in the language (or its base) comes
    /// before its catalogue key, never its English: the catalogues keep the
    /// languages the file lacks.
    pub fn display_name(&self, language: &str) -> &str {
        let base = language.split('-').next().unwrap_or(language);
        let own = self.names.get(language).or_else(|| self.names.get(base));
        if let Some(key) = self.name_key {
            return own.map_or_else(|| aurora_i18n::text(key), String::as_str);
        }
        own.or_else(|| self.names.get("en")).map_or(self.name.as_str(), String::as_str)
    }

    /// The drawing that shows (`chosen`) placed its figure's label point
    /// clear of its art (`Drawing::label_placed`): the engravings' sets do.
    /// A set that does not (no `label_box_deg`) leaves the name to the stars'
    /// rule (`name_hang`) while its drawing shows.
    pub fn label_clear_of_drawing(&self) -> bool {
        chosen(&self.drawings)
            .and_then(|k| self.drawings.get(k))
            .is_some_and(|d| d.label_placed)
    }
}

/// The cultures, the sets (in their order) and the figures (by culture,
/// then id: their index in the shader's tables).
#[derive(Debug, Default)]
pub struct Catalogue {
    pub cultures: Vec<CultureInfo>,
    pub sets: Vec<SetInfo>,
    pub figures: Vec<Figure>,
}

impl Catalogue {
    /// Nothing to draw (no folder, or nothing usable in it).
    pub fn is_empty(&self) -> bool {
        self.figures.is_empty()
    }
}

/// Which of a figure's drawings shows (its index in `drawings`, which are
/// in the sets' order): the first, every set of its culture being used, in
/// their `order` (one western set of four folders, as any culture's); none
/// without a drawing (lines and name only).
pub fn chosen(drawings: &[Drawing]) -> Option<usize> {
    (!drawings.is_empty()).then_some(0)
}

/// The culture shown (its index in `ids`, the cultures' ids in their
/// order): one at a time. The `cultures` setting keeps the list older
/// builds wrote (several could be ticked): its first id the data has, else
/// the western one, else the first culture; `None` when there is none. A
/// stored list is read, never rewritten, until the user picks one.
pub fn shown_culture(ids: &[String], stored: Option<&[String]>) -> Option<usize> {
    stored
        .into_iter()
        .flatten()
        .find_map(|id| ids.iter().position(|c| c == id))
        .or_else(|| ids.iter().position(|c| c == WESTERN))
        .or((!ids.is_empty()).then_some(0))
}

/// Per culture, its constellations: its figures but the ones no longer
/// counted (`Kind::Obsolete`, behind their own switch).
pub fn culture_counts(c: &Catalogue) -> Vec<usize> {
    let mut counts = vec![0; c.cultures.len()];
    for f in c.figures.iter().filter(|f| f.kind != Kind::Obsolete) {
        if let Some(n) = counts.get_mut(f.culture) {
            *n += 1;
        }
    }
    counts
}

/// A figure id: ASCII letters and digits (`Ori`, `CMa`, `Noctua`).
pub fn figure_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A set id: lowercase letters, digits and dashes, a letter or digit first.
pub fn set_id_ok(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_ID
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// A name from the data, as shown: no control characters, trimmed, at most
/// `MAX_NAME_CHARS` characters.
fn clean_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Reads at most `max` bytes of `path`; more is an error (the file is not
/// read past the limit).
pub fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    file.take(max + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() as u64 > max {
        return Err(format!("larger than {} KiB", max >> 10));
    }
    Ok(out)
}

#[derive(Deserialize)]
struct LineEntry {
    a: [f64; 2],
    b: [f64; 2],
    #[serde(default)]
    t0: Option<f64>,
    #[serde(default)]
    t1: Option<f64>,
}

#[derive(Deserialize)]
struct SharedEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    label: Option<[f64; 2]>,
    #[serde(default)]
    lines: Vec<LineEntry>,
}

#[derive(Deserialize)]
struct SetFile {
    name: String,
    #[serde(default)]
    names: BTreeMap<String, String>,
    #[serde(default)]
    order: Option<i64>,
    #[serde(default)]
    culture: Option<String>,
}

#[derive(Deserialize)]
struct CultureFile {
    name: String,
    #[serde(default)]
    names: BTreeMap<String, String>,
}

fn names(name: &str, by_language: BTreeMap<String, String>) -> Result<Names, String> {
    let name = clean_name(name);
    if name.is_empty() {
        return Err("no name".into());
    }
    let names = by_language
        .into_iter()
        .filter(|(lang, _)| lang.len() <= 16 && lang.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .map(|(lang, n)| (lang, clean_name(&n)))
        .filter(|(_, n)| !n.is_empty())
        .take(64)
        .collect();
    Ok(Names { name, names })
}

#[derive(Deserialize)]
struct FigureFile {
    id: String,
    kind: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    centre: Option<[f64; 2]>,
    #[serde(default)]
    label: Option<[f64; 2]>,
    #[serde(default)]
    label_set: Option<[f64; 2]>,
    #[serde(default)]
    label_box_deg: Option<[f64; 2]>,
    #[serde(default)]
    label_set_box_deg: Option<[f64; 2]>,
    #[serde(default)]
    lines: Vec<LineEntry>,
    #[serde(default)]
    size: Option<[u32; 2]>,
    #[serde(default)]
    texel: Option<f64>,
    #[serde(default)]
    origin: Option<[f64; 2]>,
    #[serde(default)]
    hidden: Option<bool>,
}

fn angles(v: [f64; 2]) -> Option<[f32; 2]> {
    (v[0].is_finite() && v[1].is_finite() && (-90.0..=90.0).contains(&v[1]))
        .then(|| [(v[0].rem_euclid(360.0) as f32).to_radians(), (v[1] as f32).to_radians()])
}

fn lines(entries: &[LineEntry]) -> Result<Vec<Line>, String> {
    if entries.len() > MAX_LINES {
        return Err(format!("more than {MAX_LINES} lines"));
    }
    entries
        .iter()
        .map(|l| {
            let (Some(a), Some(b)) = (angles(l.a), angles(l.b)) else {
                return Err("a line's bad end".to_owned());
            };
            let (t0, t1) = (l.t0.unwrap_or(0.0), l.t1.unwrap_or(1.0));
            if !(0.0..=1.0).contains(&t0) || !(t0..=1.0).contains(&t1) {
                return Err("a line's bad times".to_owned());
            }
            Ok(Line {
                a,
                b,
                t0: t0 as f32,
                t1: t1 as f32,
            })
        })
        .collect()
}

/// `lines.json`: per figure id, its shared stick lines, name and label
/// point. A bad entry is left out (a problem), a bad file entirely.
pub fn parse_lines(bytes: &[u8], problems: &mut Vec<String>) -> BTreeMap<String, (String, Option<[f32; 2]>, Vec<Line>)> {
    let mut out = BTreeMap::new();
    let entries: BTreeMap<String, serde_json::Value> = match serde_json::from_slice(bytes) {
        Ok(e) => e,
        Err(e) => {
            problems.push(format!("lines.json: {e}"));
            return out;
        }
    };
    for (id, value) in entries {
        if !figure_id_ok(&id) {
            problems.push(format!(
                "lines.json: a bad figure id {:?}",
                id.chars().take(MAX_ID).collect::<String>()
            ));
            continue;
        }
        if out.len() >= MAX_FILE_FIGURES {
            problems.push(format!("lines.json: more than {MAX_FILE_FIGURES} figures; the rest left out"));
            break;
        }
        let entry: SharedEntry = match serde_json::from_value(value) {
            Ok(e) => e,
            Err(e) => {
                problems.push(format!("lines.json: {id}: {e}"));
                continue;
            }
        };
        let label = match entry.label {
            Some(l) => match angles(l) {
                Some(l) => Some(l),
                None => {
                    problems.push(format!("lines.json: {id}: a bad label point"));
                    continue;
                }
            },
            None => None,
        };
        match lines(&entry.lines) {
            Ok(l) => {
                out.insert(id, (entry.name.as_deref().map(clean_name).unwrap_or_default(), label, l));
            }
            Err(why) => problems.push(format!("lines.json: {id}: {why}")),
        }
    }
    out
}

/// `set.json` (its culture's index is set by the scan).
pub fn parse_set(id: &str, bytes: &[u8]) -> Result<SetInfo, String> {
    let f: SetFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let culture = f.culture.unwrap_or_else(|| WESTERN.to_owned());
    if !set_id_ok(&culture) {
        return Err("a culture id is lowercase letters, digits and dashes".into());
    }
    Ok(SetInfo {
        id: id.to_owned(),
        names: names(&f.name, f.names)?,
        order: f.order.unwrap_or(0),
        culture: 0,
        culture_id: culture,
    })
}

/// `culture.json`.
pub fn parse_culture(bytes: &[u8]) -> Result<Names, String> {
    let f: CultureFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    names(&f.name, f.names)
}

/// A culture's figures' names: per figure id, its name per language code.
pub type FigureNames = BTreeMap<String, BTreeMap<String, String>>;

/// A culture's `names.json`: `{"names": {"<language>": {"<figure id>":
/// "<name>"}}}`. A bad language, id or name is left out (one problem for
/// the file), a bad file entirely.
pub fn parse_names(bytes: &[u8], problems: &mut Vec<String>) -> FigureNames {
    let mut out = FigureNames::new();
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => {
            problems.push(format!("names.json: {e}"));
            return out;
        }
    };
    let Some(languages) = value.get("names").and_then(serde_json::Value::as_object) else {
        problems.push("names.json: no \"names\" object".to_owned());
        return out;
    };
    let mut bad = 0usize;
    for (lang, names) in languages.iter().take(MAX_LANGUAGES) {
        let Some(names) = names.as_object().filter(|_| lang_ok(lang)) else {
            bad += 1;
            continue;
        };
        for (id, name) in names.iter().take(MAX_FILE_FIGURES) {
            match name.as_str().map(clean_name) {
                Some(name) if figure_id_ok(id) && !name.is_empty() => {
                    out.entry(id.clone()).or_default().insert(lang.clone(), name);
                }
                _ => bad += 1,
            }
        }
    }
    if languages.len() > MAX_LANGUAGES {
        problems.push(format!("names.json: more than {MAX_LANGUAGES} languages; the rest left out"));
    }
    if bad > 0 {
        problems.push(format!("names.json: {bad} bad entries left out"));
    }
    out
}

/// The `names.json` at `path`, its problems said under `prefix` (its
/// folder, `cultures/<culture>/`, or none at the root); no file, no names.
fn read_names(path: &Path, prefix: &str, problems: &mut Vec<String>) -> FigureNames {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => match read_capped(path, MAX_NAMES_FILE) {
            Ok(bytes) => {
                let mut found = Vec::new();
                let names = parse_names(&bytes, &mut found);
                problems.extend(found.into_iter().map(|p| format!("{prefix}{p}")));
                names
            }
            Err(e) => {
                problems.push(format!("{prefix}names.json: {e}"));
                FigureNames::new()
            }
        },
        _ => FigureNames::new(),
    }
}

/// A language code: `en`, `zh-Latn`, `pt-BR` (letters, digits, dashes).
fn lang_ok(lang: &str) -> bool {
    !lang.is_empty() && lang.len() <= 16 && lang.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A figure's file in a set, checked.
#[derive(Clone, Debug)]
pub struct FigureMeta {
    pub kind: Kind,
    pub name: String,
    pub centre: Option<[f32; 2]>,
    pub label: Option<[f32; 2]>,
    /// The set's own label point, only with its box (`label_set_box_deg`).
    pub label_set: Option<[f32; 2]>,
    /// `label_box_deg`: the label point was placed clear of this drawing.
    pub label_placed: bool,
    pub lines: Vec<Line>,
    pub picture: Option<Picture>,
    pub hidden: bool,
}

/// `<figure>.json` of a set, its id `stem` (the file's name). Not a figure
/// file (no `kind`, another id): `Ok(None)`.
pub fn parse_figure(stem: &str, bytes: &[u8]) -> Result<Option<FigureMeta>, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if value.get("kind").is_none() || value.get("id").and_then(|v| v.as_str()) != Some(stem) {
        return Ok(None);
    }
    let f: FigureFile = serde_json::from_value(value).map_err(|e| e.to_string())?;
    debug_assert_eq!(f.id, stem);
    let kind = Kind::parse(&f.kind).ok_or_else(|| format!("unknown kind {:?}", f.kind.chars().take(20).collect::<String>()))?;
    // a name's box placed in the sky (degrees): both sides real and positive
    let placed = |b: Option<[f64; 2]>| b.is_some_and(|b| b.iter().all(|v| v.is_finite() && *v > 0.0));
    let point = |v: Option<[f64; 2]>, what: &str| match v {
        Some(v) => angles(v).map(Some).ok_or_else(|| format!("a bad {what}")),
        None => Ok(None),
    };
    let centre = point(f.centre, "centre")?;
    let label = point(f.label, "label point")?;
    // a set's own label point counts only where the set placed the name's
    // box there (`label_set_box_deg`): one without it came from a converter
    // and often sits on the figure's stars, so the figure's own point and
    // the stars' rule (`name_hang`) take the name instead
    let label_set = if placed(f.label_set_box_deg) {
        point(f.label_set, "set label point")?
    } else {
        None
    };
    let lines = lines(&f.lines)?;
    let picture = match f.size {
        Some([nx, ny]) if nx > 0 && ny > 0 => {
            if nx > ATLAS_W || ny > ATLAS_H {
                return Err(format!("a drawing of {nx} x {ny} texels, more than {ATLAS_W} x {ATLAS_H}"));
            }
            let (Some(texel), Some(origin)) = (f.texel, f.origin) else {
                return Err("a drawing without its texel or origin".into());
            };
            if !(texel.is_finite() && texel > 0.0 && texel < 0.1 && origin.iter().all(|o| o.is_finite() && o.abs() < 10.0)) {
                return Err("a drawing's bad frame".into());
            }
            if centre.is_none() {
                return Err("a drawing without its centre".into());
            }
            Some(Picture {
                texel: texel as f32,
                size: [nx, ny],
                origin: [origin[0] as f32, origin[1] as f32],
            })
        }
        _ => None,
    };
    Ok(Some(FigureMeta {
        kind,
        name: f.name.as_deref().map(clean_name).unwrap_or_default(),
        centre,
        label,
        label_set,
        label_placed: placed(f.label_box_deg),
        lines,
        picture,
        hidden: f.hidden == Some(true),
    }))
}

/// The folder's files, by name, sorted (only regular files and folders).
fn entries(dir: &Path) -> Vec<(String, std::fs::Metadata)> {
    let mut out: Vec<(String, std::fs::Metadata)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| Some((e.file_name().into_string().ok()?, std::fs::metadata(e.path()).ok()?)))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// What the folder holds, as a list of (file, length, modified) the scan
/// reads or points to: a scan is redone only when it changes.
pub type Signature = Vec<(String, u64, Option<std::time::SystemTime>)>;

pub fn signature(root: &Path) -> Signature {
    let mut out = Signature::new();
    let stamp = |name: String, m: &std::fs::Metadata| (name, m.len(), m.modified().ok());
    if let Ok(m) = std::fs::metadata(root.join("lines.json")) {
        out.push(stamp("lines.json".into(), &m));
    }
    // the planets' names (constellations_planets.rs)
    if let Ok(m) = std::fs::metadata(root.join("planets.json")) {
        out.push(stamp("planets.json".into(), &m));
    }
    if let Ok(m) = std::fs::metadata(root.join("names.json")) {
        out.push(stamp("names.json".into(), &m));
    }
    for top in ["cultures", "sets"] {
        for (sub, m) in entries(&root.join(top)).into_iter().take(MAX_SETS * 4) {
            if !m.is_dir() {
                continue;
            }
            for (file, fm) in entries(&root.join(top).join(&sub)).into_iter().take(MAX_SET_FIGURES * 4) {
                if fm.is_file() && (file.ends_with(".json") || file.ends_with(".webp")) {
                    out.push(stamp(format!("{top}/{sub}/{file}"), &fm));
                }
            }
        }
    }
    out
}

/// A culture's shared figures: per id, its name, label point and lines.
type Shared = BTreeMap<String, (String, Option<[f32; 2]>, Vec<Line>)>;

/// Reads the folder `root`: each culture's `lines.json` (and
/// `culture.json`), every `sets/*/set.json` and the figures' JSON files
/// (not their pictures). What does not read is left out and said in
/// `problems` (one line a file or entry, the caller logs each once). A
/// missing folder is an empty catalogue.
///
/// Within a culture, a figure's shared data (lines, label point, name)
/// comes from the culture's `lines.json` when it has the figure, else from
/// the first of the culture's sets, in the sets' order, that has the
/// figure's file; its kind and centre from that file. That file marking it
/// `"hidden": true` leaves it out whole.
pub fn scan(root: &Path, problems: &mut Vec<String>) -> Catalogue {
    let read_lines = |path: &Path, label: &str, problems: &mut Vec<String>| -> Option<Shared> {
        match std::fs::metadata(path) {
            Ok(m) if m.is_file() => match read_capped(path, MAX_LINES_FILE) {
                Ok(bytes) => {
                    let mut found = Vec::new();
                    let shared = parse_lines(&bytes, &mut found);
                    problems.extend(found.into_iter().map(|p| format!("{label}{}", p.trim_start_matches("lines.json"))));
                    Some(shared)
                }
                Err(e) => {
                    problems.push(format!("{label}: {e}"));
                    None
                }
            },
            _ => None,
        }
    };
    // the cultures: their folders, and the western one's old place
    // (each with its figures' names, `names.json`)
    let mut cultures: BTreeMap<String, (Option<Names>, Shared, FigureNames)> = BTreeMap::new();
    for (name, m) in entries(&root.join("cultures")) {
        if !m.is_dir() {
            continue;
        }
        if !set_id_ok(&name) {
            problems.push(format!(
                "cultures/{name}: a culture's folder name is lowercase letters, digits and dashes"
            ));
            continue;
        }
        if cultures.len() == MAX_SETS {
            problems.push(format!("more than {MAX_SETS} cultures; the rest left out"));
            break;
        }
        let dir = root.join("cultures").join(&name);
        let names = match std::fs::metadata(dir.join("culture.json")) {
            Ok(_) => match read_capped(&dir.join("culture.json"), MAX_SET_FILE).and_then(|b| parse_culture(&b)) {
                Ok(n) => Some(n),
                Err(e) => {
                    problems.push(format!("cultures/{name}/culture.json: {e}"));
                    None
                }
            },
            Err(_) => None,
        };
        let shared = read_lines(&dir.join("lines.json"), &format!("cultures/{name}/lines.json"), problems).unwrap_or_default();
        let figure_names = read_names(&dir.join("names.json"), &format!("cultures/{name}/"), problems);
        cultures.insert(name, (names, shared, figure_names));
    }
    let western_folder = cultures.contains_key(WESTERN);
    if !cultures.contains_key(WESTERN)
        && let Some(shared) = read_lines(&root.join("lines.json"), "lines.json", problems)
    {
        cultures.insert(WESTERN.to_owned(), (None, shared, FigureNames::new()));
    }
    // the sets, then their figures' files
    let mut sets: Vec<(SetInfo, PathBuf, BTreeMap<String, FigureMeta>)> = Vec::new();
    for (name, m) in entries(&root.join("sets")) {
        if !m.is_dir() {
            continue;
        }
        let dir = root.join("sets").join(&name);
        if !set_id_ok(&name) {
            problems.push(format!("sets/{name}: a set's folder name is lowercase letters, digits and dashes"));
            continue;
        }
        let info = match read_capped(&dir.join("set.json"), MAX_SET_FILE).and_then(|b| parse_set(&name, &b)) {
            Ok(info) => info,
            Err(e) => {
                problems.push(format!("sets/{name}/set.json: {e}"));
                continue;
            }
        };
        if sets.len() == MAX_SETS {
            problems.push(format!("more than {MAX_SETS} sets; the rest left out"));
            break;
        }
        if !cultures.contains_key(&info.culture_id) {
            if cultures.len() == MAX_SETS {
                problems.push(format!("sets/{name}: more than {MAX_SETS} cultures; left out"));
                continue;
            }
            cultures.insert(info.culture_id.clone(), (None, Shared::new(), FigureNames::new()));
        }
        let mut figures = BTreeMap::new();
        for (file, fm) in entries(&dir) {
            let Some(stem) = file.strip_suffix(".json") else { continue };
            if stem == "set" || !fm.is_file() {
                continue;
            }
            if !figure_id_ok(stem) {
                // only a figure id is read
                continue;
            }
            if figures.len() == MAX_SET_FIGURES {
                problems.push(format!("sets/{name}: more than {MAX_SET_FIGURES} figures; the rest left out"));
                break;
            }
            match read_capped(&dir.join(&file), MAX_FIGURE_FILE).and_then(|b| parse_figure(stem, &b)) {
                Ok(Some(meta)) => {
                    figures.insert(stem.to_owned(), meta);
                }
                Ok(None) => {}
                Err(e) => problems.push(format!("sets/{name}/{file}: {e}")),
            }
        }
        sets.push((info, dir, figures));
    }
    // the western culture with no folder: its names beside its old place
    if !western_folder && let Some(w) = cultures.get_mut(WESTERN) {
        w.2 = read_names(&root.join("names.json"), "", problems);
    }
    // the western culture first, then by id; the sets by their order, then id
    let mut order: Vec<String> = cultures.keys().cloned().collect();
    order.sort_by_key(|id| (id != WESTERN, id.clone()));
    let culture_index = |id: &str| order.iter().position(|c| c == id).unwrap_or(0);
    for s in &mut sets {
        s.0.culture = culture_index(&s.0.culture_id);
    }
    sets.sort_by(|a, b| (a.0.order, &a.0.id).cmp(&(b.0.order, &b.0.id)));
    let mut figures = Vec::new();
    'cultures: for (k, culture) in order.iter().enumerate() {
        let (_, shared, figure_names) = &cultures[culture];
        let mine: Vec<(usize, &(SetInfo, PathBuf, BTreeMap<String, FigureMeta>))> =
            sets.iter().enumerate().filter(|(_, s)| s.0.culture == k).collect();
        let ids: BTreeSet<&String> = shared.keys().chain(mine.iter().flat_map(|(_, s)| s.2.keys())).collect();
        for id in ids {
            let first = mine.iter().find_map(|(_, s)| s.2.get(id));
            if first.is_some_and(|m| m.hidden) {
                continue;
            }
            let (name, label, lines) = match (shared.get(id), first) {
                (Some((name, label, lines)), _) => (
                    if name.is_empty() {
                        first.map(|m| m.name.clone()).unwrap_or_default()
                    } else {
                        name.clone()
                    },
                    label.or(first.and_then(|m| m.label)),
                    lines.clone(),
                ),
                (None, Some(m)) => (m.name.clone(), m.label, m.lines.clone()),
                (None, None) => continue,
            };
            let kind = first.map_or(Kind::Modern, |m| m.kind);
            let mut drawings = Vec::new();
            for (index, (info, dir, files)) in &mine {
                let Some(meta) = files.get(id) else { continue };
                let (Some(picture), Some(centre)) = (meta.picture, meta.centre) else {
                    continue;
                };
                let image = dir.join(format!("{id}.webp"));
                match std::fs::metadata(&image) {
                    Ok(m) if m.is_file() && m.len() <= MAX_WEBP => drawings.push(Drawing {
                        set: *index,
                        centre,
                        picture,
                        label: meta.label_set,
                        label_placed: meta.label_placed,
                        image,
                    }),
                    Ok(m) if m.is_file() => problems.push(format!("sets/{}/{id}.webp: larger than {} MiB", info.id, MAX_WEBP >> 20)),
                    _ => problems.push(format!("sets/{}/{id}.webp: missing (the drawing is left out)", info.id)),
                }
            }
            let centre = first.and_then(|m| m.centre).or_else(|| mean_point(&lines)).or(label);
            let (Some(centre), Some(label)) = (centre, label.or(centre)) else {
                problems.push(format!("{culture}/{id}: no position"));
                continue;
            };
            // a figure with no line and no drawing keeps its name at its
            // point (a figure of one star); every culture is read, the
            // window and the GPU take the ticked ones
            if figures.len() == MAX_CATALOGUE_FIGURES {
                problems.push(format!("more than {MAX_CATALOGUE_FIGURES} figures; the rest left out"));
                break 'cultures;
            }
            figures.push(Figure {
                culture: k,
                // the catalogues name the western figures only
                name_key: if culture == WESTERN { super::names::key(id) } else { None },
                name: if name.is_empty() { id.clone() } else { name },
                kind,
                geom: FigureGeom { centre, label, lines },
                drawings,
                names: figure_names.get(id).cloned().unwrap_or_default(),
                id: id.clone(),
            });
        }
    }
    Catalogue {
        cultures: order
            .iter()
            .map(|id| CultureInfo {
                id: id.clone(),
                names: cultures[id].0.clone(),
            })
            .collect(),
        sets: sets.into_iter().map(|s| s.0).collect(),
        figures,
    }
}

/// The direction halfway among a figure's stars (radians), averaged as
/// unit vectors (right ascension wraps at 360°).
fn mean_point(lines: &[Line]) -> Option<[f32; 2]> {
    let mut sum = glam::Vec3::ZERO;
    for l in lines {
        sum += aurora_render::stars::equatorial(l.a[0], l.a[1]) + aurora_render::stars::equatorial(l.b[0], l.b[1]);
    }
    let v = sum.try_normalize()?;
    // equatorial(): x toward ra 0, y toward ra 90°, z the pole
    Some([v.y.atan2(v.x).rem_euclid(std::f32::consts::TAU), v.z.clamp(-1.0, 1.0).asin()])
}

/// A count of what the scan found, for the log and the settings page.
pub fn summary(c: &Catalogue) -> String {
    let cultures: Vec<&str> = c.cultures.iter().map(|k| k.id.as_str()).collect();
    let drawings: usize = c.figures.iter().map(|f| f.drawings.len()).sum();
    let mut per_set: HashMap<usize, usize> = HashMap::new();
    for d in c.figures.iter().flat_map(|f| &f.drawings) {
        *per_set.entry(d.set).or_default() += 1;
    }
    let sets: Vec<String> = c
        .sets
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{} {}", s.id, per_set.get(&i).copied().unwrap_or(0)))
        .collect();
    format!(
        "{} figures, {drawings} drawings ({}); cultures {}",
        c.figures.len(),
        sets.join(", "),
        cultures.join(", ")
    )
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A fresh folder for one test.
    pub(crate) fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-constellations-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test folder");
        dir
    }

    pub(crate) fn write(root: &Path, file: &str, bytes: &[u8]) {
        let p = root.join(file);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("dirs");
        std::fs::write(p, bytes).expect("write");
    }

    const ORI_LINES: &str = r#"{"Ori":{"name":"Orion","label":[88.0,22.8],"lines":[{"a":[78.6,-8.2],"b":[81.1,-2.4],"t0":0.0,"t1":0.5}]},
        "Aps":{"name":"Apus","label":[242.0,-80.3],"lines":[{"a":[222.0,-79.0],"b":[245.1,-78.7]}]}}"#;

    pub(crate) fn drawn(id: &str, extra: &str) -> String {
        format!(
            r#"{{"id":"{id}","kind":"modern","name":"N","centre":[83.0,3.2],"label":[88.0,22.8],"size":[10,12],"texel":0.0007,"origin":[0.1,0.2],"lines":[]{extra},"anchors":[[1,2,3]]}}"#
        )
    }

    pub(crate) fn set(root: &Path, id: &str, order: i64) {
        write(
            root,
            &format!("sets/{id}/set.json"),
            format!(r#"{{"name":"Set {id}","names":{{"fr":"Jeu {id}"}},"order":{order}}}"#).as_bytes(),
        );
    }

    #[test]
    fn a_name_keeps_its_point_only_where_the_shown_drawing_placed_it() {
        let read = |name: &str, files: &[(&str, &str, i64, &str)]| {
            let root = folder(name);
            write(&root, "lines.json", ORI_LINES.as_bytes());
            for (set_id, id, order, extra) in files {
                set(&root, set_id, *order);
                write(&root, &format!("sets/{set_id}/{id}.json"), drawn(id, extra).as_bytes());
                write(&root, &format!("sets/{set_id}/{id}.webp"), b"RIFF");
            }
            let mut problems = Vec::new();
            let c = scan(&root, &mut problems);
            assert!(problems.is_empty(), "{problems:?}");
            let placed = |id: &str| c.figures.iter().find(|f| f.id == id).expect(id).label_clear_of_drawing();
            (placed("Ori"), placed("Aps"))
        };
        let boxed = r#","label_box_deg":[4.9,1.2]"#;
        // the engravings placed Orion's point clear of their art; a set with
        // no placement (Apus) leaves its name to the stars' rule
        assert_eq!(
            read("placed", &[("engraved", "Ori", 1, boxed), ("art", "Aps", 2, "")]),
            (true, false)
        );
        // the drawing that shows decides: a later set's placement does not
        // count, nor a set label point without a box, nor an empty box
        assert_eq!(
            read("placed-later", &[("art", "Ori", 1, ""), ("engraved", "Ori", 2, boxed)]),
            (false, false)
        );
        assert_eq!(
            read("placed-set", &[("art", "Ori", 1, r#","label_set":[96.0,2.0]"#)]),
            (false, false)
        );
        assert_eq!(
            read("placed-empty", &[("art", "Ori", 1, r#","label_box_deg":[0.0,1.2]"#)]),
            (false, false)
        );
    }

    #[test]
    fn a_set_label_point_counts_only_with_its_box() {
        let point = |name: &str, extra: &str| {
            let root = folder(name);
            write(&root, "lines.json", ORI_LINES.as_bytes());
            set(&root, "art", 1);
            write(&root, "sets/art/Ori.json", drawn("Ori", extra).as_bytes());
            write(&root, "sets/art/Ori.webp", b"RIFF");
            let mut problems = Vec::new();
            let c = scan(&root, &mut problems);
            assert!(problems.is_empty(), "{problems:?}");
            let ori = c.figures.iter().find(|f| f.id == "Ori").expect("Ori");
            (ori.drawings[0].label, ori.label_clear_of_drawing(), ori.geom.label)
        };
        // a set that placed its name's box: its point moves the name there
        let (at, kept, _) = point("set-boxed", r#","label_set":[96.0,2.0],"label_set_box_deg":[4.0,1.0]"#);
        let want = [96.0f32.to_radians(), 2.0f32.to_radians()];
        assert!(
            at.is_some_and(|p| (p[0] - want[0]).abs() < 1e-6 && (p[1] - want[1]).abs() < 1e-6),
            "{at:?}"
        );
        assert!(!kept);
        // a converter's point (no box, or an empty or bad one): the figure's
        // own point, under the stars' rule while the drawing shows
        for (name, extra) in [
            ("set-bare", r#","label_set":[96.0,2.0]"#),
            ("set-empty", r#","label_set":[96.0,2.0],"label_set_box_deg":[0.0,1.0]"#),
            ("set-bad", r#","label_set":[96.0,2.0],"label_set_box_deg":[-4.0,1.0]"#),
        ] {
            let (at, kept, label) = point(name, extra);
            assert_eq!((at, kept), (None, false), "{name}");
            assert!((label[0] - 88.0f32.to_radians()).abs() < 1e-5, "{name}: lines.json's point");
        }
    }

    #[test]
    fn a_folder_reads_lines_sets_and_drawings() {
        let root = folder("reads");
        write(&root, "lines.json", ORI_LINES.as_bytes());
        set(&root, "second", 2);
        set(&root, "first", 1);
        write(&root, "sets/first/Ori.json", drawn("Ori", "").as_bytes());
        write(&root, "sets/first/Ori.webp", b"RIFF");
        write(
            &root,
            "sets/second/Ori.json",
            drawn("Ori", r#","label_set":[96.0,2.0],"label_set_box_deg":[4.0,1.0]"#).as_bytes(),
        );
        write(&root, "sets/second/Ori.webp", b"RIFF");
        // an obsolete figure only a set has, hidden in one: left out
        write(
            &root,
            "sets/second/Noctua.json",
            br#"{"id":"Noctua","kind":"obsolete","hidden":true,"lines":[]}"#,
        );
        write(
            &root,
            "sets/second/Felis.json",
            br#"{"id":"Felis","kind":"obsolete","name":"Felis","centre":[140,-30],"label":[140,-30],"lines":[{"a":[139,-29],"b":[141,-31]}]}"#,
        );
        // never read: credits, a manifest, a figure file of another id
        write(&root, "sets/first/credits.tsv", b"x\ty\n");
        write(&root, "sets/first/manifest.json", br#"{"kind":"not a figure"}"#);
        write(&root, "sets/first/Other.json", drawn("Ori", "").as_bytes());
        let mut problems = Vec::new();
        let c = scan(&root, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            c.sets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["first", "second"],
            "by order"
        );
        assert_eq!(c.sets[0].display_name("fr"), "Jeu first");
        assert_eq!(c.sets[0].display_name("ja"), "Set first");
        assert_eq!((c.cultures.len(), c.sets[1].culture_id.as_str()), (1, WESTERN));
        let ids: Vec<&str> = c.figures.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["Aps", "Felis", "Ori"]);
        let ori = &c.figures[2];
        assert_eq!(ori.drawings.iter().map(|d| d.set).collect::<Vec<_>>(), [0, 1]);
        assert!(ori.drawings[1].label.is_some() && ori.drawings[0].label.is_none());
        assert_eq!(ori.drawings[0].image, root.join("sets/first/Ori.webp"));
        // the lines are lines.json's (the set's file has none)
        assert_eq!(ori.geom.lines.len(), 1);
        assert_eq!(ori.name_key, Some("constellation-ori"));
        // a lines-only figure of lines.json alone: its centre among its stars
        let aps = &c.figures[0];
        assert!(aps.drawings.is_empty() && aps.kind == Kind::Modern);
        assert!((aps.geom.centre[1].to_degrees() + 79.0).abs() < 1.0, "{:?}", aps.geom.centre);
        assert_eq!(
            (c.figures[1].kind, c.figures[1].name.as_str(), c.figures[1].name_key),
            (Kind::Obsolete, "Felis", None)
        );
        // the signature sees a change
        let before = signature(&root);
        write(&root, "sets/first/Ori.webp", b"RIFF!");
        assert_ne!(before, signature(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_or_partial_folder_is_empty_or_lines_only() {
        let mut problems = Vec::new();
        let none = scan(&std::env::temp_dir().join("aurora-constellations-none-here"), &mut problems);
        assert!(none.is_empty() && none.sets.is_empty() && problems.is_empty());
        let root = folder("lines-only");
        write(&root, "lines.json", ORI_LINES.as_bytes());
        let c = scan(&root, &mut problems);
        assert_eq!((c.figures.len(), c.sets.len()), (2, 0));
        assert!(c.figures.iter().all(|f| f.drawings.is_empty()));
        // a set without set.json is no set; one with a bad name neither
        write(&root, "sets/loose/Ori.json", drawn("Ori", "").as_bytes());
        write(&root, "sets/Bad Name/set.json", br#"{"name":"x"}"#);
        let c = scan(&root, &mut problems);
        assert!(c.sets.is_empty());
        assert_eq!(problems.len(), 2, "{problems:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_files_are_left_out_never_a_panic() {
        let root = folder("bad");
        set(&root, "a", 0);
        // a drawing whose picture is missing, one too large, one with no frame
        write(&root, "sets/a/Ori.json", drawn("Ori", "").as_bytes());
        write(
            &root,
            "sets/a/Big.json",
            drawn("Big", "").replace("[10,12]", "[5000,12]").as_bytes(),
        );
        write(
            &root,
            "sets/a/Nof.json",
            drawn("Nof", "").replace(r#""texel":0.0007,"#, "").as_bytes(),
        );
        write(
            &root,
            "sets/a/Pole.json",
            drawn("Pole", "").replace("[88.0,22.8]", "[88.0,95.0]").as_bytes(),
        );
        write(&root, "sets/a/Kin.json", drawn("Kin", "").replace("modern", "ancient").as_bytes());
        write(&root, "sets/a/Tru.json", &drawn("Tru", "").as_bytes()[..40]);
        let mut problems = Vec::new();
        let c = scan(&root, &mut problems);
        // the one whose picture is missing keeps its name at its point
        let ids: Vec<&str> = c.figures.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["Ori"], "{ids:?}");
        assert!(c.figures[0].drawings.is_empty() && c.figures[0].geom.lines.is_empty());
        assert_eq!(problems.len(), 6, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("Ori.webp: missing")));
        // a file over its limit is not read past it
        write(&root, "lines.json", &vec![b' '; (MAX_LINES_FILE + 1) as usize]);
        problems.clear();
        let _ = scan(&root, &mut problems);
        assert!(problems.iter().any(|p| p.starts_with("lines.json: larger")), "{problems:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_parsers_take_any_bytes() {
        // fuzz-style: every prefix and some bytes flipped of good files
        let good = [
            ORI_LINES.as_bytes().to_vec(),
            drawn("Ori", r#","label_set":[96.0,2.0],"label_set_box_deg":[4.0,1.0]"#).into_bytes(),
        ];
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut problems = Vec::new();
        for bytes in &good {
            for n in 0..bytes.len() {
                let _ = parse_lines(&bytes[..n], &mut problems);
                let _ = parse_figure("Ori", &bytes[..n]);
                let _ = parse_set("a", &bytes[..n]);
            }
            for _ in 0..2000 {
                let mut b = bytes.clone();
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let at = (seed % b.len() as u64) as usize;
                b[at] = (seed >> 32) as u8;
                let _ = parse_lines(&b, &mut problems);
                let _ = parse_figure("Ori", &b);
            }
        }
        // odd values that parse as JSON
        for v in [
            r#"{"Ori":{"lines":[{"a":[1e400,0],"b":[0,0]}]}}"#,
            r#"{"Ori":{"lines":[{"a":[0,-91],"b":[0,0]}]}}"#,
            r#"{"Ori":{"lines":[{"a":[0,0],"b":[0,0],"t0":0.9,"t1":0.1}]}}"#,
            r#"{"../x":{"lines":[]}}"#,
            r#"[]"#,
            r#"{"Ori":null}"#,
        ] {
            problems.clear();
            assert!(parse_lines(v.as_bytes(), &mut problems).is_empty(), "{v}");
            assert_eq!(problems.len(), 1, "{v}: {problems:?}");
        }
        assert!(parse_figure("Ori", br#"{"id":"Ori","kind":"modern","origin":"x"}"#).is_err());
        assert!(
            parse_figure(
                "Ori",
                br#"{"id":"Ori","kind":"modern","size":[10,10],"texel":-1,"origin":[0,0],"centre":[0,0]}"#
            )
            .is_err()
        );
        assert!(matches!(parse_figure("Ori", br#"{"id":"Ori"}"#), Ok(None)), "no kind: not a figure");
        assert!(parse_set("a", br#"{"name":"  "}"#).is_err());
        assert_eq!(clean_name("A\u{0}b\nc"), "Abc");
        assert_eq!(clean_name(&"x".repeat(500)).len(), MAX_NAME_CHARS);
    }

    #[test]
    fn ids_keep_to_safe_characters() {
        for ok in ["Ori", "CMa", "TaurusPoniatowski", "a1"] {
            assert!(figure_id_ok(ok), "{ok}");
        }
        for bad in ["", "../Ori", "Ori.json", "O ri", "Orí", &"x".repeat(41)] {
            assert!(!figure_id_ok(bad), "{bad}");
        }
        for ok in ["atlas-1824", "a", "9-x"] {
            assert!(set_id_ok(ok), "{ok}");
        }
        for bad in ["", "-a", "Upper", "a/b", "..", "a_b"] {
            assert!(!set_id_ok(bad), "{bad}");
        }
    }

    fn drawings_of(sets: &[usize]) -> Vec<Drawing> {
        let picture = Picture {
            texel: 0.001,
            size: [1, 1],
            origin: [0.0, 0.0],
        };
        sets.iter()
            .map(|&set| Drawing {
                set,
                centre: [0.0, 0.0],
                picture,
                label: None,
                label_placed: false,
                image: PathBuf::new(),
            })
            .collect()
    }

    fn sets_of(ids: &[&str]) -> Vec<SetInfo> {
        ids.iter()
            .map(|id| SetInfo {
                id: (*id).into(),
                names: Names::default(),
                order: 0,
                culture: 0,
                culture_id: WESTERN.into(),
            })
            .collect()
    }

    #[test]
    fn every_set_is_used_the_first_that_draws_a_figure_wins() {
        // a figure drawn by sets a and c (its drawings in the sets' order):
        // a's, whatever was stored for the sets once
        assert_eq!(chosen(&drawings_of(&[0, 2])), Some(0));
        assert_eq!(chosen(&drawings_of(&[2])), Some(0));
        // a figure no set draws
        assert_eq!(chosen(&[]), None);
        // the sets' own list still reads, for their names and credits
        assert_eq!(sets_of(&["a", "b"]).len(), 2);
    }

    #[test]
    fn one_culture_shows_an_old_list_reads_as_its_first_known_id() {
        let ids = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<String>>();
        let cultures = ids(&["sky-b", WESTERN, "sky-c"]);
        // nothing stored, or an empty list (once "none"): the western one
        assert_eq!(shown_culture(&cultures, None), Some(1));
        assert_eq!(shown_culture(&cultures, Some(&[])), Some(1));
        // a list of older builds: its first id the data has
        assert_eq!(shown_culture(&cultures, Some(&ids(&["gone", "sky-c", "sky-b"]))), Some(2));
        assert_eq!(shown_culture(&cultures, Some(&ids(&[WESTERN, "sky-c"]))), Some(1));
        // none known: the western one; no western culture: the first
        assert_eq!(shown_culture(&cultures, Some(&ids(&["gone"]))), Some(1));
        assert_eq!(shown_culture(&ids(&["sky-b", "sky-c"]), Some(&ids(&["gone"]))), Some(0));
        assert_eq!(shown_culture(&[], None), None);
    }

    #[test]
    fn two_cultures_keep_their_own_figures_lines_and_sets() {
        let root = folder("cultures");
        write(&root, "lines.json", ORI_LINES.as_bytes());
        // another culture with a figure of the same id, its own lines and name
        write(
            &root,
            "cultures/other/lines.json",
            br#"{"Ori":{"name":"Hunter","label":[80.0,0.0],"lines":[{"a":[80,1],"b":[81,2]},{"a":[81,2],"b":[82,3]}]}}"#,
        );
        write(
            &root,
            "cultures/other/culture.json",
            br#"{"name":"Other sky","names":{"fr":"Autre ciel"}}"#,
        );
        set(&root, "west", 1);
        write(&root, "sets/west/Ori.json", drawn("Ori", "").as_bytes());
        write(&root, "sets/west/Ori.webp", b"RIFF");
        write(
            &root,
            "sets/oth/set.json",
            br#"{"name":"Other drawings","culture":"other","order":0}"#,
        );
        write(&root, "sets/oth/Ori.json", drawn("Ori", "").as_bytes());
        write(&root, "sets/oth/Ori.webp", b"RIFF");
        let mut problems = Vec::new();
        let c = scan(&root, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.cultures.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), [WESTERN, "other"]);
        assert_eq!(c.cultures[1].display_name("fr"), "Autre ciel");
        // the sets by order: oth (0), west (1); each in its culture
        assert_eq!(
            c.sets.iter().map(|s| (s.id.as_str(), s.culture)).collect::<Vec<_>>(),
            [("oth", 1), ("west", 0)]
        );
        // two figures Ori, one a culture, each with its own set's drawing
        let oris: Vec<&Figure> = c.figures.iter().filter(|f| f.id == "Ori").collect();
        assert_eq!(oris.len(), 2);
        let (west, other) = (oris[0], oris[1]);
        assert_eq!((west.culture, other.culture), (0, 1));
        assert_eq!(west.drawings.iter().map(|d| d.set).collect::<Vec<_>>(), [1]);
        assert_eq!(other.drawings.iter().map(|d| d.set).collect::<Vec<_>>(), [0]);
        assert_eq!((other.geom.lines.len(), other.name.as_str(), other.name_key), (2, "Hunter", None));
        assert_eq!(west.name_key, Some("constellation-ori"));
        // each figure takes its own culture's set's drawing
        assert_eq!((chosen(&west.drawings), chosen(&other.drawings)), (Some(0), Some(0)));
        assert_eq!(culture_counts(&c), [2, 1]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A culture of invented figures: `Aa` with lines and names in two
    /// languages, `Bb` of one star (a line of no length), `Cc` a name and a
    /// point only, `Dd` with no name anywhere.
    fn invented_culture(root: &Path) {
        write(
            root,
            "cultures/sky-b/lines.json",
            br#"{"Aa":{"name":"Ladle","label":[10,20],"lines":[{"a":[9,19],"b":[11,21]}]},
                "Bb":{"name":"Lamp","label":[40,-10],"lines":[{"a":[40,-10],"b":[40,-10]}]},
                "Cc":{"name":"Stone","label":[70,5],"lines":[]},
                "Dd":{"label":[100,0],"lines":[{"a":[99,0],"b":[101,1]}]}}"#,
        );
        write(root, "cultures/sky-b/culture.json", br#"{"name":"Sky B","names":{"fr":"Ciel B"}}"#);
        write(
            root,
            "cultures/sky-b/names.json",
            br#"{"names":{"en":{"Aa":"Big Ladle","Bb":"Lamp Star","Zz":"Nobody"},"fr":{"Aa":"Grande Louche"},
                "xx-Latn":{"Aa":"Lado"},"bad code":{"Aa":"x"},"de":{"Aa":7,"../x":"y"}}}"#,
        );
    }

    #[test]
    fn a_culture_names_its_figures_by_language_with_fallbacks() {
        let root = folder("names");
        write(&root, "lines.json", ORI_LINES.as_bytes());
        invented_culture(&root);
        let mut problems = Vec::new();
        let c = scan(&root, &mut problems);
        assert_eq!(problems, ["cultures/sky-b/names.json: 3 bad entries left out"], "{problems:?}");
        assert_eq!(c.cultures.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), [WESTERN, "sky-b"]);
        assert_eq!(c.cultures[1].display_name("fr"), "Ciel B");
        let fig = |id: &str| c.figures.iter().find(|f| f.culture == 1 && f.id == id).expect(id);
        // the language, the language without its region, English, the
        // lines' name, the id
        assert_eq!(fig("Aa").display_name("fr"), "Grande Louche");
        assert_eq!(fig("Aa").display_name("fr-CA"), "Grande Louche");
        assert_eq!(fig("Aa").display_name("ja"), "Big Ladle");
        assert_eq!(fig("Aa").display_name("xx-Latn"), "Lado");
        assert_eq!(fig("Cc").display_name("fr"), "Stone");
        assert_eq!(fig("Dd").display_name("en"), "Dd");
        // the western names stay the catalogues'
        assert_eq!(c.figures[1].display_name("ja"), aurora_i18n::text("constellation-ori"));
        // a figure of one star and a figure of a point only both stay, with
        // their names where their label is
        assert!(fig("Bb").geom.lines.len() == 1 && fig("Bb").drawings.is_empty());
        assert!(fig("Cc").geom.lines.is_empty() && fig("Cc").drawings.is_empty());
        assert!((fig("Cc").geom.label[0].to_degrees() - 70.0).abs() < 1e-3);
        assert_eq!(culture_counts(&c), [2, 4]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_names_parser_takes_any_bytes() {
        let good = br#"{"names":{"en":{"Aa":"One","Bb":"Two"},"fr":{"Aa":"Un"}}}"#;
        let mut problems = Vec::new();
        let names = parse_names(good, &mut problems);
        assert!(problems.is_empty());
        assert_eq!(names["Aa"]["fr"], "Un");
        for n in 0..good.len() {
            let _ = parse_names(&good[..n], &mut problems);
        }
        for v in [
            r#"[]"#,
            r#"{"names":[]}"#,
            r#"{"names":{"en":"x"}}"#,
            r#"{"names":{"en":{"Aa":null}}}"#,
        ] {
            problems.clear();
            assert!(parse_names(v.as_bytes(), &mut problems).is_empty(), "{v}");
            assert_eq!(problems.len(), 1, "{v}: {problems:?}");
        }
    }

    /// The western lines (a figure with a catalogue key) and an invented
    /// obsolete figure `Aa` of a set, without one.
    fn western_with_aa(root: &Path) {
        write(root, "lines.json", ORI_LINES.as_bytes());
        set(root, "first", 1);
        write(
            root,
            "sets/first/Aa.json",
            br#"{"id":"Aa","kind":"obsolete","name":"Alpha","centre":[10,20],"label":[10,20],"lines":[{"a":[9,19],"b":[11,21]}]}"#,
        );
    }

    /// A figure's name in `lang`.
    fn name_of(c: &Catalogue, id: &str, lang: &str) -> String {
        c.figures.iter().find(|f| f.id == id).expect(id).display_name(lang).to_owned()
    }

    #[test]
    fn the_western_names_file_comes_before_the_catalogues_in_its_languages() {
        let root = folder("western-names");
        western_with_aa(&root);
        let read = |root: &Path| {
            let mut problems = Vec::new();
            let c = scan(root, &mut problems);
            (c, problems)
        };
        // no file: the catalogues' and the data's names, as without it
        let (c, problems) = read(&root);
        assert!(problems.is_empty(), "{problems:?}");
        let key = aurora_i18n::text("constellation-ori");
        assert_eq!(name_of(&c, "Ori", "de"), key);
        assert_eq!(c.figures.iter().find(|f| f.id == "Aa").map(|f| f.kind), Some(Kind::Obsolete));
        assert_eq!(name_of(&c, "Aa", "de"), "Alpha");
        // the file in the language, and the language's base
        write(
            &root,
            "names.json",
            br#"{"names":{"de":{"Ori":"Alpha-x","Aa":"Beta-x"},"pt":{"Ori":"Gamma-x"},"en":{"Ori":"Delta-x"}}}"#,
        );
        let before = signature(&root);
        let (c, problems) = read(&root);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(name_of(&c, "Ori", "de"), "Alpha-x");
        assert_eq!(name_of(&c, "Ori", "pt-BR"), "Gamma-x");
        assert_eq!(name_of(&c, "Aa", "de"), "Beta-x");
        // a language the file lacks: the catalogue key, never the file's English
        assert_eq!(name_of(&c, "Ori", "ru"), key);
        // a change to the file is a change of the folder (a Rescan reads it)
        write(&root, "names.json", br#"{"names":{"de":{"Ori":"Epsilon-x"}}}"#);
        assert_ne!(before, signature(&root));
        // a bad file: said once, left out, the names as without it
        write(&root, "names.json", b"{\"names\": [");
        let (c, problems) = read(&root);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("names.json: "), "{problems:?}");
        assert_eq!(name_of(&c, "Ori", "de"), key);
        assert_eq!(name_of(&c, "Aa", "de"), "Alpha");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_western_folder_takes_its_own_names_file_not_the_roots() {
        let root = folder("western-folder-names");
        write(&root, "cultures/western/lines.json", ORI_LINES.as_bytes());
        write(&root, "cultures/western/names.json", br#"{"names":{"de":{"Ori":"Alpha-x"}}}"#);
        write(&root, "names.json", br#"{"names":{"de":{"Ori":"Beta-x"}}}"#);
        let mut problems = Vec::new();
        let c = scan(&root, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(name_of(&c, "Ori", "de"), "Alpha-x");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_figure_only_listed_by_the_first_set_takes_a_later_sets_drawing() {
        // the first set lists the figure (its kind and point, no picture),
        // the second draws it: one figure, of the first set's kind, with
        // the second set's drawing
        let root = folder("listed-first");
        set(&root, "a", 0);
        set(&root, "b", 1);
        write(
            &root,
            "sets/a/Qq.json",
            br#"{"id":"Qq","kind":"obsolete","name":"Old Q","centre":[30,10],"label":[31,11],"lines":[{"a":[29,9],"b":[31,11]}]}"#,
        );
        write(&root, "sets/b/Qq.json", drawn("Qq", "").as_bytes());
        write(&root, "sets/b/Qq.webp", b"RIFF");
        let mut problems = Vec::new();
        let c = scan(&root, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.figures.len(), 1);
        let q = &c.figures[0];
        assert_eq!((q.id.as_str(), q.kind, q.name.as_str()), ("Qq", Kind::Obsolete, "Old Q"));
        assert_eq!(q.drawings.iter().map(|d| d.set).collect::<Vec<_>>(), [1]);
        assert_eq!(chosen(&q.drawings), Some(0));
        // not counted among the constellations
        assert_eq!(culture_counts(&c), [0]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_hidden_figure_is_left_out_whatever_set_is_chosen() {
        let root = folder("hidden");
        set(&root, "a", 0);
        set(&root, "b", 1);
        write(
            &root,
            "sets/a/Noctua.json",
            br#"{"id":"Noctua","kind":"obsolete","hidden":true,"lines":[{"a":[1,1],"b":[2,2]}]}"#,
        );
        write(&root, "sets/b/Noctua.json", drawn("Noctua", "").as_bytes());
        write(&root, "sets/b/Noctua.webp", b"RIFF");
        let mut problems = Vec::new();
        assert!(scan(&root, &mut problems).is_empty(), "the first set's file hides it");
        let _ = std::fs::remove_dir_all(&root);
    }
}
