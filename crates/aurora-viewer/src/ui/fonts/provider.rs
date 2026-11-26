//! Fonts found on demand, for the characters none of the faces installed up
//! front can draw: the rare scripts the candidate list does not name (the
//! Cuneiform sign `𒀭`), the files whose name differs from the one it asks
//! for (macOS's `EuphemiaCAS.ttc` for Canadian syllabics), and any font the
//! user installed in their own `~/Library/Fonts`.
//!
//! egui asks its [`FontProvider`]s only after every configured face has
//! missed a character, and appends the face we return to the family's
//! fallback chain (epaint `Family::discover`). The index is built once, off
//! the UI thread: one entry per face, with the Unicode scripts its cmap
//! covers. `ttf-parser` reads it, like the configured fallback list; nothing
//! is copied and a file is mapped only when a character first needs it.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once, OnceLock};

use egui::{FallbackRequest, FontData, FontFamily, FontInsert, FontPriority, FontProvider, InsertFontFamily};
use unicode_script::{Script, UnicodeScript};

use super::fallback::{self, RowMetrics};

/// Where a face is and what it can draw. The file itself is not kept here:
/// `font_for` maps the chosen one, so the index costs no font data at rest.
#[derive(Debug, Clone)]
struct FaceRef {
    path: PathBuf,
    /// Face of a collection, 0 for a single-face file.
    index: u32,
    /// egui's name for the face, the same one `fallback::scan` gives it.
    key: String,
    family: String,
    regular: bool,
    /// Codepoints of the scripts below, for ranking the scripts' faces.
    coverage: u32,
}

/// The installed faces and, per script, those that draw it, best first.
#[derive(Default)]
pub struct Index {
    faces: Vec<FaceRef>,
    by_script: HashMap<Script, Vec<(usize, u32)>>,
    /// Every face, for a character whose script is Common, Inherited or
    /// Unknown (punctuation, a new symbol): the first that has it is used.
    all: Vec<(usize, u32)>,
    /// Memory-mapped files, kept so several scripts and families reuse one.
    maps: Mutex<HashMap<PathBuf, Arc<memmap2::Mmap>>>,
    /// Characters no installed face has, so a repeated miss is not scanned
    /// again in every family.
    misses: Mutex<HashSet<char>>,
}

/// What a discovered face must align to: the primary font's row, its optical
/// shift, and the interface's text scale (the faces installed up front are
/// scaled by `fonts::install`; a face discovered later has to be too).
#[derive(Clone, Copy)]
struct Baseline {
    primary: Option<RowMetrics>,
    shift: f32,
    scale: f32,
}

impl Default for Baseline {
    fn default() -> Self {
        Self {
            primary: None,
            shift: 0.0,
            scale: 1.0,
        }
    }
}

static BASELINE: Mutex<Option<Baseline>> = Mutex::new(None);

/// Keep the current primary font's metrics for the faces discovered later.
pub fn set_baseline(primary: Option<RowMetrics>, shift: f32, scale: f32) {
    if let Ok(mut baseline) = BASELINE.lock() {
        *baseline = Some(Baseline { primary, shift, scale });
    }
}

fn baseline() -> Baseline {
    BASELINE.lock().ok().and_then(|baseline| *baseline).unwrap_or_default()
}

/// Faces whose family is a usual interface face come first among faces that
/// cover a script equally well; a dedicated Noto face still wins on coverage.
const PREFERRED: &[&str] = &[
    "segoe ui",
    "sf pro",
    "helvetica",
    "arial",
    "noto sans",
    "noto serif",
    "dejavu sans",
    "euphemia",
    "gadugi",
    "nirmala",
    "leelawadee",
    "kohinoor",
    "tahoma",
    "geneva",
];

fn family_rank(family: &str) -> usize {
    let family = family.to_lowercase();
    PREFERRED
        .iter()
        .position(|preferred| family.contains(preferred))
        .unwrap_or(PREFERRED.len())
}

