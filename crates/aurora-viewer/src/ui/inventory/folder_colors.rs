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
/// turned on). With "Subfolder takes color from parent folder", a folder
/// without a color of its own takes the one of its nearest colored parent.
pub fn tint(prefs: &InventoryPreferences, inv: &Inventory, id: Uuid) -> Option<Color32> {
    if !prefs.folder_colors_enabled {
        return None;
    }
    color_of(prefs, inv, id).map(|[r, g, b]| Color32::from_rgb(r, g, b))
}

/// A folder's own color, or (inheriting) its nearest colored parent's.
fn color_of(prefs: &InventoryPreferences, inv: &Inventory, id: Uuid) -> Option<[u8; 3]> {
    if let Some(&color) = prefs.folder_colors.get(&id) {
        return Some(color);
    }
    if !prefs.folder_colors_inherit {
        return None;
    }
    let mut at = inv.folders.get(&id)?.info.parent;
    // bounded: a broken parent chain cannot loop
    for _ in 0..64 {
        if let Some(&color) = prefs.folder_colors.get(&at) {
            return Some(color);
        }
        at = inv.folders.get(&at)?.info.parent;
    }
    None
}

/// The folders a color goes to: every folder of the inventory ("Apply to
/// ALL folders", the Library's excluded), or the chosen ones with all their
/// subfolders ("Apply to all subfolders"), or the chosen ones.
fn targets(inv: &Inventory, ids: &[Uuid], subfolders: bool, all: bool) -> Vec<Uuid> {
    if all {
        let mut every: Vec<Uuid> = inv.folders.iter().filter(|(_, f)| !f.library).map(|(id, _)| *id).collect();
        every.sort_unstable();
        return every;
    }
    if !subfolders {
        return ids.to_vec();
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<Uuid> = ids.to_vec();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        out.push(id);
        if let Some(folder) = inv.folders.get(&id) {
            stack.extend(folder.children.iter().copied());
        }
    }
    out.sort_unstable();
    out
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
    /// "Apply to all subfolders": a color also goes to every folder inside.
    subfolders: bool,
    /// "Apply to ALL folders": a color goes to every folder, once confirmed.
    all: bool,
    /// A color (or `None`, the default) waiting for "Apply to ALL folders"
    /// to be confirmed.
    confirm: Option<Option<[u8; 3]>>,
}

impl Picker {
    pub(super) fn new(prefs: &InventoryPreferences, ids: Vec<Uuid>) -> Self {
        let [r, g, b] = shared(prefs, &ids).unwrap_or(PRESETS[7]);
        Self {
            ids,
            custom: [r, g, b, 255],
            raise: true,
            subfolders: false,
            all: false,
            confirm: None,
        }
    }

    /// A color chosen in the palette (`None`: "Restore default"), given to
    /// the folders the ticks say; for all folders, once confirmed.
    fn apply(&mut self, inv: &Inventory, prefs: &mut InventoryPreferences, color: Option<[u8; 3]>) {
        if self.all {
            self.confirm = Some(color);
        } else {
            set(prefs, &targets(inv, &self.ids, self.subfolders, false), color);
        }
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
    let every = inv.folders.values().filter(|f| !f.library).count();
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
        // "Apply to ALL folders" needs no selection
        ui.add_enabled_ui(target.is_some() || picker.all, |ui| {
            palette(ui, p, ui_id, inv, picker, prefs, current)
        });
        ui.add_space(2.0);
        ui.add_enabled_ui(!picker.all, |ui| {
            crate::ui::widgets::checkbox(ui, p, &mut picker.subfolders, aurora_i18n::tr!("inventory-folder-color-subfolders"));
        });
        crate::ui::widgets::checkbox(ui, p, &mut picker.all, aurora_i18n::tr!("inventory-folder-color-all"));
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-folder-color-private"))
                .size(11.0)
                .color(p.muted),
        );
    });
    if std::mem::take(&mut picker.raise) {
        crate::ui::widgets::bring_floater_front(ctx, &window_id);
    }
    confirm_all(ctx, p, inv, picker, prefs, every, &window_id);
    if !open || !prefs.folder_colors_enabled {
        st.folder_color = None;
    }
}

/// The swatches, the custom color and "Restore default" of the picker's folders.
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
                    picker.apply(inv, prefs, Some(color));
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
            picker.apply(inv, prefs, Some([r, g, b]));
        }
        ui.label(RichText::new(aurora_i18n::tr!("inventory-folder-color-custom")).color(p.ink));
    });
    let targets = targets(inv, &picker.ids, picker.subfolders, picker.all);
    let colored = targets.iter().any(|id| prefs.folder_colors.contains_key(id));
    if ui
        .add_enabled(colored, egui::Button::new(aurora_i18n::tr!("inventory-folder-color-restore")))
        .clicked()
    {
        picker.apply(inv, prefs, None);
    }
}

