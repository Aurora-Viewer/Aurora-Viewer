//! Fallback fonts taken from the system, for the characters the embedded
//! fonts lack: Georgian, Armenian, Hebrew, Arabic, Japanese, Korean, Chinese,
//! Indic scripts, mathematical letters ("fancy text"), box drawing, arrows…
//! Residents put all of these in display names, group titles and chat, and
//! egui draws a square for a character no font of the family has.
//!
//! Same idea as Firestorm, whose `fonts.xml` appends installed system fonts
//! to every font (`<os name="Windows">` / `<os name="Mac">` in
//! indra/newview/skins/default/xui/en/fonts.xml, looked up by
//! LLFontRegistry::createFont in LLFontGL::getFontPathSystem), after its
//! bundled DejaVu Sans. The list here is the one of current Windows (one UI
//! font per script) instead of Firestorm's older one; the other systems are
//! best effort.
//!
//! Nothing is copied: the files are memory-mapped, so a 20 MB CJK font only
//! costs the pages of the glyphs actually drawn. Finding and checking them
//! happens on a background thread ([`start`]); the UI installs them once
//! [`ready`] (see `fonts::apply`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Once, OnceLock};

/// A script (or family of characters) and the installed fonts that can draw
/// it, best first. The first file that exists and has an outline for `probe`
/// is used; the others are then ignored.
pub struct Candidate {
    /// What the font is for (debug log).
    pub script: &'static str,
    /// A character of the script the face must be able to draw.
    pub probe: char,
    /// File names separated by `|`, whatever the directory and the case. A
    /// `#n` suffix asks for face `n` of a collection only, instead of its
    /// first face that draws `probe`.
    pub files: &'static str,
}

impl Candidate {
    fn files(&self) -> impl Iterator<Item = &'static str> {
        self.files.split('|')
    }
}

/// A file of [`Candidate::files`]: its name and the face it asks for.
fn face_spec(spec: &str) -> (&str, Option<u32>) {
    match spec.rsplit_once('#') {
        Some((file, face)) => match face.parse() {
            Ok(face) => (file, Some(face)),
            Err(_) => (spec, None),
        },
        None => (spec, None),
    }
}

const fn candidate(script: &'static str, probe: char, files: &'static str) -> Candidate {
    Candidate { script, probe, files }
}