impl Index {
    /// The installed faces for a character, best first.
    fn candidates(&self, c: char) -> &[(usize, u32)] {
        let script = c.script();
        if matches!(script, Script::Common | Script::Inherited | Script::Unknown) {
            return &self.all;
        }
        self.by_script.get(&script).map_or(&[], Vec::as_slice)
    }

    fn map(&self, path: &PathBuf) -> Option<Arc<memmap2::Mmap>> {
        if let Ok(maps) = self.maps.lock()
            && let Some(map) = maps.get(path)
        {
            return Some(Arc::clone(map));
        }
        let map = Arc::new(fallback::map_file(path)?);
        if let Ok(mut maps) = self.maps.lock() {
            maps.entry(path.clone()).or_insert_with(|| Arc::clone(&map));
        }
        Some(map)
    }

    /// The face for `c`, as egui's insertion for `family`, or `None` when no
    /// installed face has an outline glyph for it.
    fn font_for(&self, family: &FontFamily, c: char) -> Option<FontInsert> {
        if self.misses.lock().is_ok_and(|misses| misses.contains(&c)) {
            return None;
        }
        let baseline = baseline();
        for &(face_index, _) in self.candidates(c) {
            let Some(face) = self.faces.get(face_index) else {
                continue;
            };
            let Some(map) = self.map(&face.path) else {
                continue;
            };
            let Ok(parsed) = ttf_parser::Face::parse(&map, face.index) else {
                continue;
            };
            if !fallback::draws(&parsed, c) {
                continue;
            }
            let own = RowMetrics::of(&parsed);
            // Shared bytes: egui keeps the mapping, not a copy of the file.
            let blob: egui::epaint::text::Blob = map;
            let mut data = FontData::from_blob(blob, face.index);
            data.tweak.scale = baseline.scale;
            if let (Some(primary), Some(own)) = (baseline.primary, own) {
                data.tweak.y_offset_factor = baseline.shift + fallback::baseline_shift(primary, own);
            }
            log::debug!(
                "system fallback on demand for U+{:04X}: {} ({})",
                c as u32,
                face.key.trim_start_matches("system:"),
                face.family
            );
            return Some(FontInsert::new(
                &face.key,
                data,
                vec![InsertFontFamily {
                    family: family.clone(),
                    priority: FontPriority::Lowest,
                }],
            ));
        }
        if let Ok(mut misses) = self.misses.lock() {
            misses.insert(c);
        }
        None
    }
}

/// The face's family name and whether it is a regular (not bold or italic)
/// face. A font without a usable name is still ranked, just last.
fn face_family(face: &ttf_parser::Face<'_>) -> (String, bool) {
    let mut family = None;
    for name in face.names() {
        let english = name.language_id == 0x0409 || !name.is_unicode() || name.platform_id == ttf_parser::PlatformId::Unicode;
        if name.name_id == ttf_parser::name_id::FAMILY && english {
            family = name.to_string().or(family);
        }
    }
    let regular = !face.is_bold() && !face.is_italic();
    (family.unwrap_or_default().trim().to_owned(), regular)
}

fn rank(face: &FaceRef, coverage: u32) -> (u8, u32, usize, &str) {
    // Regular faces first, then the largest coverage of the script, then a
    // usual interface family, then a stable name.
    (u8::from(!face.regular), u32::MAX - coverage, family_rank(&face.family), &face.key)
}

fn build() -> Index {
    if !fallback::enabled() {
        log::info!("system fallback index: off (AURORA_FONT_FALLBACK=0)");
        return Index::default();
    }
    let mut dirs = super::font_dirs();
    let extra = super::extra_font_dirs(&dirs);
    dirs.extend(extra);
    #[cfg(target_os = "macos")]
    dirs.extend(super::macos_asset_dirs());
    build_from(&dirs)
}

