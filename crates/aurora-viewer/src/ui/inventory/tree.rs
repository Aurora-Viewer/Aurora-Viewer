//! The pieces of the inventory tree that other trees of inventory reuse (the
//! Marketplace listings window): the caret and the type icon of a row, its
//! label, the list frame and its scrolling, the filter field and the outline
//! of a drop target. One look for every inventory tree.

use super::super::icons::Icons;
use crate::theme::Palette;
use egui::Vec2;

/// Row spacing of the tree (compact rows of 20 points).
pub fn spacing(ui: &mut egui::Ui) {
    super::super::widgets::inventory_spacing(ui);
}

/// The open / closed triangle before a folder; clicked to toggle it.
pub fn caret(ui: &mut egui::Ui, p: &Palette, open: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::click());
    let c = r.center();
    let pts = if open {
        vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)]
    } else {
        vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, p.muted, egui::Stroke::NONE));
    resp
}

/// The Legacy Viewer icon name of a folder by its preferred type.
pub fn folder_icon(type_default: i32, open: bool) -> &'static str {
    match type_default {
        0 => {
            if open {
                "Inv_TexturesOpen"
            } else {
                "Inv_TexturesClosed"
            }
        }
        1 => {
            if open {
                "Inv_SoundOpen"
            } else {
                "Inv_SoundClosed"
            }
        }
        2 => {
            if open {
                "Inv_CallingCardOpen"
            } else {
                "Inv_CallingCardClosed"
            }
        }
        3 => {
            if open {
                "Inv_LandmarksOpen"
            } else {
                "Inv_LandmarksClosed"
            }
        }
        5 => {
            if open {
                "Inv_ClothingOpen"
            } else {
                "Inv_ClothingClosed"
            }
        }
        6 => {
            if open {
                "Inv_ObjectsOpen"
            } else {
                "Inv_ObjectsClosed"
            }
        }
        7 => {
            if open {
                "Inv_NotecardsOpen"
            } else {
                "Inv_NotecardsClosed"
            }
        }
        50 => {
            if open {
                "Inv_InboxOpen"
            } else {
                "Inv_InboxClosed"
            }
        }
        51 => {
            if open {
                "Inv_OutboxOpen"
            } else {
                "Inv_OutboxClosed"
            }
        }
        8 | 52 | 53 => {
            if open {
                "Inv_SysOpen"
            } else {
                "Inv_SysClosed"
            }
        }
        10 => {
            if open {
                "Inv_ScriptsOpen"
            } else {
                "Inv_ScriptsClosed"
            }
        }
        13 => {
            if open {
                "Inv_BodyPartsOpen"
            } else {
                "Inv_BodyPartsClosed"
            }
        }
        14 => {
            if open {
                "Inv_TrashOpen"
            } else {
                "Inv_TrashClosed"
            }
        }
        15 => {
            if open {
                "Inv_SnapshotsOpen"
            } else {
                "Inv_SnapshotsClosed"
            }
        }
        16 => {
            if open {
                "Inv_LostOpen"
            } else {
                "Inv_LostClosed"
            }
        }
        20 => {
            if open {
                "Inv_AnimationsOpen"
            } else {
                "Inv_AnimationsClosed"
            }
        }
        21 => {
            if open {
                "Inv_GesturesOpen"
            } else {
                "Inv_GesturesClosed"
            }
        }
        23 => {
            if open {
                "Inv_FavoritesOpen"
            } else {
                "Inv_FavoritesClosed"
            }
        }
        46..=48 => {
            if open {
                "Inv_LookFolderOpen"
            } else {
                "Inv_LookFolderClosed"
            }
        }
        49 => {
            if open {
                "Inv_MeshesOpen"
            } else {
                "Inv_MeshesClosed"
            }
        }
        54 => {
            if open {
                "Inv_StockFolderOpen"
            } else {
                "Inv_StockFolderClosed"
            }
        }
        55 => {
            if open {
                "Inv_VersionFolderOpen"
            } else {
                "Inv_VersionFolderClosed"
            }
        }
        56 => {
            if open {
                "Inv_SettingsOpen"
            } else {
                "Inv_SettingsClosed"
            }
        }
        57 => {
            if open {
                "Inv_MaterialsOpen"
            } else {
                "Inv_MaterialsClosed"
            }
        }
        // FT_MY_SUITCASE (llviewerfoldertype.cpp: Inv_SuitcaseOpen / Closed)
        100 if open => "Inv_SuitcaseOpen",
        100 => "Inv_SuitcaseClosed",
        _ if open => "Inv_FolderOpen",
        _ => "Inv_FolderClosed",
    }
}

