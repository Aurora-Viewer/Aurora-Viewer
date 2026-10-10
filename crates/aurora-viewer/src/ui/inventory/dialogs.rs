//! Inventory properties, thumbnail and confirmation floaters.
//! Follows LLFloaterProperties / LLFloaterInventory (Firestorm, LGPL 2.1).

use super::*;
use crate::ui::widgets::{Floater, flat_button};
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::Operation;

pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
) {
    let Some(mut dialog) = st.dialog.take() else {
        return;
    };
    let mut open = true;
    let mut done = false;
    let title = match &dialog {
        EditDialog::Properties(..) => aurora_i18n::tr!("ui-inventory-dialogs-object-properties"),
        EditDialog::Delete(_, true) => aurora_i18n::tr!("ui-inventory-dialogs-purge-selection"),
        EditDialog::Delete(_, false) => aurora_i18n::tr!("ui-inventory-dialogs-delete-selection"),
        EditDialog::Empty(_) => aurora_i18n::tr!("common-empty-trash"),
        EditDialog::Ungroup(_) => aurora_i18n::tr!("ui-inventory-dialogs-ungroup-folder"),
        EditDialog::ReplaceLinks(..) => aurora_i18n::tr!("common-replace-links"),
        EditDialog::Discard(..) => aurora_i18n::tr!("ui-inventory-dialogs-save-changes"),
        EditDialog::Share(..) => aurora_i18n::tr!("ui-inventory-dialogs-share-selection"),
    };
    let properties = matches!(dialog, EditDialog::Properties(_));
    let confirmation = matches!(dialog, EditDialog::Delete(..) | EditDialog::Empty(_));
    let window_id = format!(
        "inventory_{}_{}",
        if properties {
            "properties"
        } else if confirmation {
            "confirmation"
        } else {
            "dialog"
        },
        st.ui_id
    );
    let mut floater = Floater::new(
        &window_id,
        title,
        ctx.content_rect().center()
            - if properties {
                egui::vec2(175.0, 305.0)
            } else {
                egui::vec2(210.0, 130.0)
            },
        if properties {
            Vec2::new(350.0, 610.0)
        } else {
            Vec2::new(420.0, 260.0)
        },
    );
    if confirmation {
        floater = floater.fixed();
    }
    floater.show(ctx, p, &mut open, |ui| {
        match &mut dialog {
            EditDialog::Properties(id) => {
                done = super::properties::show(ui, p, icons, inv, st, prefs, facts, images, *id, actions);
            }
            EditDialog::Discard(id) => {
                ui.label(aurora_i18n::tr!("ui-inventory-dialogs-this-document-has-unsaved-changes"));
                if ui
                    .add_enabled(!st.save_pending, egui::Button::new(aurora_i18n::tr!("common-save")))
                    .clicked()
                    && let Some(Ok(text)) = &st.preview_text
                {
                    actions.push(InvAction::SaveContent {
                        item: *id,
                        text: text.clone(),
                    });
                    done = true;
                }
                if flat_button(ui, p, aurora_i18n::tr!("ui-inventory-dialogs-discard-changes")).clicked() {
                    st.preview = None;
                    st.preview_dirty = false;
                    done = true;
                }
            }
            EditDialog::Share(ids, resident) => {
                ui.label(aurora_i18n::tr!(
                    "ui-inventory-dialogs-send-item-s-to-resident",
                    value0 = format!("{}", ids.len()),
                    value1 = format!("{}", resident)
                ));
                if ids.iter().any(|id| {
                    rules::offer(inv, *id, &prefs.protected, &facts.worn).is_ok_and(|bucket| rules::offer_has_no_copy(inv, &bucket))
                }) {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-dialogs-no-copy-items-will-leave-your-inventory")).color(p.warn));
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-share")).clicked() {
                    actions.push(InvAction::Share {
                        items: ids.clone(),
                        resident: *resident,
                    });
                    done = true;
                }
            }
            EditDialog::ReplaceLinks(old, selected) => {
                if st.fetch_all {
                    ui.label(aurora_i18n::tr!("ui-inventory-dialogs-loading-all-folders-before-replacing"));
                }
                ui.label(aurora_i18n::tr!(
                    "ui-inventory-dialogs-replace-links-to-with",
                    value0 = rules::name(inv, *old).to_string()
                ));
                let old_type = inv.items.get(old).map(|it| (it.asset_type, it.flags & 0xff));
                let mut candidates: Vec<_> = inv
                    .items
                    .values()
                    .filter(|it| {
                        it.id != *old && !matches!(it.asset_type, 24 | 25) && !rules::library(inv, it.id) && !rules::in_type(inv, it.id, 14)
                    })
                    .filter(|it| {
                        old_type
                            .is_some_and(|(kind, subtype)| it.asset_type == kind && (!matches!(kind, 5 | 13) || it.flags & 0xff == subtype))
                    })
                    .collect();
                candidates.sort_by_key(|it| it.name.to_lowercase());
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    for it in candidates {
                        if ui.selectable_label(*selected == Some(it.id), &it.name).clicked() {
                            *selected = Some(it.id);
                        }
                    }
                });
                if ui
                    .add_enabled(
                        selected.is_some() && st.pending.is_none() && !st.fetch_all && !facts.appearance_busy,
                        egui::Button::new(aurora_i18n::tr!("ui-inventory-dialogs-replace")),
                    )
                    .clicked()
                    && let Some(new) = *selected
                {
                    let mut change = Mutation::default();
                    for link in inv.items.values().filter(|it| {
                        it.asset_type == 24
                            && it.asset_id == *old
                            && !rules::in_type(inv, it.id, 14)
                            && (rules::mutable(inv, it.id, &prefs.protected) || rules::in_type(inv, it.id, 46))
                    }) {
                        let replacement = &inv.items[&new];
                        let outfit = rules::in_type(inv, link.id, 47) || rules::in_type(inv, link.id, 46);
                        let desc = if outfit {
                            if replacement.asset_type == 5 {
                                link.desc.clone()
                            } else {
                                String::new()
                            }
                        } else {
                            replacement.desc.clone()
                        };
                        change.operations.push(Operation::Links {
                            parent: link.parent,
                            links: vec![aurora_net::outfits::OutfitLink {
                                target: new,
                                name: replacement.name.clone(),
                                desc,
                                folder: false,
                                inv_type: replacement.inv_type,
                            }],
                        });
                        change.operations.push(Operation::Delete {
                            id: link.id,
                            folder: false,
                        });
                        change.removed.push(link.id);
                        change.refresh.push(link.parent);
                    }
                    if change.operations.is_empty() {
                        st.message = aurora_i18n::tr!("ui-inventory-dialogs-no-editable-links-in-the-loaded-folders").into();
                    } else {
                        actions.push(InvAction::Edit(change));
                    }
                    done = true;
                }
            }
            EditDialog::Delete(ids, purge) => {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(if ids.len() == 1 {
                        aurora_i18n::tr!("ui-inventory-dialogs-delete", value0 = rules::name(inv, ids[0]).to_string())
                    } else {
                        aurora_i18n::tr!("ui-inventory-dialogs-delete-the-selected-items", value0 = format!("{}", ids.len()))
                    })
                    .strong()
                    .color(p.ink),
                );
                ui.add_space(4.0);
                ui.label(if *purge {
                    aurora_i18n::tr!("ui-inventory-dialogs-this-deletion-is-permanent")
                } else {
                    aurora_i18n::tr!("ui-inventory-dialogs-items-will-be-moved-to-trash")
                });
                if confirmation_buttons(
                    ui,
                    p,
                    st.pending.is_none(),
                    if *purge {
                        aurora_i18n::tr!("ui-inventory-dialogs-purge")
                    } else {
                        aurora_i18n::tr!("common-delete")
                    },
                    &mut done,
                ) {
                    let plan = if *purge {
                        let ids = rules::roots(inv, ids);
                        let refresh = ids.iter().filter_map(|id| rules::parent(inv, *id)).collect();
                        let operations = ids
                            .iter()
                            .map(|id| Operation::Delete {
                                id: *id,
                                folder: inv.folders.contains_key(id),
                            })
                            .collect();
                        if ids.iter().all(|id| {
                            rules::movable(inv, *id, &prefs.protected)
                                && rules::in_type(inv, *id, 14)
                                && !facts.worn.iter().any(|w| rules::under(inv, *w, *id))
                        }) {
                            Ok(Mutation {
                                operations,
                                refresh,
                                removed: ids,
                                ..Default::default()
                            })
                        } else {
                            Err(aurora_i18n::tr!("ui-inventory-dialogs-the-selection-can-no-longer-be-purged").into())
                        }
                    } else {
                        rules::system(inv, 14)
                            .ok_or_else(|| aurora_i18n::tr!("common-trash-folder-not-found").to_owned())
                            .and_then(|to| rules::move_selection(inv, ids, to, &prefs.protected, &facts.worn))
                    };
                    match plan {
                        Ok(change) => {
                            actions.push(InvAction::Edit(change));
                            done = true;
                        }
                        Err(reason) => st.message = reason,
                    }
                }
            }
            EditDialog::Empty(id) => {
                ui.label(aurora_i18n::tr!(
                    "ui-inventory-dialogs-all-items-in-trash-will-be-permanently-deleted"
                ));
                if confirmation_buttons(ui, p, st.pending.is_none(), aurora_i18n::tr!("common-empty-trash"), &mut done)
                    && let Some(f) = inv.folders.get(id)
                    && f.info.type_default == 14
                    && f.state == FetchState::Fetched
                {
                    let removed: Vec<_> = f.items.iter().chain(&f.children).copied().collect();
                    if !facts.worn.iter().any(|w| rules::under(inv, *w, *id)) {
                        actions.push(InvAction::Edit(Mutation {
                            operations: vec![Operation::Purge(*id)],
                            refresh: vec![*id],
                            removed,
                            ..Default::default()
                        }));
                        done = true;
                    } else {
                        st.message = aurora_i18n::tr!("ui-inventory-dialogs-trash-contains-a-worn-item").into();
                    }
                }
            }
            EditDialog::Ungroup(id) => {
                ui.label(aurora_i18n::tr!(
                    "ui-inventory-dialogs-move-the-contents-to-the-parent-folder-and-put"
                ));
                if ui
                    .add_enabled(
                        st.pending.is_none(),
                        egui::Button::new(aurora_i18n::tr!("ui-inventory-dialogs-ungroup")),
                    )
                    .clicked()
                {
                    match rules::ungroup(inv, *id, &prefs.protected, &facts.worn) {
                        Ok(change) => {
                            actions.push(InvAction::Edit(change));
                            done = true;
                        }
                        Err(reason) => st.message = reason,
                    }
                }
            }
        }
        if !st.message.is_empty() {
            ui.label(RichText::new(&st.message).color(p.warn));
        }
        if (!confirmation && flat_button(ui, p, aurora_i18n::tr!("extra-close-cancel")).clicked())
            || ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            done = true;
        }
    });
    if open && !done {
        st.dialog = Some(dialog);
    }
}

