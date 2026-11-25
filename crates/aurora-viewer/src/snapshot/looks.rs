//! The photo's look (issue #969), in three families that the interface
//! shows as tabs: the Legacy Viewer's photo filters (LLImageFiltersManager,
//! the Filter list of LLFloaterSnapshot, indra/newview, originally LGPL
//! 2.1), Aurora's own looks in sections, and the user's 3D look-up tables
//! (.cube files, `luts`), with the creators' tables built into the viewer
//! above them (`lut_packs`). All are drawn on the GPU by `aurora_render::looks`
//! on the live view of the photo mode and on the photo, at a strength, with
//! a before / after line on the live view.
//!
//! Looks stack: each layer of `Settings::looks` is drawn over the ones
//! before at its own strength, and can be switched off without losing its
//! place. Aurora's looks have settings (`knobs`), kept per look.
//!
//! A look is saved by its id: the Legacy Viewer's file name (`Sepia`),
//! `aurora:<id>`, `lut:<file name>` or `pack:<pack>/<table>`.

use super::knobs;
use super::lut_packs;
use super::luts::LutLibrary;
use aurora_render::looks::{AuroraSection, LookLayer, LookOp, Lut, PhotoLook, aurora_look, aurora_looks, legacy_look, legacy_looks};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Layers of the stack at most: the renderer's own limit, which every
/// look (the same one as often as wanted) fits in (aurora-render's
/// `eight_of_the_heaviest_look_fit_in_a_frame`).
pub const MAX_LAYERS: usize = aurora_render::looks::MAX_LAYERS;

/// One look of the stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layer {
    pub id: String,
    /// In percent.
    pub strength: u8,
    /// Drawn (unticked: kept in the stack, not drawn).
    pub on: bool,
}

impl Default for Layer {
    fn default() -> Self {
        Layer {
            id: String::new(),
            strength: 100,
            on: true,
        }
    }
}

impl Layer {
    pub fn new(id: impl Into<String>) -> Layer {
        Layer {
            id: id.into(),
            ..Default::default()
        }
    }
}

/// The stack once `pointed` (Some(None): "No filter") is tried: it swaps
/// the layer `replace` (the Replace switch on), or goes on top while there
/// is room. A full stack tries nothing.
pub fn tried(stack: &[Layer], pointed: Option<Option<&str>>, replace: Option<usize>) -> Vec<Layer> {
    let mut stack = stack.to_vec();
    match pointed {
        None => {}
        Some(None) => stack.clear(),
        Some(Some(id)) => {
            choose(&mut stack, id, replace);
        }
    }
    stack
}

/// A look clicked in the catalogue: it swaps the layer `replace` (the
/// Replace switch on; the layer keeps its place and strength), or goes on
/// top. Returns the layer it is in; None when the stack is full and nothing
/// is replaced (adding never pushes another look out).
pub fn choose(stack: &mut Vec<Layer>, id: &str, replace: Option<usize>) -> Option<usize> {
    match replace.filter(|&at| at < stack.len()) {
        Some(at) => {
            stack[at].id = id.to_owned();
            stack[at].on = true;
            Some(at)
        }
        None => insert(stack, stack.len(), id),
    }
}

/// A look dropped from the catalogue at place `at` of the stack (0..=len);
/// None when the stack is full.
pub fn insert(stack: &mut Vec<Layer>, at: usize, id: &str) -> Option<usize> {
    if stack.len() >= MAX_LAYERS {
        return None;
    }
    let at = at.min(stack.len());
    stack.insert(at, Layer::new(id));
    Some(at)
}

/// Loaded looks back into the stack's rules: at most `MAX_LAYERS`, each at
/// most 100 %.
pub fn sanitize(stack: &mut Vec<Layer>) {
    stack.truncate(MAX_LAYERS);
    for layer in stack {
        layer.strength = layer.strength.min(100);
    }
}

pub const AURORA: &str = "aurora:";
pub const LUT: &str = "lut:";
pub const PACK: &str = "pack:";

/// The tabs of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Legacy,
    Aurora,
    Luts,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Legacy, Tab::Aurora, Tab::Luts];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Legacy => aurora_i18n::tr!("photo-looks-legacy"),
            Tab::Aurora => aurora_i18n::tr!("photo-looks-aurora"),
            Tab::Luts => aurora_i18n::tr!("photo-looks-luts"),
        }
    }
}

