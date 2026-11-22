//! Per-account folder symbols. The skin supplies the image and the folder keeps its Legacy tint.
use super::*;
use aurora_i18n::tr;

const CHOICES: &[(&str, &str)] = &[
    ("star-filled", "common-favorites"),
    ("Inv_Object", "explorer-model-objects"),
    ("Inv_Clothing", "explorer-model-clothing"),
    ("Inv_BodyShape", "explorer-model-body-parts"),
    ("Inv_Texture", "explorer-model-textures"),
    ("Inv_Material", "explorer-model-materials"),
    ("Inv_Snapshot", "explorer-model-photos"),
    ("Inv_Settings", "explorer-model-environments"),
    ("Inv_Notecard", "explorer-model-notecards"),
    ("Inv_Script", "explorer-model-scripts"),
    ("Inv_Sound", "explorer-model-sounds"),
    ("Inv_Animation", "explorer-model-animations"),
    ("Inv_Gesture", "explorer-model-gestures"),
    ("Inv_Landmark", "explorer-model-saved-places"),
    ("Inv_CallingCard", "explorer-model-contact-cards"),
    ("building-store", "explorer-model-marketplace-listings"),
];

pub(super) fn for_row(layout: &Preferences, row: Row) -> Option<&'static str> {
    let Row::Place(place) = row else { return None };
    let saved = layout.folder_icon(place)?;
    CHOICES.iter().find(|(symbol, _)| *symbol == saved).map(|(symbol, _)| *symbol)
}

pub(super) fn editable(place: Location) -> bool {
    place != Location::FullTree
}

pub(super) fn menu(ui: &mut Ui, palette: &Palette, state: &mut ExplorerState, place: Location) {
    if !editable(place) {
        return;
    }
    let action = crate::ui::menu::current_row(ui, palette, "palette-filled", tr!("explorer-change-folder-icon"), "", false);
    if action.clicked() {
        state.edit_folder_icon = Some((place, action.rect));
        ui.close();
    }
}

pub(super) fn picker(ui: &Ui, style: &RowStyle, inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState) {
    let Some((place, anchor)) = state.edit_folder_icon else { return };
    if !editable(place) || !layout.available(inv, place) {
        state.edit_folder_icon = None;
        return;
    }
    let default = style.resolve(inv, Row::Place(place), false, false);
    let current = for_row(layout, Row::Place(place));
    let mut open = true;
    let mut chosen = None;
    let popup = egui::Popup::new(
        ui.scope_id().with(("folder-icon-picker", inv.root)),
        ui.ctx().clone(),
        egui::PopupAnchor::ParentRect(anchor),
        ui.layer_id(),
    );
    let p = style.palette;
    let frame = egui::Frame::popup(ui.style())
        .fill(p.panel.to_opaque())
        .stroke(egui::Stroke::new(1.0, p.muted_dim));
    popup
        .align(egui::RectAlign::RIGHT_START)
        .align_alternatives(&[
            egui::RectAlign::LEFT_START,
            egui::RectAlign::BOTTOM_START,
            egui::RectAlign::TOP_START,
        ])
        .frame(frame)
        .width(146.0)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            egui::Grid::new(ui.scope_id().with("symbols"))
                .num_columns(4)
                .spacing(vec2(4.0, 4.0))
                .show(ui, |ui| {
                    for (i, &(symbol, key)) in CHOICES.iter().enumerate() {
                        let label = aurora_i18n::text(key);
                        let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 30.0), egui::Sense::click());
                        let active = current == Some(symbol);
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
                        if resp.hovered() || resp.has_focus() || active {
                            ui.painter()
                                .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
                        }
                        if active || resp.has_focus() {
                            ui.painter().rect_stroke(
                                rect.shrink(0.5),
                                ui.visuals().widgets.inactive.corner_radius,
                                egui::Stroke::new(1.0, p.violet_pale),
                                egui::StrokeKind::Inside,
                            );
                        }
                        style.glyph(
                            ui.painter(),
                            Glyph::navigation(symbol, default.tint()),
                            Rect::from_center_size(rect.center(), vec2(20.0, 20.0)),
                        );
                        if resp.clicked() {
                            chosen = Some(Some(symbol));
                        }
                        if resp.has_focus() && !resp.hovered() {
                            egui::Tooltip::for_widget(&resp).show(|ui| {
                                ui.label(label);
                            });
                        } else {
                            resp.on_hover_text(label);
                        }
                        if (i + 1) % 4 == 0 {
                            ui.end_row();
                        }
                    }
                });
            ui.add_space(4.0);
            if ui
                .add_enabled(current.is_some(), egui::Button::new(tr!("explorer-restore-folder-icon")))
                .clicked()
            {
                chosen = Some(None);
            }
        });
    if let Some(symbol) = chosen {
        layout.set_folder_icon(place, symbol);
        open = false;
    }
    if !open {
        state.edit_folder_icon = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn organized_folder_symbols_persist_by_identity_leave_items_alone_and_reset() {
        let (inv, id) = (super::super::tests::fixture(), super::super::tests::id);
        let place = Location::Folder(id(3));
        let mut db = Preferences::default();
        let base_revision = db.revision();
        db.set_folder_icon(place, Some("Inv_Snapshot"));
        assert!(db.revision() > base_revision);
        let mut loaded: Preferences = serde_json::from_slice(&serde_json::to_vec(&db).unwrap()).unwrap();
        assert_eq!(for_row(&loaded, Row::Place(place)), Some("Inv_Snapshot"));
        assert_eq!(for_row(&loaded, Row::Place(Location::Folder(id(4)))), None);
        assert_eq!(for_row(&loaded, Row::Item(id(3))), None);
        loaded.set_folder_icon(place, None);
        assert_eq!(for_row(&loaded, Row::Place(place)), None);
        let local = loaded.create_custom(&inv, place, "Old virtual folder").unwrap();
        loaded.set_folder_icon(local, Some("Inv_Clothing"));
        let Location::Custom(key) = local else {
            panic!("expected local folder")
        };
        loaded.remove_custom(key);
        assert_eq!(loaded.folder_icon(local), None);
    }
}