/// "Apply to ALL folders": a color (or the default) waits for this answer,
/// as the Legacy Viewer's confirmations do: Enter applies, Escape cancels.
fn confirm_all(
    ctx: &egui::Context,
    p: &Palette,
    inv: &Inventory,
    picker: &mut Picker,
    prefs: &mut InventoryPreferences,
    every: usize,
    window_id: &str,
) {
    let Some(color) = picker.confirm else {
        return;
    };
    let mut answer = None;
    crate::ui::widgets::Modal::new(egui::Id::unique((window_id, "all"))).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-folder-color-all-title"))
                .strong()
                .color(p.ink),
        );
        ui.add_space(6.0);
        let question = match color {
            Some(_) => aurora_i18n::tr!("inventory-folder-color-all-question", count = every),
            None => aurora_i18n::tr!("inventory-folder-color-all-restore-question", count = every),
        };
        ui.label(RichText::new(question).color(p.ink));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = aurora_i18n::tr!("inventory-folder-color-all-apply");
                if crate::ui::widgets::default_button_sized(ui, p, label, Vec2::new(120.0, 28.0)).clicked() {
                    answer = Some(true);
                }
                if crate::ui::widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(96.0, 28.0)).clicked() {
                    answer = Some(false);
                }
            });
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(false);
        }
    });
    match answer {
        Some(true) => {
            set(prefs, &targets(inv, &[], false, true), color);
            picker.confirm = None;
        }
        Some(false) => picker.confirm = None,
        None => {}
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
///   (no folder: "Select a folder", the palette greyed).
pub fn demo_frame(ctx: &egui::Context, st: &mut InventoryUi, prefs: &InventoryPreferences, frame: u64) {
    let Ok(mode) = std::env::var("AURORA_DEMO_INVENTORY") else { return };
    if !mode.contains("folder-colors") {
        return;
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
        _ => {}
    }
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

    #[test]
    fn only_colored_folders_change_and_turning_colors_off_restores_every_icon() {
        let mut prefs = InventoryPreferences::default();
        let id = Uuid::from_u128(1);
        let inv = Inventory::default();
        set(&mut prefs, &[id], Some([10, 20, 30]));
        assert_eq!(tint(&prefs, &inv, id), Some(Color32::from_rgb(10, 20, 30)));
        assert_eq!(tint(&prefs, &inv, Uuid::from_u128(2)), None);
        prefs.folder_colors_enabled = false;
        assert_eq!(tint(&prefs, &inv, id), None);
    }

    /// A small tree: root 1 > a 2 > b 3 > c 4, and root > d 5; library 9.
    fn tree() -> Inventory {
        let mut inv = Inventory::default();
        for (id, parent, library) in [
            (1u128, 0u128, false),
            (2, 1, false),
            (3, 2, false),
            (4, 3, false),
            (5, 1, false),
            (9, 0, true),
        ] {
            let (id, parent) = (Uuid::from_u128(id), Uuid::from_u128(parent));
            inv.folders.insert(
                id,
                crate::world::inventory::Folder {
                    info: aurora_net::inventory::InvFolder {
                        id,
                        parent,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: crate::world::inventory::FetchState::Fetched,
                    library,
                },
            );
            if let Some(parent) = inv.folders.get_mut(&parent) {
                parent.children.push(id);
            }
        }
        inv
    }

    #[test]
    fn subfolders_and_all_folders_take_the_color() {
        let inv = tree();
        let u = Uuid::from_u128;
        let mut picker = Picker::new(&InventoryPreferences::default(), vec![u(2)]);
        let mut prefs = InventoryPreferences::default();
        picker.subfolders = true;
        picker.apply(&inv, &mut prefs, Some(PRESETS[1]));
        assert_eq!(prefs.folder_colors.keys().copied().collect::<Vec<_>>(), vec![u(2), u(3), u(4)]);
        // all folders: nothing until confirmed, then every folder but the Library's
        picker.all = true;
        picker.apply(&inv, &mut prefs, Some(PRESETS[2]));
        assert_eq!(picker.confirm, Some(Some(PRESETS[2])));
        assert_eq!(prefs.folder_colors.get(&u(1)), None);
        set(&mut prefs, &targets(&inv, &[], false, true), Some(PRESETS[2]));
        assert_eq!(prefs.folder_colors.len(), 5);
        assert_eq!(prefs.folder_colors.get(&u(9)), None);
    }

    #[test]
    fn subfolders_take_the_parents_color_only_with_the_option() {
        let inv = tree();
        let u = Uuid::from_u128;
        let mut prefs = InventoryPreferences::default();
        set(&mut prefs, &[u(2)], Some([1, 2, 3]));
        set(&mut prefs, &[u(3)], Some([4, 5, 6]));
        assert_eq!(tint(&prefs, &inv, u(4)), None);
        prefs.folder_colors_inherit = true;
        // the nearest colored parent, never a sibling's
        assert_eq!(tint(&prefs, &inv, u(4)), Some(Color32::from_rgb(4, 5, 6)));
        assert_eq!(tint(&prefs, &inv, u(5)), None);
        set(&mut prefs, &[u(3)], None);
        assert_eq!(tint(&prefs, &inv, u(4)), Some(Color32::from_rgb(1, 2, 3)));
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