/// The tab a look is in.
pub fn tab_of(id: &str) -> Tab {
    if id.starts_with(AURORA) {
        Tab::Aurora
    } else if id.starts_with(LUT) || id.starts_with(PACK) {
        Tab::Luts
    } else {
        Tab::Legacy
    }
}

/// The translated name of a Legacy Viewer filter (its strings.xml).
fn legacy_name(file: &str) -> Option<&'static str> {
    Some(match file {
        "Antique" => aurora_i18n::tr!("photo-look-antique"),
        "Autocontrast" => aurora_i18n::tr!("photo-look-autocontrast"),
        "Badtrip" => aurora_i18n::tr!("photo-look-badtrip"),
        "BlackAndWhite" => aurora_i18n::tr!("photo-look-black-and-white"),
        "Blownhighlights" => aurora_i18n::tr!("photo-look-blown-highlights"),
        "Brighten" => aurora_i18n::tr!("photo-look-brighten"),
        "Cartoon" => aurora_i18n::tr!("photo-look-cartoon"),
        "Colors1970" => aurora_i18n::tr!("photo-look-colors-1970"),
        "Darken" => aurora_i18n::tr!("photo-look-darken"),
        "Edges" => aurora_i18n::tr!("photo-look-edges"),
        "Focus" => aurora_i18n::tr!("photo-look-focus"),
        "Heatwave" => aurora_i18n::tr!("photo-look-heatwave"),
        "Intense" => aurora_i18n::tr!("photo-look-intense"),
        "Julesverne" => aurora_i18n::tr!("photo-look-jules-verne"),
        "LensFlare" => aurora_i18n::tr!("photo-look-lens-flare"),
        "Lightleak" => aurora_i18n::tr!("photo-look-light-leak"),
        "Linearize" => aurora_i18n::tr!("photo-look-linearize"),
        "Miniature" => aurora_i18n::tr!("photo-look-miniature"),
        "Negative" => aurora_i18n::tr!("photo-look-negative"),
        "Newspaper" => aurora_i18n::tr!("photo-look-newspaper"),
        "Overcast" => aurora_i18n::tr!("photo-look-overcast"),
        "Posterize" => aurora_i18n::tr!("photo-look-posterize"),
        "Rotatecolors180" => aurora_i18n::tr!("photo-look-color-shift"),
        "Sepia" => aurora_i18n::tr!("photo-look-sepia"),
        "Sharpen" => aurora_i18n::tr!("photo-look-sharpen"),
        "Softfocus" => aurora_i18n::tr!("photo-look-soft-focus"),
        "Spotlight" => aurora_i18n::tr!("photo-look-spotlight"),
        "Thematrix" => aurora_i18n::tr!("photo-look-the-matrix"),
        "Toycamera" => aurora_i18n::tr!("photo-look-toy-camera"),
        "Video" => aurora_i18n::tr!("photo-look-video"),
        _ => return None,
    })
}