/// Fallback order. Windows names first in each line (the release platform,
/// checked on Windows 11), then macOS (Firestorm's list) and common Linux
/// ones, both untested. Japanese comes before Korean and Chinese for the Han
/// characters they share, as in Firestorm (meiryo, MSGOTHIC, gulim, simhei,
/// msyh). For Japanese, the UI faces of the collections come first (Meiryo
/// UI, Yu Gothic UI, MS UI Gothic): their kana are proportional, where the
/// first faces give every kana a full em and space the Japanese interface
/// out like typewriter text.
#[rustfmt::skip]
pub const CANDIDATES: &[Candidate] = &[
    candidate("Georgian, Armenian", 'ღ', "segoeui.ttf|sylfaen.ttf|ARIALUNI.TTF|Arial Unicode.ttf|DejaVuSans.ttf|FreeSans.ttf"),
    candidate("Arabic", 'ب', "segoeui.ttf|tahoma.ttf|micross.ttf|arial.ttf|GeezaPro.ttc|NotoSansArabic-Regular.ttf|DejaVuSans.ttf"),
    candidate("Hebrew", 'א', "segoeui.ttf|tahoma.ttf|arial.ttf|ArialHB.ttc|NotoSansHebrew-Regular.ttf|DejaVuSans.ttf"),
    candidate("symbols, mathematical letters", '𝓐', "seguisym.ttf|cambria.ttc|STIXGeneral.otf|STIXTwoMath.otf|NotoSansMath-Regular.ttf|DejaVuSans.ttf"),
    candidate("box drawing, dingbats", '░', "seguisym.ttf|Apple Symbols.ttf|DejaVuSans.ttf|NotoSansSymbols2-Regular.ttf"),
    candidate("Japanese", 'あ', "meiryo.ttc#2|YuGothM.ttc#1|msgothic.ttc#1|meiryo.ttc|YuGothR.ttc|YuGothM.ttc|msgothic.ttc|ヒラギノ角ゴシック W3.ttc|NotoSansCJK-Regular.ttc|NotoSansCJKjp-Regular.otf"),
    candidate("Korean", '한', "malgun.ttf|gulim.ttc|AppleSDGothicNeo.ttc|NotoSansCJK-Regular.ttc|NotoSansCJKkr-Regular.otf"),
    candidate("Chinese (simplified)", '们', "msyh.ttc|simsun.ttc|simhei.ttf|PingFang.ttc|NotoSansCJK-Regular.ttc|NotoSansCJKsc-Regular.otf"),
    candidate("Chinese (traditional)", '們', "msjh.ttc|mingliub.ttc|PingFang.ttc|NotoSansCJKtc-Regular.otf"),
    candidate("Devanagari and other Indic scripts", 'ॐ', "Nirmala.ttc|Nirmala.ttf|mangal.ttf|Kohinoor.ttc|NotoSansDevanagari-Regular.ttf"),
    candidate("Tamil", 'ஐ', "Nirmala.ttc|Nirmala.ttf|latha.ttf|NotoSansTamil-Regular.ttf"),
    candidate("Thai, Lao, Khmer", 'ก', "LeelawUI.ttf|leelawad.ttf|tahoma.ttf|Thonburi.ttc|NotoSansThai-Regular.ttf"),
    candidate("Ethiopic, N'Ko, Tifinagh, Vai", 'ጀ', "ebrima.ttf|nyala.ttf|NotoSansEthiopic-Regular.ttf"),
    candidate("Cherokee, Canadian syllabics", 'ᗩ', "gadugi.ttf|euphemia.ttf|NotoSansCanadianAboriginal-Regular.ttf|DejaVuSans.ttf"),
    candidate("Tibetan", '༺', "himalaya.ttf|NotoSerifTibetan-Regular.ttf"),
    candidate("Javanese", '꧁', "javatext.ttf|NotoSansJavanese-Regular.ttf"),
    candidate("Yi", 'ꋬ', "msyi.ttf|NotoSansYi-Regular.ttf"),
    candidate("Mongolian", 'ᠠ', "monbaiti.ttf|NotoSansMongolian-Regular.ttf"),
    candidate("Myanmar", 'က', "mmrtext.ttf|NotoSansMyanmar-Regular.ttf"),
    candidate("Thaana", 'ހ', "mvboli.ttf|NotoSansThaana-Regular.ttf"),
    candidate("runes, ogham, old scripts", 'ᚱ', "seguihis.ttf|NotoSansRunic-Regular.ttf|DejaVuSans.ttf"),
    candidate("rare Han characters", '𠀀', "simsunb.ttf|mingliub.ttc"),
    // Balinese, Batak, Cham, Lepcha, Tai Tham… (Windows 11)
    candidate("other scripts", 'ᬅ', "SansSerifCollection.ttf|NotoSansBalinese-Regular.ttf"),
    // recent emoji missing from egui's Noto Emoji, as monochrome outlines
    candidate("recent emoji", '🫠', "seguiemj.ttf|NotoEmoji-Regular.ttf|NotoEmoji[wght].ttf"),
];

/// A face of an installed font, mapped and checked: ready for egui.
pub struct SystemFace {
    /// Name in egui's `FontDefinitions`.
    pub key: String,
    /// The whole file, memory-mapped for the life of the process.
    pub data: &'static [u8],
    /// Face in the file (collections).
    pub index: u32,
    face: ttf_parser::Face<'static>,
}

impl SystemFace {
    /// Does this face draw `c`?
    #[cfg(test)]
    pub fn has(&self, c: char) -> bool {
        self.face.glyph_index(c).is_some()
    }

    /// The face for egui, its glyphs moved onto the baseline of the primary
    /// font (`primary_shift`: the primary's own `y_offset_factor`).
    pub fn font_data(&self, primary: Option<RowMetrics>, primary_shift: f32) -> egui::FontData {
        let mut data = egui::FontData::from_static(self.data);
        data.index = self.index;
        if let (Some(primary), Some(own)) = (primary, RowMetrics::of(&self.face)) {
            data.tweak.y_offset_factor = primary_shift + baseline_shift(primary, own);
        }
        data
    }
}

