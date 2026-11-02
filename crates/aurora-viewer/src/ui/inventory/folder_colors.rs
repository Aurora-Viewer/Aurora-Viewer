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

/// The color all of `ids` share, if they share one.
fn shared(prefs: &InventoryPreferences, ids: &[Uuid]) -> Option<[u8; 3]> {
    let first = *prefs.folder_colors.get(ids.first()?)?;
    ids.iter().all(|id| prefs.folder_colors.get(id) == Some(&first)).then_some(first)
}

/// The palette window of a selection of folders.
pub(super) struct Picker {
    ids: Vec<Uuid>,
    /// The custom swatch, seeded with the folders' color.
    custom: Rgba,
    /// Opened from a menu: comes to the front once shown. The window keeps
    /// its id, so egui reopens it at its old depth, under an inventory
    /// clicked since (the expanded window covers it).
    raise: bool,
}

impl Picker {
    pub(super) fn new(prefs: &InventoryPreferences, ids: Vec<Uuid>) -> Self {
        let [r, g, b] = shared(prefs, &ids).unwrap_or(PRESETS[7]);
        Self {
            ids,
            custom: [r, g, b, 255],
            raise: true,
        }
    }
}

/// The folders of a selection, the ones a color applies to.
pub(super) fn folders(inv: &Inventory, ids: &[Uuid]) -> Vec<Uuid> {
    ids.iter().copied().filter(|id| inv.folders.contains_key(id)).collect()
}

/// The palette window: a click on a swatch colors the folders at once; the
/// custom swatch opens the color picker; "Restore default" removes the color.
pub(super) fn show(ctx: &egui::Context, p: &Palette, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let ui_id = st.ui_id;
    let Some(picker) = &mut st.folder_color else {
        return;
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
        for row in PRESETS.chunks(PER_ROW) {
            ui.horizontal(|ui| {
                for &color in row {
                    if swatch(ui, p, color, current == Some(color)).clicked() {
                        set(prefs, &picker.ids, Some(color));
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
                set(prefs, &picker.ids, Some([r, g, b]));
            }
            ui.label(RichText::new(aurora_i18n::tr!("inventory-folder-color-custom")).color(p.ink));
        });
        let colored = picker.ids.iter().any(|id| prefs.folder_colors.contains_key(id));
        if ui
            .add_enabled(colored, egui::Button::new(aurora_i18n::tr!("inventory-folder-color-restore")))
            .clicked()
        {
            set(prefs, &picker.ids, None);
        }
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

/// One color of the palette, outlined when the folders have it.
fn swatch(ui: &mut egui::Ui, p: &Palette, [r, g, b]: [u8; 3], chosen: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(SWATCH), Sense::click());
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(rect, radius, Color32::from_rgb(r, g, b));
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

/// `AURORA_DEMO_INVENTORY=aurora-folder-colors-behind`: the palette is open,
/// the inventory widened and brought over it (a click on it), then "Folder
/// color…" again on another folder. Capture 750 (palette covered) and 820
/// (palette in front again).
pub fn demo_frame(ctx: &egui::Context, st: &mut InventoryUi, prefs: &InventoryPreferences, frame: u64) {
    if std::env::var("AURORA_DEMO_INVENTORY").as_deref() != Ok("aurora-folder-colors-behind") {
        return;
    }
    match frame {
        640 => super::aurora::widen(ctx, "inventory"),
        700 => crate::ui::widgets::bring_floater_front(ctx, "inventory"),
        760 => st.folder_color = Some(Picker::new(prefs, vec![Uuid::from_u128(8000)])),
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
    fn only_colored_folders_change_and_turning_colors_off_restores_every_icon() {
        let mut prefs = InventoryPreferences::default();
        let id = Uuid::from_u128(1);
        set(&mut prefs, &[id], Some([10, 20, 30]));
        assert_eq!(tint(&prefs, id), Some(Color32::from_rgb(10, 20, 30)));
        assert_eq!(tint(&prefs, Uuid::from_u128(2)), None);
        prefs.folder_colors_enabled = false;
        assert_eq!(tint(&prefs, id), None);
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