fn confirmation_buttons(ui: &mut egui::Ui, p: &Palette, enabled: bool, label: &str, done: &mut bool) -> bool {
    ui.add_space(16.0);
    ui.separator();
    ui.add_space(6.0);
    let mut confirm = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                confirm = super::super::widgets::flat_button_sized(ui, p, label, Vec2::new(120.0, 28.0)).clicked();
            });
            if super::super::widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(96.0, 28.0)).clicked() {
                *done = true;
            }
        });
    });
    confirm
}

pub(super) fn preview(
    ctx: &egui::Context,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    prefs: &InventoryPreferences,
    actions: &mut Vec<InvAction>,
) {
    let Some(id) = st.preview else {
        return;
    };
    let Some(it) = inv.items.get(&id) else {
        st.preview = None;
        return;
    };
    if it.asset_type == 20 {
        animation(ctx, p, inv, st, it, prefs, actions);
        return;
    }
    let mut open = true;
    Floater::new(
        &format!("inventory_preview_{}_{}", st.ui_id, id),
        it.name.clone(),
        ctx.content_rect().center() - egui::vec2(250.0, 200.0),
        Vec2::new(500.0, 400.0),
    )
    .show(ctx, p, &mut open, |ui| {
        if it.asset_type == 0 {
            st.wanted_images.insert(it.asset_id);
            if let Some(t) = images.get(&it.asset_id) {
                ui.add(egui::Image::new(t).max_size(Vec2::splat(350.0)).fit_to_original_size(1.0));
            } else {
                ui.label(aurora_i18n::tr!("ui-inventory-dialogs-loading-image"));
            }
        } else if matches!(it.asset_type, 20 | 1) {
            if flat_button(ui, p, aurora_i18n::tr!("common-play")).clicked() {
                if it.asset_type == 20 {
                    actions.push(InvAction::Animation {
                        asset: it.asset_id,
                        local: false,
                        start: true,
                    });
                } else {
                    actions.push(InvAction::Sound(it.asset_id));
                }
            }
            if it.asset_type == 20 {
                if flat_button(ui, p, aurora_i18n::tr!("common-play-locally")).clicked() {
                    actions.push(InvAction::Animation {
                        asset: it.asset_id,
                        local: true,
                        start: true,
                    });
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-stop")).clicked()
                    && let Some((asset, local)) = st.playing
                {
                    actions.push(InvAction::Animation {
                        asset,
                        local,
                        start: false,
                    });
                }
            }
        } else {
            if it.asset_type == 56 && !it.asset_id.is_nil() && flat_button(ui, p, aurora_i18n::tr!("common-apply-to-myself-only")).clicked()
            {
                actions.push(InvAction::Environment(it.asset_id, it.flags));
            }
            match &mut st.preview_text {
                Some(Ok(text)) => {
                    egui::ScrollArea::both().max_height(300.0).show(ui, |ui| {
                        if st.preview_can_edit && !st.save_pending {
                            let response = ui.add(
                                egui::TextEdit::multiline(text)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Monospace),
                            );
                            st.preview_dirty |= response.changed();
                        } else {
                            ui.add(egui::Label::new(RichText::new(text.as_str()).monospace()).selectable(true));
                        }
                    });
                }
                Some(Err(reason)) => {
                    ui.label(RichText::new(reason.as_str()).color(p.warn));
                }
                None => {
                    ui.label(aurora_i18n::tr!("common-loading-2"));
                }
            }
            if matches!(it.asset_type, 7 | 10)
                && st.preview_can_edit
                && ui
                    .add_enabled(
                        st.preview_dirty && !st.save_pending,
                        egui::Button::new(aurora_i18n::tr!("common-save")),
                    )
                    .clicked()
                && let Some(Ok(text)) = &st.preview_text
            {
                actions.push(InvAction::SaveContent {
                    item: it.id,
                    text: text.clone(),
                });
            }
            if !st.message.is_empty() {
                ui.label(RichText::new(&st.message).color(p.warn));
            }
        }
    });
    if !open {
        if st.preview_dirty || st.save_pending {
            st.dialog = Some(EditDialog::Discard(id));
            return;
        }
        st.preview = None;
        if let Some((asset, local)) = st.playing {
            actions.push(InvAction::Animation {
                asset,
                local,
                start: false,
            });
        }
    }
}

