//! Folder icon colors: a color the user gives the icon of an inventory
//! folder, to find it at a glance. Kept in this viewer's inventory
//! preferences by folder id, never sent to the grid: nobody else sees it.
//! An Aurora addition (the Legacy Viewer has no equivalent), on by default:
//! nothing changes until a folder is given a color. The inventory
//! preferences can turn it off (menu entry and tints hidden, colors kept).

use super::*;
use crate::ui::widgets::{Floater, color::Rgba, color_button};
use egui::{Color32, Sense, Stroke};

/// The palette's colors, readable on the dark and the light theme alike.
pub const PRESETS: [[u8; 3]; 12] = [
    [0xE5, 0x48, 0x4D], // red
    [0xF7, 0x6B, 0x15], // orange
    [0xFF, 0xB2, 0x24], // amber
    [0x8D, 0xB6, 0x00], // lime
    [0x30, 0xA4, 0x6C], // green
    [0x12, 0xA5, 0x94], // teal
    [0x05, 0xA2, 0xC2], // cyan
    [0x3E, 0x63, 0xDD], // blue
    [0x8E, 0x4E, 0xC6], // purple
    [0xD6, 0x40, 0x9F], // pink
    [0xAD, 0x7F, 0x58], // brown
    [0x8B, 0x8D, 0x98], // gray
];
const SWATCH: f32 = 22.0;
const PER_ROW: usize = 6;

/// The tint of a folder's icon, when the user colored it (and the colors are
/// turned on).
pub fn tint(prefs: &InventoryPreferences, id: Uuid) -> Option<Color32> {
    if !prefs.folder_colors_enabled {
        return None;
    }
    prefs.folder_colors.get(&id).map(|&[r, g, b]| Color32::from_rgb(r, g, b))
}

/// Gives every folder of `ids` this color, or its default one with `None`.
pub fn set(prefs: &mut InventoryPreferences, ids: &[Uuid], color: Option<[u8; 3]>) {
    for id in ids {
        match color {
            Some(color) => {
                prefs.folder_colors.insert(*id, color);
            }
            None => {
                prefs.folder_colors.remove(id);
            }
        }
    }
}

/// New folders take their parent's color (the switch on): `created` are the
/// folders a change just made, under the ids the server gave them and parents
/// first, so a tree made at once takes the color down. A folder already
/// colored keeps its own; folders merely loaded never pass here. Whether a
/// color was given (the preferences then need saving).
pub fn inherit(prefs: &mut InventoryPreferences, created: &[aurora_net::inventory::InvFolder]) -> bool {
    if !prefs.folder_colors_enabled || !prefs.folder_colors_inherit {
        return false;
    }
    let mut given = false;
    for folder in created {
        if prefs.folder_colors.contains_key(&folder.id) {
            continue;
        }
        if let Some(&color) = prefs.folder_colors.get(&folder.parent) {
            prefs.folder_colors.insert(folder.id, color);
            given = true;
        }
    }
    given
}

/// The color all of `ids` share, if they share one.
fn shared(prefs: &InventoryPreferences, ids: &[Uuid]) -> Option<[u8; 3]> {
    let first = *prefs.folder_colors.get(ids.first()?)?;
    ids.iter().all(|id| prefs.folder_colors.get(id) == Some(&first)).then_some(first)
}

/// The palette window. It colors the folders selected in its inventory
/// window, whichever they are now: click a folder, click a color, click the
/// next one. Its line under the title names them.
pub(super) struct Picker {
    ids: Vec<Uuid>,
    /// The custom swatch, seeded with the folders' color.
    custom: Rgba,
    /// Opened from a menu: comes to the front once shown. The window keeps
    /// its id, so egui reopens it at its old depth, under an inventory
    /// clicked since (the expanded window covers it).
    raise: bool,
    /// "Also color its subfolders": off at every opening, so a click never
    /// recolors a whole tree by surprise.
    subfolders: bool,
    /// The folders below `ids`, kept for the inventory generation they were
    /// gathered at (a tree under "My Inventory" can hold thousands).
    below: Option<(Vec<Uuid>, u64, Vec<Uuid>)>,
}