fn build_from(dirs: &[PathBuf]) -> Index {
    let start = std::time::Instant::now();
    let mut files: Vec<(String, PathBuf)> = fallback::index_files(dirs).into_iter().collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut index = Index::default();
    for (name, path) in files {
        // The fallback of last resort draws a labelled box for every
        // codepoint: it must never be chosen over a real face.
        let lower = name.to_lowercase();
        if lower.contains("lastresort") || lower.contains("last_resort") || lower.contains("emoji") {
            continue;
        }
        let Some(map) = fallback::map_file(&path) else {
            continue;
        };
        let faces = ttf_parser::fonts_in_collection(&map).unwrap_or(1).min(64);
        for face_index in 0..faces {
            let Ok(face) = ttf_parser::Face::parse(&map, face_index) else {
                continue;
            };
            let (family, regular) = face_family(&face);
            if family.to_lowercase().contains("last resort") {
                continue;
            }
            let mut seen = HashSet::new();
            let mut counts: HashMap<Script, u32> = HashMap::new();
            if let Some(cmap) = face.tables().cmap {
                for subtable in cmap.subtables {
                    if !subtable.is_unicode() {
                        continue;
                    }
                    subtable.codepoints(|codepoint| {
                        if !seen.insert(codepoint) {
                            return;
                        }
                        let Some(c) = char::from_u32(codepoint) else {
                            return;
                        };
                        let script = c.script();
                        if !matches!(script, Script::Common | Script::Inherited | Script::Unknown) {
                            *counts.entry(script).or_default() += 1;
                        }
                    });
                }
            }
            let coverage: u32 = counts.values().sum();
            let at = index.faces.len();
            index.faces.push(FaceRef {
                path: path.clone(),
                index: face_index,
                key: format!("system:{}#{face_index}", name.to_lowercase()),
                family,
                regular,
                coverage,
            });
            if coverage == 0 {
                index.all.push((at, 0));
            } else {
                for (script, count) in counts {
                    index.by_script.entry(script).or_default().push((at, count));
                }
                index.all.push((at, coverage));
            }
        }
    }
    let faces = &index.faces;
    for lists in index.by_script.values_mut() {
        lists.sort_by(|a, b| rank(&faces[a.0], a.1).cmp(&rank(&faces[b.0], b.1)));
    }
    index.all.sort_by(|a, b| rank(&faces[a.0], a.1).cmp(&rank(&faces[b.0], b.1)));
    log::info!(
        "system fallback index: {} faces, {} scripts, {} codepoints in {:.1} ms",
        index.faces.len(),
        index.by_script.len(),
        faces.iter().map(|face| u64::from(face.coverage)).sum::<u64>(),
        start.elapsed().as_secs_f64() * 1000.0
    );
    index
}

static STARTED: Once = Once::new();
static INDEX: OnceLock<Index> = OnceLock::new();

/// Build the index on a background thread (once). Call it early, next to
/// `fallback::start`; the UI installs the provider when [`ready`] is true.
pub fn start() {
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new().name("aurora-font-index".into()).spawn(|| {
            INDEX.get_or_init(build);
        });
        if let Err(e) = spawned {
            log::warn!("system fallback index: no thread ({e}), characters outside the configured fonts stay squares");
            INDEX.get_or_init(Index::default);
        }
    });
}

/// Is the index built (or known to be empty)? Never blocks.
pub fn ready() -> bool {
    INDEX.get().is_some()
}

/// The index, waiting for the scan: for tests, never for the viewer's frame.
#[cfg(test)]
pub fn wait() -> &'static Index {
    start();
    INDEX.get_or_init(build)
}

/// The provider for the viewer's context, once the index is ready. Adds it
/// only once; the addition makes egui reload its fonts, which clears the
/// squares cached before the index was there.
pub fn install(ctx: &egui::Context) -> bool {
    if !fallback::enabled() {
        return true;
    }
    if !ready() {
        return false;
    }
    let Some(index) = INDEX.get() else {
        return false;
    };
    let id = egui::Id::unique("aurora-system-font-provider");
    if ctx.data(|data| data.get_temp::<bool>(id).unwrap_or(false)) {
        return true;
    }
    log::info!("system fallback on demand: provider installed ({} faces)", index.faces.len());
    ctx.add_font_provider(Arc::new(SystemFontProvider(index)));
    ctx.data_mut(|data| data.insert_temp(id, true));
    true
}