/// The name and the description of one of Aurora's looks.
fn aurora_text(id: &str) -> Option<(&'static str, &'static str)> {
    use aurora_i18n::tr;
    Some(match id {
        "fine-grain" => (tr!("photo-look-a-fine-grain"), tr!("photo-look-a-fine-grain-tip")),
        "silver-grain" => (tr!("photo-look-a-silver-grain"), tr!("photo-look-a-silver-grain-tip")),
        "teal-orange" => (tr!("photo-look-a-teal-orange"), tr!("photo-look-a-teal-orange-tip")),
        "bleach-bypass" => (tr!("photo-look-a-bleach-bypass"), tr!("photo-look-a-bleach-bypass-tip")),
        "cross-process" => (tr!("photo-look-a-cross-process"), tr!("photo-look-a-cross-process-tip")),
        "instant" => (tr!("photo-look-a-instant"), tr!("photo-look-a-instant-tip")),
        "chrome" => (tr!("photo-look-a-chrome"), tr!("photo-look-a-chrome-tip")),
        "halation" => (tr!("photo-look-a-halation"), tr!("photo-look-a-halation-tip")),
        "cyanotype" => (tr!("photo-look-a-cyanotype"), tr!("photo-look-a-cyanotype-tip")),
        "infrared" => (tr!("photo-look-a-infrared"), tr!("photo-look-a-infrared-tip")),
        "vivid" => (tr!("photo-look-a-vivid"), tr!("photo-look-a-vivid-tip")),
        "faded" => (tr!("photo-look-a-faded"), tr!("photo-look-a-faded-tip")),
        "keep-red" => (tr!("photo-look-a-keep-red"), tr!("photo-look-a-keep-tip")),
        "keep-yellow" => (tr!("photo-look-a-keep-yellow"), tr!("photo-look-a-keep-tip")),
        "keep-green" => (tr!("photo-look-a-keep-green"), tr!("photo-look-a-keep-tip")),
        "keep-blue" => (tr!("photo-look-a-keep-blue"), tr!("photo-look-a-keep-tip")),
        "duotone" => (tr!("photo-look-a-duotone"), tr!("photo-look-a-duotone-tip")),
        "sunset" => (tr!("photo-look-a-sunset"), tr!("photo-look-a-sunset-tip")),
        "night-vision" => (tr!("photo-look-a-night-vision"), tr!("photo-look-a-night-vision-tip")),
        "chromatic" => (tr!("photo-look-a-chromatic"), tr!("photo-look-a-chromatic-tip")),
        "fisheye" => (tr!("photo-look-a-fisheye"), tr!("photo-look-a-fisheye-tip")),
        "old-lens" => (tr!("photo-look-a-old-lens"), tr!("photo-look-a-old-lens-tip")),
        "scope" => (tr!("photo-look-a-scope"), tr!("photo-look-a-bars-tip")),
        "widescreen" => (tr!("photo-look-a-widescreen"), tr!("photo-look-a-bars-tip")),
        "dreamy" => (tr!("photo-look-a-dreamy"), tr!("photo-look-a-dreamy-tip")),
        "pixel" => (tr!("photo-look-a-pixel"), tr!("photo-look-a-pixel-tip")),
        "vhs" => (tr!("photo-look-a-vhs"), tr!("photo-look-a-vhs-tip")),
        "glitch" => (tr!("photo-look-a-glitch"), tr!("photo-look-a-glitch-tip")),
        "oil-paint" => (tr!("photo-look-a-oil-paint"), tr!("photo-look-a-oil-paint-tip")),
        "sketch" => (tr!("photo-look-a-sketch"), tr!("photo-look-a-sketch-tip")),
        "clarity" => (tr!("photo-look-a-clarity"), tr!("photo-look-a-clarity-tip")),
        "haze" => (tr!("photo-look-a-haze"), tr!("photo-look-a-haze-tip")),
        "golden-haze" => (tr!("photo-look-a-golden-haze"), tr!("photo-look-a-haze-tip")),
        "miniature-3d" => (tr!("photo-look-a-miniature-3d"), tr!("photo-look-a-miniature-3d-tip")),
        "background-blur" => (tr!("photo-look-a-background-blur"), tr!("photo-look-a-background-blur-tip")),
        "subject" => (tr!("photo-look-a-subject"), tr!("photo-look-a-subject-tip")),
        _ => return None,
    })
}

pub fn section_name(section: AuroraSection) -> &'static str {
    match section {
        AuroraSection::Film => aurora_i18n::tr!("photo-looks-film"),
        AuroraSection::Color => aurora_i18n::tr!("photo-looks-color"),
        AuroraSection::Lens => aurora_i18n::tr!("photo-looks-lens"),
        AuroraSection::Creative => aurora_i18n::tr!("photo-looks-creative"),
        AuroraSection::Depth => aurora_i18n::tr!("photo-looks-depth"),
    }
}

/// The name of a look: translated, or a table's file name.
pub fn name(id: &str) -> Option<Cow<'static, str>> {
    if let Some(aurora) = id.strip_prefix(AURORA) {
        aurora_text(aurora).map(|(name, _)| Cow::Borrowed(name))
    } else if let Some(lut) = id.strip_prefix(LUT) {
        Some(Cow::Owned(lut_display(lut).to_owned()))
    } else if let Some(packed) = id.strip_prefix(PACK) {
        lut_packs::find(packed).map(|(_, t)| Cow::Borrowed(t.name.as_str()))
    } else {
        legacy_name(id).map(Cow::Borrowed)
    }
}

/// A table's name without its extension.
pub fn lut_display(file: &str) -> &str {
    file.strip_suffix(".cube").or_else(|| file.strip_suffix(".CUBE")).unwrap_or(file)
}

/// The name of the chosen look, "No filter" without one or with an unknown
/// one.
pub fn label(id: Option<&str>) -> Cow<'static, str> {
    id.and_then(name).unwrap_or(Cow::Borrowed(aurora_i18n::tr!("photo-look-none")))
}