/// Vertical metrics of a face, in em: what egui builds a text row from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowMetrics {
    /// Top of the row to the baseline.
    pub ascent: f32,
    /// Ascent + descent + line gap.
    pub height: f32,
}

impl RowMetrics {
    pub fn of(face: &ttf_parser::Face<'_>) -> Option<RowMetrics> {
        let em = f32::from(face.units_per_em());
        let (ascent, descent, gap) = (f32::from(face.ascender()), f32::from(face.descender()), f32::from(face.line_gap()));
        let height = (ascent - descent + gap) / em;
        (em > 0.0 && height > 0.0).then_some(RowMetrics {
            ascent: ascent / em,
            height,
        })
    }
}

/// How far down (fraction of the font size) the glyphs of a fallback face
/// must move to sit on the primary font's baseline. egui centers the row of
/// a fallback face on the primary's row (epaint text_layout:
/// `face ascent + 0.5 * (row height - face row height)`), which is right only
/// for faces with the same proportions: Yu Gothic's tall row put Japanese a
/// quarter of the size above the Latin text around it.
pub fn baseline_shift(primary: RowMetrics, face: RowMetrics) -> f32 {
    let centered = face.ascent + 0.5 * (primary.height - face.height);
    (primary.ascent - centered).clamp(-0.5, 0.5)
}

static FACES: OnceLock<Vec<SystemFace>> = OnceLock::new();

/// Look for the fallback fonts on a background thread (once). Call it early:
/// the scan is usually over before the first frame.
pub fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new().name("aurora-fonts".into()).spawn(|| {
            FACES.get_or_init(scan);
        });
        if let Err(e) = spawned {
            log::warn!("system fallback fonts: no thread ({e}), characters outside the embedded fonts stay squares");
            FACES.get_or_init(Vec::new);
        }
    });
}

/// The fallback faces, in order, once the scan is over (never blocks).
pub fn ready() -> Option<&'static [SystemFace]> {
    FACES.get().map(Vec::as_slice)
}

/// The fallback faces, waiting for the scan if it is still running: for the
/// recovery window and the tests, never for the viewer's frame.
pub fn wait() -> &'static [SystemFace] {
    start();
    FACES.get_or_init(scan)
}

/// `AURORA_FONT_FALLBACK=0`: embedded fonts only (comparisons, diagnostics).
fn enabled() -> bool {
    std::env::var("AURORA_FONT_FALLBACK").map_or(true, |v| v.trim() != "0")
}