/// Asks the index for a face only after every configured face has missed
/// the character.
struct SystemFontProvider(&'static Index);

impl FontProvider for SystemFontProvider {
    fn font_for(&self, request: &FallbackRequest<'_>) -> Option<FontInsert> {
        let c = request.cluster.chars().next()?;
        self.0.font_for(request.family, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-font-provider-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn egui_finds_a_character_from_the_provider() {
        let dir = temp_dir("egui");
        let path = dir.join("Noto.ttf");
        std::fs::write(&path, include_bytes!("../../../assets/fonts/NotoSans-Regular.ttf")).expect("font");
        let index: &'static Index = Box::leak(Box::new(build_from(std::slice::from_ref(&dir))));
        let ctx = egui::Context::default();
        let mut definitions = egui::FontDefinitions::empty();
        definitions.font_data.insert(
            "test-inter".into(),
            Arc::new(FontData::from_static(include_bytes!("../../../assets/fonts/Inter-Regular.ttf"))),
        );
        definitions.families.insert(FontFamily::Proportional, vec!["test-inter".into()]);
        ctx.set_fonts(definitions);
        ctx.add_font_provider(Arc::new(SystemFontProvider(index)));
        let mut has = false;
        let _ = ctx.run_ui(Default::default(), |ui| {
            has = ui.fonts_mut(|fonts| fonts.has_glyph(&egui::FontId::proportional(14.0), 'ᴄ'));
        });
        assert!(has, "the provider draws a small capital Inter lacks");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shipped_macos_fonts_draw_the_reported_characters() {
        if !fallback::enabled() {
            return;
        }
        let installed = |paths: &[&str]| paths.iter().any(|path| PathBuf::from(path).exists());
        let index = wait();
        let cases = [
            (
                'ᐰ',
                [
                    "/System/Library/Fonts/Supplemental/EuphemiaCAS.ttc",
                    "/System/Library/Fonts/Supplemental/NotoSansCanadianAboriginal-Regular.otf",
                ],
            ),
            (
                '𒀭',
                [
                    "/System/Library/Fonts/Supplemental/NotoSansCuneiform-Regular.ttf",
                    "/Library/Fonts/NotoSansCuneiform-Regular.ttf",
                ],
            ),
        ];
        for (c, paths) in cases {
            if installed(&paths) {
                assert!(
                    index.font_for(&FontFamily::Proportional, c).is_some(),
                    "an installed font draws U+{:04X} {c}",
                    c as u32
                );
            } else {
                eprintln!("no installed font for U+{:04X} {c}: {paths:?}", c as u32);
            }
        }
    }

    #[test]
    fn an_installed_face_is_indexed_with_its_scripts() {
        let dir = temp_dir("script");
        let path = dir.join("Noto.ttf");
        std::fs::write(&path, include_bytes!("../../../assets/fonts/NotoSans-Regular.ttf")).expect("font");
        let index = build_from(std::slice::from_ref(&dir));
        let canadians = index.by_script.get(&Script::Canadian_Aboriginal).map_or(0, Vec::len);
        assert_eq!(&canadians, &0, "Noto Sans has no Canadian syllabics");
        let latin = index.by_script.get(&Script::Latin).map_or(0, Vec::len);
        assert!(latin > 0, "Noto Sans covers Latin");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_face_is_found_by_its_char_and_not_for_a_missing_one() {
        let dir = temp_dir("find");
        let path = dir.join("Noto.ttf");
        std::fs::write(&path, include_bytes!("../../../assets/fonts/NotoSans-Regular.ttf")).expect("font");
        let index = build_from(std::slice::from_ref(&dir));
        let insert = index.font_for(&FontFamily::Proportional, 'ᴄ').expect("Noto Sans small capital");
        assert_eq!(insert.name, "system:noto.ttf#0");
        assert!(
            index.font_for(&FontFamily::Proportional, '𒀭').is_none(),
            "Noto Sans has no Cuneiform"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