impl Picker {
    pub(super) fn new(prefs: &InventoryPreferences, ids: Vec<Uuid>) -> Self {
        let [r, g, b] = shared(prefs, &ids).unwrap_or(PRESETS[7]);
        Self {
            ids,
            custom: [r, g, b, 255],
            raise: true,
            subfolders: false,
            below: None,
        }
    }

    /// The folders below the targets, gathered again when they or the
    /// inventory changed.
    fn below(&mut self, inv: &Inventory) -> &[Uuid] {
        let fresh = matches!(&self.below, Some((ids, generation, _)) if *ids == self.ids && *generation == inv.generation);
        if !fresh {
            self.below = Some((self.ids.clone(), inv.generation, descendants(inv, &self.ids)));
        }
        self.below.as_ref().map_or(&[], |(_, _, below)| below.as_slice())
    }

    /// What a click colors: the targets, and the folders below them when the
    /// toggle is on.
    fn colored_by_a_click(&mut self, inv: &Inventory) -> Vec<Uuid> {
        let mut ids = self.ids.clone();
        if self.subfolders {
            ids.extend_from_slice(self.below(inv));
        }
        ids
    }

    /// Follows the selection: new folders also bring their color to the
    /// custom swatch.
    fn retarget(&mut self, prefs: &InventoryPreferences, ids: Vec<Uuid>) {
        if ids != self.ids {
            if let Some([r, g, b]) = shared(prefs, &ids) {
                self.custom = [r, g, b, 255];
            }
            self.ids = ids;
        }
    }
}

/// The folders of a selection, the ones a color applies to, in a steady order.
pub(super) fn folders<'a>(inv: &Inventory, ids: impl IntoIterator<Item = &'a Uuid>) -> Vec<Uuid> {
    let mut folders: Vec<Uuid> = ids.into_iter().copied().filter(|id| inv.folders.contains_key(id)).collect();
    folders.sort_unstable();
    folders
}

/// Every folder below `roots` (not the roots themselves), each once, even if
/// a corrupt tree loops or a root lies under another one.
fn descendants(inv: &Inventory, roots: &[Uuid]) -> Vec<Uuid> {
    let mut seen: HashSet<Uuid> = roots.iter().copied().collect();
    let mut below = Vec::new();
    let mut next: Vec<Uuid> = roots.to_vec();
    while let Some(id) = next.pop() {
        let Some(folder) = inv.folders.get(&id) else { continue };
        for &child in &folder.children {
            if inv.folders.contains_key(&child) && seen.insert(child) {
                below.push(child);
                next.push(child);
            }
        }
    }
    below.sort_unstable();
    below
}

/// The palette window: a click on a swatch colors the selected folders at
/// once; the custom swatch opens the color picker; "Restore default" removes
/// the color.
pub(super) fn show(ctx: &egui::Context, p: &Palette, inv: &Inventory, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let ui_id = st.ui_id;
    let selected = folders(inv, &st.selection);
    let Some(picker) = &mut st.folder_color else {
        return;
    };
    picker.retarget(prefs, selected);
    let target = match picker.ids.as_slice() {
        [] => None,
        [one] => Some(super::folder_name(inv, *one)),
        many => Some(aurora_i18n::tr!("inventory-folder-color-many", count = many.len()).to_owned()),
    };
    let window_id = format!("inventory_folder_color_{ui_id}");
    let size = Vec2::new(220.0, 150.0);
    let mut open = true;
    Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-folder-color-title"),
        ctx.content_rect().center() - size / 2.0,
        size,
    )
    .auto_height()
    .show(ctx, p, &mut open, |ui| {
        let current = shared(prefs, &picker.ids);
        ui.spacing_mut().item_spacing = Vec2::splat(6.0);
        // what a click colors, so a selection changed since is never a surprise
        match &target {
            Some(name) => ui.label(RichText::new(name).strong().color(p.ink)),
            None => ui.label(RichText::new(aurora_i18n::tr!("inventory-folder-color-none")).color(p.muted)),
        };
        ui.add_enabled_ui(target.is_some(), |ui| {
            // off by default; greyed when the folders have no subfolder
            let count = picker.below(inv).len();
            ui.add_enabled_ui(count > 0, |ui| {
                let label = aurora_i18n::tr!("inventory-folder-color-subfolders", count = count);
                crate::ui::widgets::checkbox(ui, p, &mut picker.subfolders, label);
            });
            palette(ui, p, ui_id, inv, picker, prefs, current);
        });
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-folder-color-private"))
                .size(11.0)
                .color(p.muted),
        );
    });
    if std::mem::take(&mut picker.raise) {
        crate::ui::widgets::bring_floater_front(ctx, &window_id);
    }
    if !open || !prefs.folder_colors_enabled {
        st.folder_color = None;
    }
}