/// What one of Aurora's looks does (its tile's tooltip); None for the
/// others, which have no description.
pub fn tip(id: &str) -> Option<&'static str> {
    id.strip_prefix(AURORA).and_then(aurora_text).map(|(_, tip)| tip)
}

/// The key a name sorts and is searched by: letters without their
/// accents, any case (the Legacy Viewer's map sorts the bytes, which puts É
/// after Z).
pub fn fold(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'ç' => 'c',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            other => other,
        })
        .collect()
}

/// One entry of a list: its id, name and description.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: Cow<'static, str>,
    pub tip: Option<&'static str>,
}

/// The Legacy Viewer's filters, in the order of their names.
pub fn legacy_list() -> Vec<Entry> {
    let mut looks: Vec<Entry> = legacy_looks()
        .iter()
        .filter_map(|(id, _)| {
            Some(Entry {
                id: (*id).to_owned(),
                name: Cow::Borrowed(legacy_name(id)?),
                tip: None,
            })
        })
        .collect();
    looks.sort_by_cached_key(|e| fold(&e.name));
    looks
}

/// Aurora's looks by section.
pub fn aurora_list() -> Vec<(AuroraSection, Vec<Entry>)> {
    let mut sections: Vec<(AuroraSection, Vec<Entry>)> = Vec::new();
    for look in aurora_looks() {
        let Some((name, tip)) = aurora_text(look.id) else { continue };
        let entry = Entry {
            id: format!("{AURORA}{}", look.id),
            name: Cow::Borrowed(name),
            tip: Some(tip),
        };
        match sections.last_mut() {
            Some((section, entries)) if *section == look.section => entries.push(entry),
            _ => sections.push((look.section, vec![entry])),
        }
    }
    sections
}

/// The user's tables.
pub fn lut_list(names: &[String]) -> Vec<Entry> {
    names
        .iter()
        .map(|file| Entry {
            id: format!("{LUT}{file}"),
            name: Cow::Owned(lut_display(file).to_owned()),
            tip: None,
        })
        .collect()
}

/// The creators' packs: each one's creator and tables.
pub fn pack_list() -> Vec<(&'static str, Vec<Entry>)> {
    lut_packs::packs()
        .iter()
        .map(|pack| {
            let entries = pack
                .tables
                .iter()
                .map(|t| Entry {
                    id: lut_packs::look_id(pack, t),
                    name: Cow::Borrowed(t.name.as_str()),
                    tip: None,
                })
                .collect();
            (pack.creator, entries)
        })
        .collect()
}

/// The entries of every tab whose name or description holds `query`.
pub fn search(query: &str, lut_names: &[String]) -> Vec<Entry> {
    let query = fold(query.trim());
    legacy_list()
        .into_iter()
        .chain(aurora_list().into_iter().flat_map(|(_, entries)| entries))
        .chain(pack_list().into_iter().flat_map(|(_, entries)| entries))
        .chain(lut_list(lut_names))
        .filter(|e| fold(&e.name).contains(&query) || e.tip.is_some_and(|t| fold(t).contains(&query)))
        .collect()
}

/// The starred looks of `entries` first, in the list's own order, then the
/// others (Aurora's own: the Legacy Viewer's filter list has no favourites).
pub fn favorites_first(entries: Vec<Entry>, favorites: &[String]) -> Vec<Entry> {
    let (mut first, rest): (Vec<Entry>, Vec<Entry>) = entries.into_iter().partition(|e| favorites.contains(&e.id));
    first.extend(rest);
    first
}

/// The starred looks, by family in the catalogue's order (Legacy, Aurora,
/// the LUTs); a star whose look is gone (a table deleted) is left out.
pub fn favorite_list(favorites: &[String], lut_names: &[String]) -> Vec<(Tab, Vec<Entry>)> {
    let starred = |entries: Vec<Entry>| -> Vec<Entry> { entries.into_iter().filter(|e| favorites.contains(&e.id)).collect() };
    let families = [
        (Tab::Legacy, starred(legacy_list())),
        (Tab::Aurora, starred(aurora_list().into_iter().flat_map(|(_, e)| e).collect())),
        (
            Tab::Luts,
            starred(pack_list().into_iter().flat_map(|(_, e)| e).chain(lut_list(lut_names)).collect()),
        ),
    ];
    families.into_iter().filter(|(_, e)| !e.is_empty()).collect()
}

/// Stars a look, or takes its star away.
pub fn toggle_favorite(favorites: &mut Vec<String>, id: &str) {
    match favorites.iter().position(|f| f == id) {
        Some(at) => {
            favorites.remove(at);
        }
        None => favorites.push(id.to_owned()),
    }
}