pub fn is_folder_icon(name: &str) -> bool {
    name.starts_with("Inv_") && (name.ends_with("Open") || name.ends_with("Closed"))
}

/// The type icon of a row: folders in the light accent, items in ink, or
/// `tint` when given (a state the row shows by its icon).
pub fn icon(ui: &mut egui::Ui, icons: &Icons, name: &str, p: &Palette, tint: Option<egui::Color32>) {
    if let Some(t) = icons.get(name) {
        let tint = tint.unwrap_or(if is_folder_icon(name) { p.violet_light } else { p.ink });
        ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::splat(16.0)).tint(tint));
    } else {
        ui.add_space(16.0);
    }
}

/// The label of a row: the whole width, selected or not, in the row font.
pub fn label(
    ui: &mut egui::Ui,
    label: egui::text::LayoutJob,
    selected: bool,
    sense: egui::Sense,
    bold_name: Option<std::sync::Arc<egui::Galley>>,
) -> egui::Response {
    let response = super::super::widgets::inventory_item_label(ui, label, selected, sense, bold_name);
    super::super::widgets::reveal_search_selection(ui, &response, selected);
    response
}

/// A name in the row font, for `label`.
pub fn name(name: &str, p: &Palette) -> egui::text::LayoutJob {
    super::super::widgets::name_with_suffix(name, "", font(), p.ink)
}

pub fn font() -> egui::FontId {
    egui::FontId::proportional(13.0)
}

/// The frame the tree scrolls in.
pub fn frame<R>(ui: &mut egui::Ui, p: &Palette, body: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(p.field.gamma_multiply(0.72))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(4)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
}

pub fn scroll(ui: &mut egui::Ui, height: f32) -> egui::ScrollArea {
    // (egui's own salt: the area keeps its saved offset)
    crate::ui::widgets::scroll_offset::show(ui, egui::ScrollArea::both(), "scroll_area")
        .auto_shrink([false, false])
        .max_width(ui.available_width())
        .max_height(height)
        .min_scrolled_height((height - ui.spacing().scroll.allocated_width()).max(40.0))
}

/// The height left to the list above the one-line footer.
pub fn height(ui: &egui::Ui) -> f32 {
    // One fixed-height footer, including the frame margins and layout spacing.
    // Async messages share this row so fetching never enlarges the floater.
    (ui.available_height() - 58.0).max(40.0)
}

/// The filter field (LLSearchEditor, indra/llui/llsearcheditor.cpp): the
/// clear action inside the field, its text padding kept even before typing.
/// Returns the field and whether the clear action was clicked.
pub fn search_field(
    ui: &mut egui::Ui,
    p: &Palette,
    text: &mut String,
    hint: &str,
    width: f32,
    clear_shown: bool,
    clear_tip: &str,
) -> (egui::Response, bool) {
    let centre = crate::ui::widgets::field_text_margin(ui.ctx(), &egui::TextStyle::Body.resolve(ui.style()));
    let field = ui.add_sized(
        [width, ui.spacing().interact_size.y],
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .align(egui::Align2::LEFT_CENTER)
            .margin(egui::Margin {
                left: 5,
                right: 28,
                ..centre
            }),
    );
    if !clear_shown {
        return (field, false);
    }
    let rect = egui::Rect::from_center_size(field.rect.right_center() - Vec2::new(14.0, 0.0), Vec2::splat(20.0));
    let clear = ui
        .interact(rect, field.id.with("clear"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(clear_tip);
    clear.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), clear_tip));
    let color = if clear.hovered() { p.ink } else { p.muted };
    if let Some(icon) = crate::ui::icons::global("x") {
        ui.painter().image(
            icon.id(),
            egui::Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    } else {
        ui.painter().line_segment(
            [rect.center() - Vec2::splat(3.0), rect.center() + Vec2::splat(3.0)],
            egui::Stroke::new(1.0, color),
        );
        ui.painter().line_segment(
            [rect.center() + Vec2::new(-3.0, 3.0), rect.center() + Vec2::new(3.0, -3.0)],
            egui::Stroke::new(1.0, color),
        );
    }
    let clicked = clear.clicked();
    if clicked {
        field.request_focus();
    }
    (field, clicked)
}