fn animation(
    ctx: &egui::Context,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    it: &aurora_net::inventory::InvItem,
    prefs: &InventoryPreferences,
    actions: &mut Vec<InvAction>,
) {
    if st.animation_description.as_ref().is_none_or(|(id, _)| *id != it.id) {
        st.animation_description = Some((it.id, it.desc.clone()));
        st.animation_details = true;
    }
    let mut open = true;
    Floater::new(
        &format!("inventory_preview_{}_{}", st.ui_id, it.id),
        aurora_i18n::tr!("ui-inventory-dialogs-animation", value0 = it.name.to_string()),
        ctx.content_rect().center() - egui::vec2(215.0, 85.0),
        Vec2::new(430.0, 170.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("common-description"));
            if let Some((_, description)) = &mut st.animation_description {
                let can_edit = rules::mutable(inv, it.id, &prefs.protected) && it.owner_mask & rules::MODIFY != 0 && st.pending.is_none();
                let response = ui.add_enabled(
                    can_edit,
                    egui::TextEdit::singleline(description)
                        .char_limit(127)
                        .desired_width(ui.available_width()),
                );
                if can_edit
                    && description != &it.desc
                    && response.lost_focus()
                    && let Ok(change) = rules::patch(inv, it.id, llsd_map! { "desc" => description.clone() })
                {
                    actions.push(InvAction::Edit(change));
                }
            }
        });
        ui.horizontal(|ui| {
            let playing = st.playing.filter(|(asset, _)| *asset == it.asset_id);
            for (local, label) in [
                (false, aurora_i18n::tr!("ui-inventory-dialogs-play-for-everyone")),
                (true, aurora_i18n::tr!("common-play-locally")),
            ] {
                let same = playing.is_some_and(|(_, mode)| mode == local);
                if ui
                    .add_enabled(
                        playing.is_none() || same,
                        egui::Button::new(if same { aurora_i18n::tr!("common-stop") } else { label }),
                    )
                    .clicked()
                {
                    actions.push(InvAction::Animation {
                        asset: it.asset_id,
                        local,
                        start: !same,
                    });
                }
            }
            if crate::ui::widgets::icon_button(
                ui,
                p,
                if st.animation_details { "caret-up" } else { "caret-down" },
                aurora_i18n::tr!("ui-inventory-dialogs-show-statistics"),
                true,
            ) {
                st.animation_details = !st.animation_details;
            }
        });
        if st.animation_details {
            if let Some((_, stats)) = st.animation_stats.as_ref().filter(|(id, _)| *id == it.id) {
                egui::Grid::new("animation_stats")
                    .num_columns(2)
                    .spacing(egui::vec2(64.0, 2.0))
                    .show(ui, |ui| {
                        ui.label(aurora_i18n::tr!(
                            "ui-inventory-dialogs-priority",
                            value0 = format!("{}", stats.priority)
                        ));
                        ui.label(aurora_i18n::tr!(
                            "ui-inventory-dialogs-ease-in-s",
                            value0 = aurora_i18n::number_text(&format!("{:.2}", stats.entry))
                        ));
                        ui.end_row();
                        ui.label(aurora_i18n::tr!(
                            "ui-inventory-dialogs-duration-s",
                            value0 = aurora_i18n::number_text(&format!("{:.2}", stats.duration))
                        ));
                        ui.label(aurora_i18n::tr!(
                            "ui-inventory-dialogs-ease-out-s",
                            value0 = aurora_i18n::number_text(&format!("{:.2}", stats.exit))
                        ));
                        ui.end_row();
                        ui.label(aurora_i18n::tr!(
                            "ui-inventory-dialogs-looping",
                            value0 = (if stats.looping {
                                aurora_i18n::tr!("common-yes")
                            } else {
                                aurora_i18n::tr!("common-no")
                            })
                            .to_string()
                        ));
                        ui.label(aurora_i18n::tr!(
                            "ui-inventory-dialogs-joints",
                            value0 = format!("{}", stats.joints)
                        ));
                        ui.end_row();
                    });
            } else {
                ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-dialogs-loading-animation-statistics")).color(p.muted));
            }
        }
    });
    if !open {
        st.preview = None;
        if let Some((asset, local)) = st.playing.filter(|(asset, _)| *asset == it.asset_id) {
            actions.push(InvAction::Animation {
                asset,
                local,
                start: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_confirmation_places_both_buttons_in_one_footer_row() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let mut st = InventoryUi {
            dialog: Some(EditDialog::Delete(
                vec![Uuid::from_u128(8100), Uuid::from_u128(8109), Uuid::from_u128(8110)],
                false,
            )),
            ..Default::default()
        };
        let mut output = None;
        for _ in 0..5 {
            output = Some(headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                    ..Default::default()
                },
                |_ui| {
                    show(
                        &ctx,
                        &p,
                        &Icons::default(),
                        &inv,
                        &mut st,
                        &InventoryPreferences::default(),
                        &Facts {
                            agent: Uuid::nil(),
                            worn: HashSet::new(),
                            worn_labels: Default::default(),
                            points: Vec::new(),
                            appearance_busy: false,
                            names: Default::default(),
                        },
                        &Default::default(),
                        &mut Vec::new(),
                    );
                },
            )));
        }
        let output = output.expect("confirmation frame");
        let text = |label| {
            output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(t.pos),
                    _ => None,
                })
                .expect("confirmation label")
        };
        let cancel = text(aurora_i18n::tr!("common-cancel"));
        let delete = text(aurora_i18n::tr!("common-delete"));
        assert!((cancel.y - delete.y).abs() < 1.0);
        assert!(cancel.x < delete.x);
        assert!(delete.y > text(aurora_i18n::tr!("ui-inventory-dialogs-items-will-be-moved-to-trash")).y);
        assert!(
            delete.y - text(aurora_i18n::tr!("ui-inventory-dialogs-items-will-be-moved-to-trash")).y < 65.0,
            "footer must stay close to the message"
        );
        assert!(
            !output
                .shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if t.galley.text() == "Fermer / Annuler"))
        );
    }
}