/// Whether a look can be applied to the faces of a 360° snapshot one by one
/// (`LookOp::is_pointwise`): its operations depend on a pixel's colour alone.
/// A table of the user's is a colour operation. An unknown look is not.
pub fn is_pointwise(id: &str) -> bool {
    if id.starts_with(LUT) {
        return true;
    }
    if let Some(aurora) = id.strip_prefix(AURORA) {
        return aurora_look(aurora).is_some_and(|look| look.ops.iter().all(LookOp::is_pointwise));
    }
    legacy_look(id).is_some_and(|ops| ops.iter().all(LookOp::is_pointwise))
}

/// What a look draws: its operations, and its table.
#[derive(Clone)]
pub struct Resolved {
    pub ops: Arc<[LookOp]>,
    pub lut: Option<Arc<Lut>>,
}

/// The settings of one of Aurora's looks by id.
pub type Settings = BTreeMap<String, Vec<f32>>;

/// The operations of a look with its settings; a table not read yet gives
/// None (it is being read, on a worker).
pub fn resolve(id: &str, settings: &Settings, luts: &mut LutLibrary) -> Option<Resolved> {
    if let Some(aurora) = id.strip_prefix(AURORA) {
        let look = aurora_look(aurora)?;
        let ops = match settings.get(id) {
            Some(values) => knobs::apply(knobs::knobs(aurora), &look.ops, values).into(),
            None => look.ops.clone(),
        };
        return Some(Resolved { ops, lut: None });
    }
    if let Some(file) = id.strip_prefix(LUT) {
        let lut = luts.table(file)?;
        return Some(Resolved {
            ops: Arc::from([LookOp::Lut]),
            lut: Some(lut),
        });
    }
    if let Some(packed) = id.strip_prefix(PACK) {
        return Some(Resolved {
            ops: Arc::from([LookOp::Lut]),
            lut: Some(lut_packs::table(packed)?),
        });
    }
    Some(Resolved {
        ops: legacy_look(id)?,
        lut: None,
    })
}

/// Where the photo is in a frame drawn and how big it is there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The photo's top left corner, in pixels of the frame.
    pub origin: [f32; 2],
    /// The photo's size, in its own pixels.
    pub photo: (u32, u32),
    /// Pixels of the frame per pixel of the photo.
    pub scale: f32,
}

/// The layers switched on, resolved, with their strength (a table not
/// read yet is left out this frame).
pub fn resolve_stack(stack: &[Layer], settings: &Settings, luts: &mut LutLibrary) -> Vec<(Resolved, u8)> {
    stack
        .iter()
        .filter(|l| l.on)
        .filter_map(|l| Some((resolve(&l.id, settings, luts)?, l.strength)))
        .collect()
}

/// The looks of a frame; `focus`: the focus distance of the depth
/// operations (the photo's depth of field), None for the scene at the
/// middle of the photo. None without a layer.
pub fn frame_look(layers: &[(Resolved, u8)], place: Placement, compare: Option<f32>, hold: bool, focus: Option<f32>) -> Option<PhotoLook> {
    if layers.is_empty() {
        return None;
    }
    Some(PhotoLook {
        layers: layers
            .iter()
            .map(|(look, strength)| LookLayer {
                ops: look.ops.clone(),
                intensity: f32::from((*strength).min(100)) / 100.0,
                lut: look.lut.clone(),
            })
            .collect(),
        compare: compare.map(|c| c.clamp(0.0, 1.0)),
        image: [place.origin[0], place.origin[1], place.photo.0 as f32, place.photo.1 as f32],
        scale: place.scale,
        hold,
        focus,
    })
}

/// The world's look (Preferences › Graphics › Image › World look, and its
/// window `ui/world_look.rs`): a stack of its own, with its own settings,
/// drawn on the 3D view, under the interface. Aurora's own: the Legacy
/// Viewer has looks only in its snapshot window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldLook {
    /// Drawn (off: the stack is kept for later).
    pub on: bool,
    pub looks: Vec<Layer>,
    /// The settings of its Aurora looks by id, apart from the photos'.
    pub look_settings: Settings,
}