fn scan() -> Vec<SystemFace> {
    if !enabled() {
        log::info!("system fallback fonts: off (AURORA_FONT_FALLBACK=0)");
        return Vec::new();
    }
    let start = std::time::Instant::now();
    let files = index_files(&super::font_dirs());
    let mut maps: HashMap<&'static str, memmap2::Mmap> = HashMap::new();
    let chosen = select(CANDIDATES, |spec, probe| {
        let (file, face) = face_spec(spec);
        if !maps.contains_key(file) {
            maps.insert(file, map_file(files.get(&file.to_lowercase())?)?);
        }
        face_with(maps.get(file)?, probe, face)
    });
    let mut faces = Vec::with_capacity(chosen.len());
    let mut kept: HashMap<&'static str, &'static [u8]> = HashMap::new();
    for (file, index, script) in chosen {
        let data = match kept.get(file) {
            Some(data) => *data,
            None => {
                let Some(map) = maps.remove(file) else { continue };
                // The mapping lives as long as the process: egui keeps the
                // font for good, and a leaked mapping is what makes the
                // slice 'static without copying the file.
                let map: &'static memmap2::Mmap = Box::leak(Box::new(map));
                kept.insert(file, &map[..]);
                &map[..]
            }
        };
        let Ok(face) = ttf_parser::Face::parse(data, index) else {
            continue;
        };
        log::debug!("system fallback font for {script}: {file}, face {index}");
        faces.push(SystemFace {
            key: format!("system:{}#{index}", file.to_lowercase()),
            data,
            index,
            face,
        });
    }
    let mapped: usize = kept.values().map(|d| d.len()).sum();
    log::info!(
        "system fallback fonts: {} faces ({:.1} MB mapped) in {:.1} ms: {}",
        faces.len(),
        mapped as f64 / 1_048_576.0,
        start.elapsed().as_secs_f64() * 1000.0,
        faces
            .iter()
            .map(|f| f.key.trim_start_matches("system:"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    faces
}

/// The faces to install, in fallback order: for each candidate, its first
/// file where `probe(file, character)` finds a face; a face already taken by
/// an earlier candidate is not repeated. Returns (file, face index, script).
fn select(candidates: &[Candidate], mut probe: impl FnMut(&'static str, char) -> Option<u32>) -> Vec<(&'static str, u32, &'static str)> {
    let mut out: Vec<(&'static str, u32, &'static str)> = Vec::new();
    for c in candidates {
        let Some((file, index)) = c.files().find_map(|f| probe(f, c.probe).map(|i| (face_spec(f).0, i))) else {
            continue;
        };
        if !out.iter().any(|(f, i, _)| f.eq_ignore_ascii_case(file) && *i == index) {
            out.push((file, index, c.script));
        }
    }
    out
}

/// Font files of the directories (and their subdirectories), by lowercase
/// file name; the first directory wins.
fn index_files(dirs: &[PathBuf]) -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    let mut stack: Vec<PathBuf> = dirs.iter().rev().cloned().collect();
    let mut visited = 0;
    while let Some(dir) = stack.pop() {
        visited += 1;
        if visited > 512 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            let ext = path.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
            if !matches!(ext.as_deref(), Some("ttf" | "otf" | "ttc")) {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                out.entry(name.to_lowercase()).or_insert(path);
            }
        }
    }
    out
}

fn map_file(path: &Path) -> Option<memmap2::Mmap> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: the font file is opened read-only and never written by the
    // viewer. Windows refuses to truncate or delete a mapped file; elsewhere
    // system fonts are replaced by renaming, which leaves this mapping whole.
    unsafe { memmap2::Mmap::map(&file) }
        .inspect_err(|e| log::warn!("system fallback font {}: {e}", path.display()))
        .ok()
}

/// The first face of a font file (a collection has several) that has an
/// outline for `c`, or face `only` if it has one.
///
/// This is also what keeps a damaged file away from egui, which panics on a
/// face it cannot open: ttf-parser wants the same header and table directory
/// as egui's parser, and the `head`, `hhea` and `maxp` tables on top.
fn face_with(data: &[u8], c: char, only: Option<u32>) -> Option<u32> {
    let faces = ttf_parser::fonts_in_collection(data).unwrap_or(1).min(64);
    let usable = |i: u32| ttf_parser::Face::parse(data, i).is_ok_and(|face| draws(&face, c));
    match only {
        Some(i) => (i < faces && usable(i)).then_some(i),
        None => (0..faces).find(|&i| usable(i)),
    }
}

/// A glyph with an outline: egui draws outlines only, so a face that has
/// just a bitmap or a color layer for `c` would show nothing.
fn draws(face: &ttf_parser::Face<'_>, c: char) -> bool {
    struct Sink;
    impl ttf_parser::OutlineBuilder for Sink {
        fn move_to(&mut self, _: f32, _: f32) {}
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    face.glyph_index(c)
        .is_some_and(|glyph| face.outline_glyph(glyph, &mut Sink).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(probe: char, files: &'static str) -> Candidate {
        super::candidate("test", probe, files)
    }

    #[test]
    fn first_usable_file_of_each_candidate_in_order() {
        let list = [
            candidate('a', "missing.ttf|one.ttf|two.ttf"),
            candidate('b', "two.ttf"),
            candidate('c', "nowhere.ttf"),
        ];
        let chosen = select(&list, |file, _| matches!(file, "one.ttf" | "two.ttf").then_some(0));
        assert_eq!(
            chosen.iter().map(|c| (c.0, c.1)).collect::<Vec<_>>(),
            [("one.ttf", 0), ("two.ttf", 0)]
        );
    }

    #[test]
    fn a_face_serves_several_candidates_once() {
        // one file for two scripts; a collection whose second face has the
        // other character is a different face and is kept
        let list = [
            candidate('a', "Shared.ttf"),
            candidate('b', "shared.ttf"),
            candidate('c', "collection.ttc"),
            candidate('m', "collection.ttc"),
        ];
        let chosen = select(&list, |file, c| match (file.to_lowercase().as_str(), c) {
            ("shared.ttf", _) => Some(0),
            ("collection.ttc", 'c') => Some(0),
            ("collection.ttc", 'm') => Some(1),
            _ => None,
        });
        assert_eq!(
            chosen.iter().map(|c| (c.0, c.1)).collect::<Vec<_>>(),
            [("Shared.ttf", 0), ("collection.ttc", 0), ("collection.ttc", 1)]
        );
    }

    #[test]
    fn a_file_without_the_probe_is_skipped() {
        let list = [candidate('ღ', "latin-only.ttf|georgian.ttf")];
        let chosen = select(&list, |file, _| (file == "georgian.ttf").then_some(0));
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].0, "georgian.ttf");
    }

    #[test]
    fn candidates_are_well_formed() {
        for c in CANDIDATES {
            assert!(c.files().all(|f| !f.is_empty() && f.contains('.')), "{}", c.script);
            // every probe is a character the embedded fonts cannot draw:
            // otherwise the candidate would never be asked for it
            assert!(
                !super::super::embedded_has(c.probe),
                "{} is already embedded ({})",
                c.probe,
                c.script
            );
        }
    }

    #[test]
    fn embedded_font_draws_its_probe_and_no_other_script() {
        let noto = include_bytes!("../../../assets/fonts/NotoSans-Regular.ttf");
        assert_eq!(face_with(noto, 'A', None), Some(0));
        assert_eq!(face_with(noto, 'Я', None), Some(0));
        assert_eq!(face_with(noto, 'ღ', None), None);
        assert_eq!(face_with(noto, '中', None), None);
        assert_eq!(face_with(b"not a font", 'A', None), None);
        assert_eq!(face_with(noto, 'A', Some(0)), Some(0));
        // a single font has no second face
        assert_eq!(face_with(noto, 'A', Some(1)), None);
    }

    #[test]
    fn a_face_suffix_asks_for_that_face_of_the_collection() {
        assert_eq!(face_spec("meiryo.ttc#2"), ("meiryo.ttc", Some(2)));
        assert_eq!(face_spec("YuGothM.ttc"), ("YuGothM.ttc", None));
        assert_eq!(face_spec("odd#name.ttf"), ("odd#name.ttf", None));
        // the face asked for is the one kept, under the file's own name
        let list = [candidate('あ', "ui.ttc#1|ui.ttc")];
        let chosen = select(&list, |spec, _| match face_spec(spec) {
            ("ui.ttc", Some(1)) => Some(1),
            ("ui.ttc", None) => Some(0),
            _ => None,
        });
        assert_eq!(chosen.iter().map(|c| (c.0, c.1)).collect::<Vec<_>>(), [("ui.ttc", 1)]);
    }

    #[test]
    fn files_are_indexed_by_lowercase_name_first_directory_first() {
        let root = std::env::temp_dir().join(format!("aurora-font-index-{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("b").join("nested"));
        std::fs::create_dir_all(&a).expect("temp dir");
        std::fs::create_dir_all(&b).expect("temp dir");
        for path in [a.join("Demo.TTF"), b.join("demo.ttf"), b.join("Other.otf"), b.join("notes.txt")] {
            std::fs::write(path, b"x").expect("temp file");
        }
        let files = index_files(&[a.clone(), root.join("b"), root.join("absent")]);
        assert_eq!(files.get("demo.ttf"), Some(&a.join("Demo.TTF")));
        assert_eq!(files.get("other.otf"), Some(&b.join("Other.otf")));
        assert_eq!(files.len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}
