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
        EditDialog::Discard(..) | EditDialog::SaveEmbedded(..) => aurora_i18n::tr!("ui-inventory-dialogs-save-changes"),
        EditDialog::Share(..) => aurora_i18n::tr!("ui-inventory-dialogs-share-selection"),
        EditDialog::CopyEmbedded(..) => aurora_i18n::tr!("notecard-copy-inventory"),
    };
    let properties = matches!(dialog, EditDialog::Properties(_));
    let document_confirmation = matches!(
        dialog,
        EditDialog::Discard(_) | EditDialog::CopyEmbedded(..) | EditDialog::SaveEmbedded(_)
    );
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
            } else if document_confirmation {
                egui::vec2(220.0, 70.0)
            } else {
                egui::vec2(210.0, 130.0)
            },
        if properties {
            Vec2::new(350.0, 610.0)
        } else if document_confirmation {
            Vec2::new(440.0, 140.0)
        } else {
            Vec2::new(420.0, 260.0)
        },
    );
    if confirmation || document_confirmation {
        floater = floater.fixed();
    }
    floater.show(ctx, p, &mut open, |ui| {
        match &mut dialog {
            EditDialog::Properties(id) => {
                done = super::properties::show(ui, p, icons, inv, st, prefs, facts, images, *id, actions);
            }
            EditDialog::Discard(id) => {
                ui.label(aurora_i18n::tr!("ui-inventory-dialogs-this-document-has-unsaved-changes"));
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !st.save_pending
                                && st.preview_can_edit
                                && crate::world::inventory::preview::can_edit(inv, *id)
                                && st.preview_text.as_ref().is_some_and(|r| {
                                    r.as_ref().is_ok_and(|t| {
                                        inv.items.get(id).is_none_or(|it| it.asset_type != 7)
                                            || aurora_net::inventory::notecard::Notecard::validate_text(t).is_ok()
                                    })
                                }),
                            egui::Button::new(aurora_i18n::tr!("common-save")),
                        )
                        .clicked()
                        && let Some(Ok(text)) = &st.preview_text
                    {
                        actions.push(InvAction::SaveContent {
                            editor: st.ui_id,
                            item: *id,
                            text: text.clone(),
                            close: true,
                        });
                        done = true;
                    }
                    if ui
                        .add_enabled(
                            !st.save_pending,
                            egui::Button::new(aurora_i18n::tr!("ui-inventory-dialogs-discard-changes")),
                        )
                        .clicked()
                    {
                        st.preview = None;
                        st.reset_document();
                        done = true;
                    }
                    if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                        done = true;
                    }
                });
                if !st.preview_message.is_empty() {
                    ui.label(RichText::new(&st.preview_message).color(p.warn));
                }
            }
            EditDialog::CopyEmbedded(notecard, marker) => {
                let item = st.preview_source.as_ref().and_then(|source| source.attachment(*marker));
                if let Some(item) = item {
                    ui.label(aurora_i18n::tr!("notecard-copy-confirm", name = item.item.name.as_str()));
                    ui.horizontal(|ui| {
                        let button = flat_button(ui, p, aurora_i18n::tr!("notecard-copy-inventory"));
                        ctx.data_mut(|data| data.insert_temp(egui::Id::new(("notecard_copy_confirm", st.ui_id)), button.rect));
                        if button.clicked() {
                            actions.push(InvAction::Embedded {
                                editor: st.ui_id,
                                notecard: *notecard,
                                marker: *marker,
                                copy: true,
                            });
                            done = true;
                        }
                        if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                            done = true;
                        }
                    });
                } else {
                    done = true;
                }
            }
            EditDialog::SaveEmbedded(notecard) => {
                ui.label(aurora_i18n::tr!("notecard-attachment-unsaved"));
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !st.save_pending && st.preview_can_edit && crate::world::inventory::preview::can_edit(inv, *notecard),
                            egui::Button::new(aurora_i18n::tr!("common-save")),
                        )
                        .clicked()
                        && let Some(Ok(text)) = &st.preview_text
                    {
                        actions.push(InvAction::SaveContent {
                            editor: st.ui_id,
                            item: *notecard,
                            text: text.clone(),
                            close: false,
                        });
                        done = true;
                    }
                    if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                        done = true;
                    }
                });
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
        if (!confirmation && !document_confirmation && flat_button(ui, p, aurora_i18n::tr!("extra-close-cancel")).clicked())
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
    crate::ui::widgets::divider(ui, p);
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
    icons: &Icons,
    inv: &Inventory,
    drag_inventory: &Inventory,
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
    let editable = st.preview_can_edit && crate::world::inventory::preview::can_edit(inv, id);
    if it.asset_type == 7
        && !st.save_pending
        && let (Some(source), Some(Ok(text))) = (&mut st.preview_source, &st.preview_text)
    {
        super::embedded::clipboard(
            ctx,
            egui::Id::new(("inventory_document_text", st.ui_id, id)),
            source,
            text,
            editable,
        );
    }
    let mut open = true;
    let size = if it.asset_type == 7 {
        Vec2::new(760.0, 600.0)
    } else {
        Vec2::new(500.0, 400.0)
    };
    let size = size.min((ctx.content_rect().size() - Vec2::splat(32.0)).max(Vec2::new(300.0, 220.0)));
    Floater::new(
        &format!(
            "{}_{}_{}",
            if it.asset_type == 7 {
                "notecard_preview"
            } else {
                "inventory_preview"
            },
            st.ui_id,
            id
        ),
        format!("{}{}", it.name, if st.preview_dirty { " *" } else { "" }),
        ctx.content_rect().center() - size * 0.5,
        size,
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
            let mut edited = super::embedded::EditorResult::default();
            match &mut st.preview_text {
                Some(Ok(text)) => {
                    if matches!(it.asset_type, 7 | 10) && !editable {
                        ui.label(RichText::new(aurora_i18n::tr!("notecard-read-only")).color(p.muted));
                    }
                    let documents = matches!(it.asset_type, 7 | 10);
                    let footer_height = ui.spacing().interact_size.y
                        + ui.spacing().item_spacing.y * 2.0
                        + if st.preview_message.is_empty() {
                            0.0
                        } else {
                            ui.fonts_mut(|fonts| {
                                fonts.layout(
                                    st.preview_message.clone(),
                                    egui::TextStyle::Body.resolve(ui.style()),
                                    p.ink,
                                    ui.available_width(),
                                )
                            })
                            .size()
                            .y + ui.spacing().item_spacing.y
                        }
                        + if it.asset_type == 7 && aurora_net::inventory::notecard::Notecard::validate_text(text).is_err() {
                            ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y
                        } else {
                            0.0
                        };
                    let editor_size = Vec2::new(ui.available_width(), (ui.available_height() - footer_height).max(60.0));
                    egui::ScrollArea::both()
                        .max_height(if documents { editor_size.y } else { 300.0 })
                        .auto_shrink(if documents { [false, false] } else { [true, true] })
                        .show(ui, |ui| {
                            if matches!(it.asset_type, 7 | 10) {
                                edited = super::embedded::editor(
                                    ui,
                                    p,
                                    icons,
                                    egui::Id::new(("inventory_document_text", st.ui_id, id)),
                                    text,
                                    st.preview_source.as_ref(),
                                    editable && !st.save_pending && !matches!(st.dialog, Some(EditDialog::Discard(_))),
                                    editor_size,
                                );
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
            if let Some((position, dropped)) = edited.drop {
                let item = rules::original(drag_inventory, dropped).and_then(|id| drag_inventory.items.get(&id));
                if let Some(item) = item
                    && let (Some(source), Some(Ok(text))) = (&mut st.preview_source, &mut st.preview_text)
                {
                    match source.insert_attachment(text, position, item) {
                        Ok(_) => edited.changed = true,
                        Err(reason) => st.preview_message = reason,
                    }
                } else {
                    st.preview_message = aurora_i18n::tr!("common-item-not-found").into();
                }
            }
            if edited.changed {
                st.document_changed();
            }
            if let Some(marker) = edited.open {
                actions.push(InvAction::Embedded {
                    editor: st.ui_id,
                    notecard: id,
                    marker,
                    copy: false,
                });
            }
            if let Some(marker) = edited.copy {
                st.dialog = Some(EditDialog::CopyEmbedded(id, marker));
            }
            let valid_size = st.preview_text.as_ref().is_some_and(|r| {
                r.as_ref()
                    .is_ok_and(|text| it.asset_type != 7 || aurora_net::inventory::notecard::Notecard::validate_text(text).is_ok())
            });
            let can_save =
                editable && st.preview_dirty && !st.save_pending && valid_size && !matches!(st.dialog, Some(EditDialog::Discard(_)));
            let mut clicked_save = false;
            if matches!(it.asset_type, 7 | 10) {
                ui.horizontal(|ui| {
                    clicked_save = ui
                        .push_id(("inventory_document_save", st.ui_id, id), |ui| {
                            ui.add_enabled(can_save, egui::Button::new(aurora_i18n::tr!("notecard-save-button")))
                        })
                        .inner
                        .clicked();
                    if st.save_pending {
                        ui.spinner();
                        ui.label(aurora_i18n::tr!("notecard-saving"));
                    } else if it.asset_type == 7
                        && let Some(Ok(text)) = &st.preview_text
                    {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("notecard-byte-count", bytes = text.len().to_string())).color(if valid_size {
                                p.muted
                            } else {
                                p.warn
                            }),
                        );
                    }
                });
            }
            if can_save
                && (clicked_save || edited.save)
                && let Some(Ok(text)) = &st.preview_text
            {
                actions.push(InvAction::SaveContent {
                    editor: st.ui_id,
                    item: it.id,
                    text: text.clone(),
                    close: false,
                });
            }
            if !valid_size && it.asset_type == 7 && st.preview_text.as_ref().is_some_and(Result::is_ok) {
                ui.label(RichText::new(aurora_i18n::tr!("notecard-text-too-large")).color(p.warn));
            }
            if !st.preview_message.is_empty() {
                let saved = st.preview_message == aurora_i18n::tr!("app-document-saved")
                    || st.preview_message == aurora_i18n::tr!("notecard-copy-complete");
                ui.label(RichText::new(&st.preview_message).color(if saved { p.success } else { p.warn }));
            }
        }
    });
    super::embedded::preview(ctx, p, st, images);
    if !open {
        if st.save_pending {
            st.close_after_save = true;
            return;
        }
        if st.preview_dirty {
            st.dialog = Some(EditDialog::Discard(id));
            return;
        }
        st.preview = None;
        st.reset_document();
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
                if st.animation_details {
                    "caret-up-filled"
                } else {
                    "caret-down-filled"
                },
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

    fn document_frame(
        ctx: &egui::Context,
        inv: &Inventory,
        st: &mut InventoryUi,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Vec<InvAction>) {
        let mut actions = Vec::new();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |_ui| {
                let p = crate::theme::Theme::default().palette();
                show(
                    ctx,
                    &p,
                    &Icons::default(),
                    inv,
                    st,
                    &InventoryPreferences::default(),
                    &Facts {
                        agent: Uuid::from_u128(2),
                        worn: Default::default(),
                        worn_labels: Default::default(),
                        points: Vec::new(),
                        appearance_busy: false,
                        active_gestures: Default::default(),
                        touchable: Default::default(),
                        names: Default::default(),
                    },
                    &Default::default(),
                    &mut actions,
                );
                preview(
                    ctx,
                    &p,
                    &Icons::default(),
                    inv,
                    inv,
                    st,
                    &Default::default(),
                    &InventoryPreferences::default(),
                    &mut actions,
                );
            },
        );
        (headless_output(output), actions)
    }

    fn note_fixture() -> (Inventory, InventoryUi) {
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let mut st = InventoryUi {
            preview: Some(Uuid::from_u128(8103)),
            ..Default::default()
        };
        st.load_document(7, true, &Ok(b"Avant".to_vec()));
        (inv, st)
    }

    #[test]
    fn embedded_copy_confirmation_uses_the_current_document_and_marker() {
        let ctx = egui::Context::default();
        let (inv, mut st) = note_fixture();
        st.load_document(7, true, &Ok(crate::world::inventory::demo::note_asset(true)));
        let note = st.preview.expect("note");
        for _ in 0..5 {
            document_frame(&ctx, &inv, &mut st, vec![]);
        }
        st.dialog = Some(EditDialog::CopyEmbedded(note, '\u{100002}'));
        for _ in 0..5 {
            document_frame(&ctx, &inv, &mut st, vec![]);
        }
        let rect = ctx
            .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::new(("notecard_copy_confirm", st.ui_id))))
            .expect("copy button");
        let event = |pressed| egui::Event::PointerButton {
            pos: rect.center(),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        document_frame(&ctx, &inv, &mut st, vec![egui::Event::PointerMoved(rect.center()), event(true)]);
        let (_, actions) = document_frame(&ctx, &inv, &mut st, vec![event(false)]);
        assert!(actions.iter().any(|action| matches!(action, InvAction::Embedded { editor, notecard, marker: '\u{100002}', copy: true } if *editor == st.ui_id && *notecard == note)));
        assert!(st.dialog.is_none());
    }

    fn label_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == label => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            })
            .expect("label")
    }

    fn click_document(ctx: &egui::Context, inv: &Inventory, st: &mut InventoryUi, pos: egui::Pos2) -> Vec<InvAction> {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        document_frame(ctx, inv, st, vec![egui::Event::PointerMoved(pos), button(true)]);
        document_frame(ctx, inv, st, vec![button(false)]).1
    }

    #[test]
    fn editing_and_ctrl_s_emit_the_current_draft_for_its_own_editor() {
        let ctx = egui::Context::default();
        let (inv, mut st) = note_fixture();
        for _ in 0..5 {
            document_frame(&ctx, &inv, &mut st, Vec::new());
        }
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(("inventory_document_text", st.ui_id, st.preview.expect("note")))));
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        };
        document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::End), egui::Event::Text("\nÉté 🐺".into())]);
        assert!(st.preview_dirty);
        let (_, actions) = document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::S)]);
        assert!(matches!(&actions[..], [InvAction::SaveContent { editor, item, text, close: false }]
            if *editor == st.ui_id && Some(*item) == st.preview && text == "Avant\nÉté 🐺"));
    }

    #[test]
    fn save_button_and_close_confirmation_preserve_the_draft_until_reply() {
        let ctx = egui::Context::default();
        let (inv, mut st) = note_fixture();
        st.preview_text = Some(Ok("Après".into()));
        st.document_changed();
        for _ in 0..5 {
            document_frame(&ctx, &inv, &mut st, Vec::new());
        }
        let (output, _) = document_frame(&ctx, &inv, &mut st, Vec::new());
        let pos = label_center(&output, aurora_i18n::tr!("notecard-save-button"));
        let actions = click_document(&ctx, &inv, &mut st, pos);
        assert!(matches!(&actions[..], [InvAction::SaveContent { text, close: false, .. }] if text == "Après"));
        st.dialog = Some(EditDialog::Discard(st.preview.expect("note")));
        for _ in 0..5 {
            document_frame(&ctx, &inv, &mut st, Vec::new());
        }
        let (output, _) = document_frame(&ctx, &inv, &mut st, Vec::new());
        let pos = label_center(&output, aurora_i18n::tr!("common-save"));
        let actions = click_document(&ctx, &inv, &mut st, pos);
        assert!(matches!(&actions[..], [InvAction::SaveContent { text, close: true, .. }] if text == "Après"));
        assert!(st.preview_dirty && st.preview.is_some());
    }

    #[test]
    fn discard_and_cancel_choose_whether_to_keep_the_draft() {
        for discard in [false, true] {
            let ctx = egui::Context::default();
            let (inv, mut st) = note_fixture();
            st.preview_text = Some(Ok("Après".into()));
            st.document_changed();
            for _ in 0..5 {
                document_frame(&ctx, &inv, &mut st, Vec::new());
            }
            st.dialog = Some(EditDialog::Discard(st.preview.expect("note")));
            for _ in 0..5 {
                document_frame(&ctx, &inv, &mut st, Vec::new());
            }
            let (output, _) = document_frame(&ctx, &inv, &mut st, Vec::new());
            let pos = label_center(
                &output,
                if discard {
                    aurora_i18n::tr!("ui-inventory-dialogs-discard-changes")
                } else {
                    aurora_i18n::tr!("common-cancel")
                },
            );
            assert!(click_document(&ctx, &inv, &mut st, pos).is_empty());
            assert_eq!(st.preview.is_none(), discard);
            assert_eq!(st.preview_dirty, !discard);
            assert!(st.dialog.is_none());
        }
    }

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
                            active_gestures: Default::default(),
                            touchable: Default::default(),
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