/// The world look's stack for this frame. The photo window and the
/// Director (`photo_looks_shown`) draw their own stack, the one of the
/// photos and films they take, so the world's is not drawn under it. Off,
/// nothing is drawn unless a look is pointed at in its window: the stack is
/// then tried with it (`tried`), as when it is on, so that the list can be
/// browsed before the look is switched on (the human's choice, 2026-11-21:
/// the same preview on or off). `replace`: the card the look pointed at
/// swaps (`LooksUi::replace_target`), else it is tried at the end; a full
/// stack with Replace off tries nothing new.
pub fn world_stack(look: &WorldLook, pointed: Option<Option<&str>>, replace: Option<usize>, photo_looks_shown: bool) -> Vec<Layer> {
    if photo_looks_shown || (!look.on && pointed.is_none()) {
        return Vec::new();
    }
    tried(&look.looks, pointed, replace)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The photo window's 360° format keeps the colour looks and greys the
    /// others (the classification itself is tested for every look in
    /// aurora-render's looks tests).
    #[test]
    fn colour_looks_are_kept_for_a_360_snapshot_and_the_others_are_not() {
        assert!(is_pointwise("Sepia") && is_pointwise("BlackAndWhite") && is_pointwise("Intense"));
        assert!(is_pointwise(&format!("{AURORA}keep-red")));
        assert!(is_pointwise(&format!("{LUT}warm.cube")), "a table is a colour operation");
        assert!(!is_pointwise("Spotlight") && !is_pointwise("Sharpen") && !is_pointwise("Posterize"));
        assert!(!is_pointwise(&format!("{AURORA}fine-grain")) && !is_pointwise(&format!("{AURORA}fisheye")));
        assert!(!is_pointwise("No such look"));
        let all = legacy_list().into_iter().chain(aurora_list().into_iter().flat_map(|(_, e)| e));
        let kept = all.filter(|e| is_pointwise(&e.id)).count();
        assert!(kept > 5 && kept < 66, "{kept} of the 66 looks are colour looks");
    }

    #[test]
    fn every_look_has_a_name() {
        // a missing message gives its key back (the French catalogue has
        // every English key, aurora-i18n's tests)
        let all = legacy_list().into_iter().chain(aurora_list().into_iter().flat_map(|(_, e)| e));
        let mut count = 0;
        for entry in all {
            assert!(!entry.name.starts_with("photo-look"), "{}: {}", entry.id, entry.name);
            assert!(entry.tip.is_none_or(|t| !t.starts_with("photo-look")), "{}", entry.id);
            count += 1;
        }
        assert_eq!(count, 30 + 36);
        assert_eq!(aurora_list().len(), 5);
    }

    #[test]
    fn ids_name_their_tab() {
        assert_eq!(tab_of("Sepia"), Tab::Legacy);
        assert_eq!(tab_of("aurora:vhs"), Tab::Aurora);
        assert_eq!(tab_of("lut:Warm.cube"), Tab::Luts);
        assert_eq!(name("lut:Warm.cube").as_deref(), Some("Warm"));
        assert_eq!(tab_of("pack:czar-fenris/Noir"), Tab::Luts);
        assert_eq!(name("pack:czar-fenris/Noir").as_deref(), Some("Noir"));
        assert_eq!(name("pack:czar-fenris/Nothing"), None);
        assert_eq!(label(Some("Nothing")), aurora_i18n::tr!("photo-look-none"));
    }

    #[test]
    fn names_sort_and_search_without_their_accents() {
        assert!(fold("Éclaircir") < fold("Fuite"));
        assert!(fold("Négatif") > fold("Matrix"));
        let found = search("SEPIA", &["Sépia doux.cube".to_owned()]);
        assert!(found.iter().any(|e| e.id == "lut:Sépia doux.cube"), "{found:?}");
        // a Legacy filter is found by its name in the interface's language
        // (Sepia, Sépia, セピア)
        let found = search(&aurora_i18n::tr!("photo-look-sepia").to_uppercase(), &[]);
        assert!(found.iter().any(|e| e.id == "Sepia"), "{found:?}");
        // the creators' tables are found too
        let found = search("winter", &[]);
        assert!(found.iter().any(|e| e.id == "pack:czar-fenris/Winter Fog"), "{found:?}");
    }

    #[test]
    fn a_pack_table_draws_as_a_lut() {
        let mut luts = LutLibrary::new(std::env::temp_dir().join("aurora-no-luts"));
        let look = resolve("pack:czar-fenris/Film look", &Settings::new(), &mut luts).expect("resolves");
        assert_eq!(look.lut.map(|l| l.size), Some(33));
        assert!(resolve("pack:czar-fenris/Nothing", &Settings::new(), &mut luts).is_none());
        let (creator, entries) = &pack_list()[0];
        assert_eq!(*creator, "Czar Fenris");
        assert!(entries.iter().all(|e| name(&e.id).as_deref() == Some(e.name.as_ref())));
    }

    #[test]
    fn a_frame_look_takes_its_strength_and_place() {
        let place = Placement {
            origin: [10.0, -4.0],
            photo: (3840, 2160),
            scale: 0.5,
        };
        let mut luts = LutLibrary::new(std::env::temp_dir().join("aurora-no-luts"));
        let settings = Settings::new();
        let mut stack = vec![Layer::new("Sepia"), Layer::new("aurora:vhs"), Layer::new("Nothing")];
        stack[0].strength = 140;
        stack[1].on = false;
        let layers = resolve_stack(&stack, &settings, &mut luts);
        assert_eq!(layers.len(), 1);
        let look = frame_look(&layers, place, Some(1.4), true, Some(3.0)).expect("a look");
        assert_eq!(look.layers[0].intensity, 1.0);
        assert_eq!(look.compare, Some(1.0));
        assert_eq!(look.image, [10.0, -4.0, 3840.0, 2160.0]);
        assert!(look.hold);
        assert_eq!(look.focus, Some(3.0));
        assert!(frame_look(&[], place, None, false, None).is_none());
        assert!(resolve("aurora:nothing", &settings, &mut luts).is_none());
    }

    #[test]
    fn settings_change_an_aurora_look() {
        let mut luts = LutLibrary::new(std::env::temp_dir().join("aurora-no-luts"));
        let mut settings = Settings::new();
        let plain = resolve("aurora:glitch", &settings, &mut luts).expect("glitch");
        settings.insert("aurora:glitch".into(), vec![0.05, 0.02, 9.0]);
        let set = resolve("aurora:glitch", &settings, &mut luts).expect("glitch");
        assert_ne!(plain.ops, set.ops);
        assert_eq!(set.ops[0].param(0), Some(0.05));
    }

    #[test]
    fn clicks_add_unless_replace_is_on() {
        let mut stack = Vec::new();
        // a click adds; with Replace on over an empty stack too
        assert_eq!(choose(&mut stack, "Sepia", None), Some(0));
        assert_eq!(choose(&mut stack, "aurora:vhs", None), Some(1));
        stack[0].strength = 40;
        stack[0].on = false;
        // Replace on: the chosen layer swaps, keeping its place and strength
        assert_eq!(choose(&mut stack, "Negative", Some(0)), Some(0));
        assert_eq!((stack[0].strength, stack[0].on), (40, true));
        // a chosen layer gone (taken out meanwhile): the click adds
        assert_eq!(choose(&mut stack, "Antique", Some(9)), Some(2));
        let ids: Vec<&str> = stack.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["Negative", "aurora:vhs", "Antique"]);
        // trying leaves the stack as it is
        assert_eq!(tried(&stack, Some(Some("Sepia")), None).len(), 4);
        assert_eq!(tried(&stack, Some(Some("Sepia")), Some(1))[1].id, "Sepia");
        assert_eq!(stack.len(), 3);
        assert!(tried(&stack, Some(None), None).is_empty());
    }

    #[test]
    fn a_full_stack_adds_nothing_but_still_replaces() {
        let mut stack = Vec::new();
        for _ in 0..MAX_LAYERS {
            assert!(choose(&mut stack, "aurora:pixel", None).is_some());
        }
        assert_eq!(MAX_LAYERS, 8, "the renderer's limit");
        // full: neither a click nor a drop pushes a look out
        assert_eq!(choose(&mut stack, "Sepia", None), None);
        assert_eq!(insert(&mut stack, 0, "Sepia"), None);
        assert_eq!(tried(&stack, Some(Some("Sepia")), None), stack, "nothing to try");
        assert!(stack.iter().all(|l| l.id == "aurora:pixel"));
        // Replace still swaps one
        assert_eq!(choose(&mut stack, "Sepia", Some(MAX_LAYERS - 1)), Some(MAX_LAYERS - 1));
        assert_eq!(stack.len(), MAX_LAYERS);
    }

    #[test]
    fn the_world_look_is_drawn_when_on_and_gives_way_to_the_photo_looks() {
        let mut world = WorldLook {
            on: true,
            looks: vec![Layer::new("pack:czar-fenris/Noir")],
            ..Default::default()
        };
        let ids = |stack: Vec<Layer>| stack.into_iter().map(|l| l.id).collect::<Vec<_>>();
        assert_eq!(ids(world_stack(&world, None, Some(0), false)), ["pack:czar-fenris/Noir"]);
        // the photo window or the Director draws its own stack
        assert!(world_stack(&world, None, Some(0), true).is_empty());
        assert!(world_stack(&world, Some(Some("Sepia")), Some(0), true).is_empty());
        // a look pointed at in its window is tried, the stack unchanged
        assert_eq!(
            ids(world_stack(&world, Some(Some("Sepia")), None, false)),
            ["pack:czar-fenris/Noir", "Sepia"]
        );
        assert!(world_stack(&world, Some(None), Some(0), false).is_empty());
        assert_eq!(world.looks.len(), 1);
        // off: nothing, unless a look is pointed at
        world.on = false;
        assert!(world_stack(&world, None, Some(0), false).is_empty());
        assert_eq!(ids(world_stack(&world, Some(Some("Sepia")), Some(0), false)), ["Sepia"]);
        // Replace off (the default): the switched-off stack with the look
        // at its end, as when it is on
        assert_eq!(
            ids(world_stack(&world, Some(Some("Sepia")), None, false)),
            ["pack:czar-fenris/Noir", "Sepia"]
        );
        let full = WorldLook {
            looks: (0..MAX_LAYERS).map(|_| Layer::new("Negative")).collect(),
            ..Default::default()
        };
        assert_eq!(
            world_stack(&full, Some(Some("Sepia")), None, false),
            full.looks,
            "a full stack with Replace off tries nothing new"
        );
        // a setting saved before the world look keeps working: empty and off
        let read: WorldLook = serde_json::from_str("{}").expect("reads");
        assert_eq!(read, WorldLook::default());
        assert!(!read.on && read.looks.is_empty());
    }

    #[test]
    fn a_drop_takes_its_place_in_the_stack() {
        let mut stack = vec![Layer::new("Sepia"), Layer::new("Negative")];
        assert_eq!(insert(&mut stack, 1, "aurora:vhs"), Some(1));
        assert_eq!(insert(&mut stack, 99, "Antique"), Some(3), "past the end: last");
        let ids: Vec<&str> = stack.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["Sepia", "aurora:vhs", "Negative", "Antique"]);
    }

    #[test]
    fn loaded_looks_keep_to_the_stacks_rules() {
        let mut stack: Vec<Layer> = (0..12)
            .map(|_| Layer {
                id: "Sepia".into(),
                strength: 250,
                on: true,
            })
            .collect();
        sanitize(&mut stack);
        assert_eq!(stack.len(), MAX_LAYERS);
        assert!(stack.iter().all(|l| l.strength == 100));
    }

    #[test]
    fn starred_looks_come_first_in_their_list() {
        let favorites = vec!["aurora:vhs".to_owned(), "aurora:fine-grain".to_owned()];
        let creative: Vec<Entry> = aurora_list().into_iter().flat_map(|(_, e)| e).collect();
        let sorted = favorites_first(creative.clone(), &favorites);
        assert_eq!(sorted.len(), creative.len(), "nothing lost");
        // the list's own order among the starred, then among the others
        assert_eq!(sorted[0].id, "aurora:fine-grain");
        assert_eq!(sorted[1].id, "aurora:vhs");
        assert_eq!(
            sorted[2].id,
            creative.iter().find(|e| !favorites.contains(&e.id)).expect("others").id
        );
    }

    #[test]
    fn the_star_list_groups_the_starred_by_family_and_skips_what_is_gone() {
        let mut favorites = Vec::new();
        for id in ["lut:mine.cube", "Sepia", "aurora:vhs", "lut:deleted.cube"] {
            toggle_favorite(&mut favorites, id);
        }
        let luts = vec!["mine.cube".to_owned()];
        let list = favorite_list(&favorites, &luts);
        let tabs: Vec<Tab> = list.iter().map(|(t, _)| *t).collect();
        assert_eq!(tabs, [Tab::Legacy, Tab::Aurora, Tab::Luts]);
        assert_eq!(list[2].1.len(), 1, "the deleted table is not listed");
        // a second star takes it away
        toggle_favorite(&mut favorites, "Sepia");
        assert!(!favorites.contains(&"Sepia".to_owned()));
        assert_eq!(favorite_list(&favorites, &luts).len(), 2);
        assert!(favorite_list(&[], &luts).is_empty());
    }
}