/// The outline of a row or a list that cargo hovers: the light accent when
/// it takes the drop, the danger colour when it refuses it.
pub fn drop_outline(ui: &egui::Ui, p: &Palette, rect: egui::Rect, allowed: bool) {
    let color = if allowed { p.violet_light } else { p.danger };
    ui.painter().rect_stroke(
        rect,
        ui.visuals().window_corner_radius,
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
}

/// The footer line of a tree window: small text, a message colour.
pub fn status_text(ui: &mut egui::Ui, text: &str, color: egui::Color32) -> egui::Response {
    ui.add(egui::Label::new(egui::RichText::new(text).size(11.0).color(color)).truncate())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_system_folder_types_have_specific_open_and_closed_icons() {
        let expected = [
            (0, "Inv_TexturesClosed", "Inv_TexturesOpen"),
            (1, "Inv_SoundClosed", "Inv_SoundOpen"),
            (2, "Inv_CallingCardClosed", "Inv_CallingCardOpen"),
            (3, "Inv_LandmarksClosed", "Inv_LandmarksOpen"),
            (5, "Inv_ClothingClosed", "Inv_ClothingOpen"),
            (6, "Inv_ObjectsClosed", "Inv_ObjectsOpen"),
            (7, "Inv_NotecardsClosed", "Inv_NotecardsOpen"),
            (8, "Inv_SysClosed", "Inv_SysOpen"),
            (10, "Inv_ScriptsClosed", "Inv_ScriptsOpen"),
            (13, "Inv_BodyPartsClosed", "Inv_BodyPartsOpen"),
            (14, "Inv_TrashClosed", "Inv_TrashOpen"),
            (15, "Inv_SnapshotsClosed", "Inv_SnapshotsOpen"),
            (16, "Inv_LostClosed", "Inv_LostOpen"),
            (20, "Inv_AnimationsClosed", "Inv_AnimationsOpen"),
            (21, "Inv_GesturesClosed", "Inv_GesturesOpen"),
            (23, "Inv_FavoritesClosed", "Inv_FavoritesOpen"),
            (46, "Inv_LookFolderClosed", "Inv_LookFolderOpen"),
            (47, "Inv_LookFolderClosed", "Inv_LookFolderOpen"),
            (48, "Inv_LookFolderClosed", "Inv_LookFolderOpen"),
            (49, "Inv_MeshesClosed", "Inv_MeshesOpen"),
            (50, "Inv_InboxClosed", "Inv_InboxOpen"),
            (51, "Inv_OutboxClosed", "Inv_OutboxOpen"),
            (52, "Inv_SysClosed", "Inv_SysOpen"),
            (53, "Inv_SysClosed", "Inv_SysOpen"),
            (54, "Inv_StockFolderClosed", "Inv_StockFolderOpen"),
            (55, "Inv_VersionFolderClosed", "Inv_VersionFolderOpen"),
            (56, "Inv_SettingsClosed", "Inv_SettingsOpen"),
            (57, "Inv_MaterialsClosed", "Inv_MaterialsOpen"),
            (100, "Inv_SuitcaseClosed", "Inv_SuitcaseOpen"),
        ];
        for (kind, closed, opened) in expected {
            assert_eq!(folder_icon(kind, false), closed, "closed folder type {kind}");
            assert_eq!(folder_icon(kind, true), opened, "open folder type {kind}");
            assert!(is_folder_icon(closed));
            assert!(is_folder_icon(opened));
        }
        assert!(!is_folder_icon("Inv_Material"), "material items keep the item tint");
        assert!(!is_folder_icon("Inv_Mesh"), "mesh items keep the item tint");
    }
}