/// The swatches, the custom color and "Restore default" of the picker's
/// folders (and of the folders below them, the toggle on).
fn palette(
    ui: &mut egui::Ui,
    p: &Palette,
    ui_id: Uuid,
    inv: &Inventory,
    picker: &mut Picker,
    prefs: &mut InventoryPreferences,
    current: Option<[u8; 3]>,
) {
    for row in PRESETS.chunks(PER_ROW) {
        ui.horizontal(|ui| {
            for &color in row {
                if swatch(ui, p, color, current == Some(color)).clicked() {
                    set(prefs, &picker.colored_by_a_click(inv), Some(color));
                    let [r, g, b] = color;
                    picker.custom = [r, g, b, 255];
                }
            }
        });
    }
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if color_button(ui, p, ("inventory_folder_color_custom", ui_id), &mut picker.custom, false).changed() {
            let [r, g, b, _] = picker.custom;
            set(prefs, &picker.colored_by_a_click(inv), Some([r, g, b]));
        }
        ui.label(RichText::new(aurora_i18n::tr!("inventory-folder-color-custom")).color(p.ink));
    });
    let reach = picker.colored_by_a_click(inv);
    let colored = reach.iter().any(|id| prefs.folder_colors.contains_key(id));
    if ui
        .add_enabled(colored, egui::Button::new(aurora_i18n::tr!("inventory-folder-color-restore")))
        .clicked()
    {
        set(prefs, &reach, None);
    }
}

/// One color of the palette, outlined when the folders have it.
fn swatch(ui: &mut egui::Ui, p: &Palette, [r, g, b]: [u8; 3], chosen: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(SWATCH), Sense::click());
    let radius = ui.visuals().window_corner_radius;
    // dimmed while no folder is selected (painted colors don't fade with the ui)
    let fill = Color32::from_rgb(r, g, b);
    let fill = if ui.is_enabled() { fill } else { fill.gamma_multiply(0.3) };
    ui.painter().rect_filled(rect, radius, fill);
    let edge = if chosen {
        Some(Stroke::new(2.0, p.ink))
    } else if resp.hovered() {
        Some(Stroke::new(1.0, p.violet_light))
    } else {
        None
    };
    if let Some(edge) = edge {
        ui.painter().rect_stroke(rect, radius, edge, egui::StrokeKind::Outside);
    }
    resp.on_hover_text(crate::ui::widgets::color::format_hex([r, g, b, 255], false))
}

/// `AURORA_DEMO_INVENTORY=folder-colors|aurora-folder-colors`: colors on the
/// demo's folders, and the palette open on one of them.
pub fn demo(st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    prefs.folder_colors_enabled = true;
    for (folder, color) in [(8000, 0), (8200, 4), (8202, 7), (8203, 9), (8204, 2)] {
        set(prefs, &[Uuid::from_u128(folder)], Some(PRESETS[color]));
    }
    st.folder_color = Some(Picker::new(prefs, vec![Uuid::from_u128(8200)]));
}

/// The frames of the folder color demos (the palette follows the selection,
/// so each step selects its folder after the demo's own start-up selection):
/// - every mode: "A — Personal folder" selected at frame 600, before the
///   first capture (620);
/// - `aurora-folder-colors-behind`: the inventory widened (640) and brought
///   over the palette (700, a click on it), then "Folder color…" again on
///   "Right-click tests" (760). Capture 750 (palette covered), 820 (in front);
/// - `aurora-folder-colors-follow`: "Clothing" selected (700), then an item
///   (780). Capture 750 (the palette names Clothing, its pink outlined) and 830
///   (no folder: "Select a folder", the palette greyed);
/// - `aurora-folder-colors-subfolders`: the tree open, "Also color its
///   subfolders" ticked (700), purple picked (720). Capture 690 (unticked, its
///   count 3) and 780 (A — Personal folder and its 3 subfolders purple);
/// - `aurora-folder-colors-inherit`: "New folders take their parent's color"
///   on, a New Folder made in the green A — Personal folder (700) through the
///   menu's own change. Capture 820 (the new folder green).
///
/// The change a step makes, for the app to run as the menu's would be.
pub fn demo_frame(
    ctx: &egui::Context,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    frame: u64,
) -> Option<InvAction> {
    let mode = std::env::var("AURORA_DEMO_INVENTORY").ok()?;
    if !mode.contains("folder-colors") {
        return None;
    }
    // as a click: the selection and its focused entry (the details card's)
    let select = |st: &mut InventoryUi, id: u128| {
        st.selection.clear();
        st.selection.insert(Uuid::from_u128(id));
        st.selected = Some(Uuid::from_u128(id));
    };
    match (mode.as_str(), frame) {
        (_, 600) => select(st, 8200),
        ("aurora-folder-colors-behind", 640) => super::aurora::widen(ctx, "inventory"),
        ("aurora-folder-colors-behind", 700) => crate::ui::widgets::bring_floater_front(ctx, "inventory"),
        ("aurora-folder-colors-behind", 760) => {
            select(st, 8000);
            st.folder_color = Some(Picker::new(prefs, vec![Uuid::from_u128(8000)]));
        }
        ("aurora-folder-colors-follow", 700) => select(st, 8203),
        ("aurora-folder-colors-follow", 780) => select(st, 8100),
        ("aurora-folder-colors-subfolders", 610) => st.expand_all = Some(true),
        ("aurora-folder-colors-subfolders", 700) => {
            if let Some(picker) = &mut st.folder_color {
                picker.subfolders = true;
            }
        }
        ("aurora-folder-colors-subfolders", 720) => {
            if let Some(picker) = &mut st.folder_color {
                set(prefs, &picker.colored_by_a_click(inv), Some(PRESETS[8]));
            }
        }
        // the open tree is long: scroll to the colored ones (Body, the last)
        ("aurora-folder-colors-subfolders", 730) => st.reveal = Some(Uuid::from_u128(8204)),
        // after the start-up selection (600, every mode)
        ("aurora-folder-colors-inherit", 605) => prefs.folder_colors_inherit = true,
        ("aurora-folder-colors-inherit", 610) => st.expand_all = Some(true),
        ("aurora-folder-colors-inherit", 700) => {
            let change = rules::create_folder(Uuid::from_u128(8200), aurora_i18n::tr!("common-new-folder")).ok()?;
            return Some(InvAction::Edit(change));
        }
        // New Folder sorts after Clothing, the folder before it
        ("aurora-folder-colors-inherit", 760) => st.reveal = Some(Uuid::from_u128(8203)),
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_apply_to_every_folder_and_restore_removes_them() {
        let mut prefs = InventoryPreferences::default();
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        set(&mut prefs, &[a, b], Some(PRESETS[0]));
        assert_eq!(shared(&prefs, &[a, b]), Some(PRESETS[0]));
        set(&mut prefs, &[b], Some([1, 2, 3]));
        assert_eq!(shared(&prefs, &[a, b]), None);
        set(&mut prefs, &[a, b], None);
        assert!(prefs.folder_colors.is_empty());
    }

    #[test]
    fn the_palette_follows_the_selection_and_takes_the_new_folders_color() {
        let mut prefs = InventoryPreferences::default();
        let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        set(&mut prefs, &[b], Some(PRESETS[9]));
        let mut picker = Picker::new(&prefs, vec![a]);
        picker.custom = [1, 2, 3, 255];
        // same folders: the custom color being picked stays
        picker.retarget(&prefs, vec![a]);
        assert_eq!(picker.custom, [1, 2, 3, 255]);
        // another folder: it is the target, with its color
        picker.retarget(&prefs, vec![b]);
        assert_eq!(picker.ids, vec![b]);
        assert_eq!(picker.custom, [0xD6, 0x40, 0x9F, 255]);
        // an uncolored one keeps the last swatch; no folder empties the target
        picker.retarget(&prefs, vec![c]);
        assert_eq!(picker.custom, [0xD6, 0x40, 0x9F, 255]);
        picker.retarget(&prefs, Vec::new());
        assert!(picker.ids.is_empty());
    }

    /// The demo's "A — Personal folder" (8200) holds Attachments, Clothing
    /// and Body (8202–8204).
    fn personal_tree() -> Inventory {
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        crate::world::inventory::demo::seed_view(&mut inv, Uuid::from_u128(2), false);
        inv
    }

    #[test]
    fn subfolders_are_colored_only_with_the_toggle_on() {
        let inv = personal_tree();
        let mut prefs = InventoryPreferences::default();
        let personal = Uuid::from_u128(8200);
        let below: Vec<Uuid> = (8202..=8204).map(Uuid::from_u128).collect();
        let mut picker = Picker::new(&prefs, vec![personal]);
        assert!(!picker.subfolders, "off at every opening");
        assert_eq!(picker.below(&inv), below.as_slice());
        // off: the folder alone
        set(&mut prefs, &picker.colored_by_a_click(&inv), Some(PRESETS[0]));
        assert_eq!(prefs.folder_colors.keys().copied().collect::<Vec<_>>(), vec![personal]);
        // on: the folder and every folder below it; Restore default the same
        picker.subfolders = true;
        set(&mut prefs, &picker.colored_by_a_click(&inv), Some(PRESETS[8]));
        assert_eq!(prefs.folder_colors.len(), 4);
        assert!(below.iter().all(|id| prefs.folder_colors[id] == PRESETS[8]));
        set(&mut prefs, &picker.colored_by_a_click(&inv), None);
        assert!(prefs.folder_colors.is_empty());
    }

    #[test]
    fn a_looping_tree_gives_each_folder_once() {
        let mut inv = personal_tree();
        let personal = Uuid::from_u128(8200);
        // a corrupt tree: Body lists its own ancestor as a child
        inv.folders.get_mut(&Uuid::from_u128(8204)).expect("Body").children.push(personal);
        let below = descendants(&inv, &[personal]);
        assert_eq!(below, (8202..=8204).map(Uuid::from_u128).collect::<Vec<_>>());
        // a selected folder under another selected one is a target, not below:
        // the two selected and the two others, each colored once
        let clothing = Uuid::from_u128(8203);
        assert_eq!(
            descendants(&inv, &[personal, clothing]),
            vec![Uuid::from_u128(8202), Uuid::from_u128(8204)]
        );
        let prefs = InventoryPreferences::default();
        let mut picker = Picker::new(&prefs, vec![personal, clothing]);
        picker.subfolders = true;
        let mut reach = picker.colored_by_a_click(&inv);
        reach.sort_unstable();
        reach.dedup();
        assert_eq!(reach.len(), picker.colored_by_a_click(&inv).len(), "no folder twice");
        assert_eq!(reach.len(), 4);
    }

    fn made(id: u128, parent: u128) -> aurora_net::inventory::InvFolder {
        aurora_net::inventory::InvFolder {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            ..Default::default()
        }
    }

    #[test]
    fn new_folders_take_their_parents_color_only_with_the_switch_on() {
        let mut prefs = InventoryPreferences::default();
        assert!(!prefs.folder_colors_inherit, "off by default");
        set(&mut prefs, &[Uuid::from_u128(1)], Some(PRESETS[4]));
        let created = [made(10, 1), made(11, 10), made(12, 99)];
        // off: nothing given
        assert!(!inherit(&mut prefs, &created));
        assert_eq!(prefs.folder_colors.len(), 1);
        // on: the new folder, and the one made inside it in the same change
        // (parents first); a folder under an uncolored parent stays plain
        prefs.folder_colors_inherit = true;
        assert!(inherit(&mut prefs, &created));
        assert_eq!(prefs.folder_colors.get(&Uuid::from_u128(10)), Some(&PRESETS[4]));
        assert_eq!(prefs.folder_colors.get(&Uuid::from_u128(11)), Some(&PRESETS[4]));
        assert!(!prefs.folder_colors.contains_key(&Uuid::from_u128(12)));
        // a folder that already has a color keeps it
        set(&mut prefs, &[Uuid::from_u128(13)], Some(PRESETS[0]));
        assert!(!inherit(&mut prefs, &[made(13, 1)]));
        assert_eq!(prefs.folder_colors.get(&Uuid::from_u128(13)), Some(&PRESETS[0]));
        // the colors off: nothing given either
        prefs.folder_colors_enabled = false;
        assert!(!inherit(&mut prefs, &[made(14, 1)]));
    }

    #[test]
    fn only_colored_folders_change_and_turning_colors_off_restores_every_icon() {
        let mut prefs = InventoryPreferences::default();
        let id = Uuid::from_u128(1);
        set(&mut prefs, &[id], Some([10, 20, 30]));
        assert_eq!(tint(&prefs, id), Some(Color32::from_rgb(10, 20, 30)));
        assert_eq!(tint(&prefs, Uuid::from_u128(2)), None);
        prefs.folder_colors_enabled = false;
        assert_eq!(tint(&prefs, id), None);
    }

    /// The palette keeps its size, its privacy note inside, with a folder
    /// selected and with none (the greyed palette and its hint).
    #[test]
    fn the_palette_keeps_its_size() {
        let ui_id = Uuid::from_u128(77);
        for selected in [Some(8200u128), None] {
            crate::ui::widgets::steady_tests::assert_window_steady(
                &format!("inventory_folder_color_{ui_id}"),
                &[Vec2::new(220.0, 150.0)],
                aurora_i18n::tr!("inventory-folder-color-private"),
                || {
                    let mut inv = Inventory::default();
                    crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
                    let mut prefs = InventoryPreferences::default();
                    let mut st = InventoryUi {
                        ui_id,
                        ..Default::default()
                    };
                    demo(&mut st, &mut prefs);
                    st.selection.clear();
                    st.selection.extend(selected.map(Uuid::from_u128));
                    (inv, st, prefs)
                },
                |(inv, st, prefs), ctx, p| show(ctx, p, inv, st, prefs),
            );
        }
    }

    #[test]
    fn colors_survive_a_save_and_old_settings_load_with_colors_on_and_none_set() {
        let old: InventoryPreferences = serde_json::from_str("{}").expect("empty settings");
        assert!(old.folder_colors_enabled && old.folder_colors.is_empty());
        let mut prefs = InventoryPreferences {
            folder_colors_enabled: false,
            ..Default::default()
        };
        set(&mut prefs, &[Uuid::from_u128(7)], Some(PRESETS[3]));
        let loaded: InventoryPreferences = serde_json::from_str(&serde_json::to_string(&prefs).expect("serialize")).expect("deserialize");
        assert_eq!(loaded, prefs);
    }
}
