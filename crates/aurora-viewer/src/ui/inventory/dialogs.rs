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
    if let EditDialog::Share(ids, resident) = &dialog {
        let picture = facts.pictures.get(resident);
        if !share(ctx, p, icons, inv, prefs, facts, picture, st.ui_id, ids, *resident, actions) {
            st.dialog = Some(dialog);
        }
        return;
    }
    if let EditDialog::ReplaceLinks(picker) = &mut dialog {
        if super::replace_links::show(ctx, p, icons, inv, st, prefs, facts, picker, actions) {
            st.dialog = Some(dialog);
        }
        return;
    }
    let mut open = true;
    let mut done = false;
    let title = match &dialog {
        EditDialog::Properties(..) => aurora_i18n::tr!("ui-inventory-dialogs-object-properties"),
        EditDialog::Delete(_, true) => aurora_i18n::tr!("ui-inventory-dialogs-purge-selection"),
        EditDialog::Delete(_, false) => aurora_i18n::tr!("ui-inventory-dialogs-delete-selection"),
        EditDialog::Empty(_) => aurora_i18n::tr!("common-empty-trash"),
        EditDialog::Ungroup(_) => aurora_i18n::tr!("ui-inventory-dialogs-ungroup-folder"),
        EditDialog::Discard(..) | EditDialog::SaveEmbedded(..) => aurora_i18n::tr!("ui-inventory-dialogs-save-changes"),
        EditDialog::Share(..) => aurora_i18n::tr!("ui-inventory-dialogs-share-selection"),
        EditDialog::ReplaceLinks(..) => aurora_i18n::tr!("common-replace-links"),
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
    let mut floater = if properties {
        super::properties::window(ctx, &window_id)
    } else {
        Floater::new(
            &window_id,
            title,
            ctx.content_rect().center()
                - if document_confirmation {
                    egui::vec2(220.0, 70.0)
                } else {
                    egui::vec2(210.0, 130.0)
                },
            if document_confirmation {
                Vec2::new(440.0, 140.0)
            } else {
                Vec2::new(420.0, 260.0)
            },
        )
    };
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
                        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("notecard_copy_confirm", st.ui_id)), button.rect));
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
            // windows of their own (`share`, `replace_links::show`)
            EditDialog::Share(..) | EditDialog::ReplaceLinks(..) => done = true,
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
                        let holds_worn = rules::worn_ancestors(inv, &facts.worn);
                        if ids.iter().all(|id| {
                            rules::movable(inv, *id, &prefs.protected) && rules::in_type(inv, *id, 14) && !holds_worn.contains(id)
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

/// Entries listed by the give confirmation; the rest are counted.
const SHARE_SHOWN: usize = 5;

/// What a give does with one of its entries.
#[derive(Clone, Copy, PartialEq)]
enum Fate {
    Given,
    /// Given, and gone from the inventory (CannotCopyWarning).
    NoCopy,
    /// Not given: no transfer permission (LLGiveInventory::
    /// isInventoryGiveAcceptable), or another reason `rules::offer` names.
    Excluded,
}

/// The confirmation of a give, from an inventory window or a drop on a
/// conversation (ShareItemsConfirmation, LLToolDragAndDrop's
/// show_object_sharing_confirmation and LLAvatarActions::shareWithAvatars,
/// indra/newview): to whom, then what. The entries that cannot be copied are
/// marked as they leave the inventory (CannotCopyWarning); those that cannot
/// be given at all (no transfer, worn, protected, a link…) are marked and
/// left out, as the Legacy Viewer leaves them out, the reason on hover. Then
/// Cancel / Give: Enter gives; Escape or a click beside it cancels. True
/// once answered.
#[allow(clippy::too_many_arguments)]
fn share(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    prefs: &InventoryPreferences,
    facts: &Facts,
    picture: Option<&egui::TextureHandle>,
    ui_id: Uuid,
    ids: &[Uuid],
    resident: Uuid,
    actions: &mut Vec<InvAction>,
) -> bool {
    let name = facts.names.get(&resident).map_or("…", String::as_str);
    // ((icon, tint), name, fate, tag, why)
    let rows: Vec<((&str, egui::Color32), String, Fate, &str, String)> = ids
        .iter()
        .map(|id| {
            let item = inv.items.get(id);
            let icon = item.map_or_else(
                || {
                    let kind = inv.folders.get(id).map_or(-1, |f| f.info.type_default);
                    (super::tree::folder_icon(kind, false), p.muted)
                },
                |it| {
                    let color = item_icon_color(it.asset_type, it.inv_type, p);
                    (item_icon(it.asset_type, it.inv_type, it.flags), color)
                },
            );
            let (fate, tag, why) = match rules::offer(inv, *id, &prefs.protected, &facts.worn) {
                Ok(bucket) if rules::offer_has_no_copy(inv, &bucket) => {
                    (Fate::NoCopy, aurora_i18n::tr!("ui-inventory-dialogs-no-copy"), String::new())
                }
                Ok(_) => (Fate::Given, "", String::new()),
                Err(why) if item.is_some_and(|it| it.owner_mask & rules::TRANSFER == 0) => {
                    (Fate::Excluded, aurora_i18n::tr!("ui-inventory-dialogs-no-transfer"), why)
                }
                Err(why) => (Fate::Excluded, aurora_i18n::tr!("ui-inventory-dialogs-left-out"), why),
            };
            (icon, rules::name(inv, *id).to_string(), fate, tag, why)
        })
        .collect();
    let count = |fate| rows.iter().filter(|row| row.2 == fate).count();
    let (no_copy, excluded) = (count(Fate::NoCopy), count(Fate::Excluded));
    let given: Vec<Uuid> = ids
        .iter()
        .zip(&rows)
        .filter(|(_, row)| row.2 != Fate::Excluded)
        .map(|(id, _)| *id)
        .collect();
    let mut give = false;
    let mut cancel = false;
    let modal = crate::ui::widgets::Modal::new(egui::Id::unique(("inventory_share", ui_id))).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
            crate::ui::widgets::avatar_disc(ui, p, rect, picture, name, false);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-inventory-dialogs-give-to"))
                        .size(12.0)
                        .color(p.muted),
                );
                ui.add(egui::Label::new(RichText::new(name).size(15.0).strong().color(p.ink)).truncate());
            });
        });
        ui.add_space(10.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        let tag_font = egui::FontId::proportional(11.5);
        for ((icon, tint), label, fate, tag, why) in rows.iter().take(SHARE_SHOWN) {
            let (ink, tag_color) = match fate {
                Fate::Given => (p.ink, p.muted),
                Fate::NoCopy => (p.ink, p.warn),
                Fate::Excluded => (p.muted_dim, p.muted),
            };
            ui.horizontal(|ui| {
                super::tree::icon(ui, icons, icon, p, Some(if *fate == Fate::Excluded { p.muted_dim } else { *tint }));
                let tag_w = if tag.is_empty() {
                    0.0
                } else {
                    ui.fonts_mut(|f| f.layout_no_wrap((*tag).to_owned(), tag_font.clone(), tag_color))
                        .size()
                        .x
                        + 10.0
                };
                ui.allocate_ui(Vec2::new((ui.available_width() - tag_w).max(40.0), 18.0), |ui| {
                    ui.add(egui::Label::new(RichText::new(label).color(ink)).truncate());
                });
                if !tag.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let tag = ui.label(RichText::new(*tag).font(tag_font.clone()).color(tag_color));
                        if !why.is_empty() {
                            tag.on_hover_text(why);
                        }
                    });
                }
            });
        }
        if rows.len() > SHARE_SHOWN {
            let more = rows.len() - SHARE_SHOWN;
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "ui-inventory-dialogs-and-more",
                    count = more,
                    number = aurora_i18n::integer(more as i64)
                ))
                .size(12.0)
                .color(p.muted),
            );
        }
        // the notes share one frame, the amber one filled, so that their
        // icons line up
        if excluded > 0 {
            ui.add_space(4.0);
            share_note(
                ui,
                "info-circle-filled",
                p.muted,
                None,
                aurora_i18n::tr!(
                    "ui-inventory-dialogs-not-given",
                    count = excluded,
                    number = aurora_i18n::integer(excluded as i64)
                ),
            );
        }
        if no_copy > 0 {
            ui.add_space(4.0);
            share_note(
                ui,
                "alert-triangle-filled",
                p.warn,
                Some(p.warn.gamma_multiply(0.12)),
                aurora_i18n::tr!(
                    "ui-inventory-dialogs-no-copy-will-leave",
                    count = no_copy,
                    number = aurora_i18n::integer(no_copy as i64)
                ),
            );
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // nothing left to give: Give stays greyed, the line above says why
            ui.add_enabled_ui(!given.is_empty(), |ui| {
                let primary =
                    crate::ui::widgets::primary_button_sized(ui, p, aurora_i18n::tr!("ui-inventory-dialogs-give"), Vec2::new(96.0, 28.0));
                ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("inventory_share_give", ui_id)), primary.rect));
                give = !given.is_empty() && (primary.clicked() || crate::ui::widgets::enter_pressed(ui));
            });
            cancel = crate::ui::widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(96.0, 28.0)).clicked();
        });
    });
    if give {
        actions.push(InvAction::Share { items: given, resident });
    }
    give || cancel || modal.should_close()
}

/// A note under the give confirmation's entries: its icon and text in
/// `color`, on `fill` when it warns.
fn share_note(ui: &mut egui::Ui, icon: &str, color: egui::Color32, fill: Option<egui::Color32>, text: String) {
    let mut frame = egui::Frame::new()
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(egui::Margin::symmetric(8, 6));
    if let Some(fill) = fill {
        frame = frame.fill(fill);
    }
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            crate::ui::widgets::glyph(ui, icon, color, 15.0);
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).wrap());
        });
    });
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
    prefs: &mut InventoryPreferences,
    script_settings: &crate::ui::scripts::settings::ScriptSettings,
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
    if !inv.can_preview(it.asset_type) {
        return;
    }
    let mut view = if it.asset_type == 7 {
        *st.notecard_view.get_or_insert(prefs.notecard_view)
    } else {
        Default::default()
    };
    let editable = st.preview_can_edit && crate::world::inventory::preview::can_edit(inv, id);
    if it.asset_type == 7
        && view != super::markdown::View::Markdown
        && !st.save_pending
        && let (Some(source), Some(Ok(text))) = (&mut st.preview_source, &st.preview_text)
    {
        super::embedded::clipboard(
            ctx,
            egui::Id::unique(("inventory_document_text", st.ui_id, id)),
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
        &super::document::preview_window_id(st.ui_id, id, it.asset_type),
        format!("{}{}", it.name, if st.preview_dirty { " *" } else { "" }),
        ctx.content_rect().center() - size * 0.5,
        size,
    )
    .cascade(crate::ui::widgets::CascadeGroup::inventory(it.asset_type))
    .show(ctx, p, &mut open, |ui| {
        if it.asset_type == 0 {
            st.wanted_images.insert(it.asset_id);
            if let Some(t) = images.get(&it.asset_id) {
                // LLPreview::{handleMouseDown,handleHover}: carry the actual
                // inventory item, so world drops still check its permissions.
                let response = ui.add(
                    egui::Image::new(t)
                        .max_size(Vec2::splat(350.0))
                        .fit_to_original_size(1.0)
                        .sense(egui::Sense::click_and_drag()),
                );
                if drag_inventory.items.get(&id).is_some_and(|item| item.asset_type == 0) {
                    response.dnd_set_drag_payload(InvDrag(id));
                }
                ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_texture_preview", st.ui_id, id)), response.rect));
            } else {
                ui.label(aurora_i18n::tr!("ui-inventory-dialogs-loading-image"));
            }
            ui.label(
                RichText::new(aurora_i18n::tr!("texture-library-drag-hint"))
                    .size(10.0)
                    .color(p.muted),
            );
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
            if it.asset_type == 7 {
                notecard_description(ui, inv, st, it, prefs, actions);
            }
            if it.asset_type == 7 && view.choose(ui, p) {
                st.notecard_view = Some(view);
                prefs.notecard_view = view;
                let editor = egui::Id::unique(("inventory_document_text", st.ui_id, id));
                ctx.memory_mut(|m| {
                    m.request_focus(if view == super::markdown::View::Markdown {
                        editor.with("markdown")
                    } else {
                        editor
                    })
                });
            }
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
                    // the Save row is as tall as its button: its text and
                    // padding once a large text size outgrows interact_size
                    let save_row = ui
                        .spacing()
                        .interact_size
                        .y
                        .max(ui.text_style_height(&egui::TextStyle::Button) + ui.spacing().button_padding.y * 2.0);
                    let footer_height = save_row
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
                    if it.asset_type == 7 {
                        edited = super::markdown::editor(
                            ui,
                            p,
                            icons,
                            egui::Id::unique(("inventory_document_text", st.ui_id, id)),
                            text,
                            st.preview_source.as_mut(),
                            editable && !st.save_pending && !matches!(st.dialog, Some(EditDialog::Discard(_))),
                            editor_size,
                            view,
                            &mut st.notecard_markdown,
                            script_settings,
                        );
                    } else {
                        egui::ScrollArea::both()
                            .id_salt(("inventory_document_scroll", st.ui_id, id, view as usize))
                            .max_height(if documents { editor_size.y } else { 300.0 })
                            .auto_shrink(if documents { [false, false] } else { [true, true] })
                            .show(ui, |ui| {
                                if it.asset_type == 10 {
                                    // beside the scroll bar's strip, when it shows
                                    let size = Vec2::new(ui.available_width().min(editor_size.x), editor_size.y);
                                    let editor_id = egui::Id::unique(("inventory_document_text", st.ui_id, id));
                                    edited = super::embedded::editor(
                                        ui,
                                        p,
                                        icons,
                                        editor_id,
                                        text,
                                        st.preview_source.as_ref(),
                                        editable && !st.save_pending && !matches!(st.dialog, Some(EditDialog::Discard(_))),
                                        size,
                                        super::embedded::Appearance::Source,
                                    );
                                } else {
                                    ui.add(egui::Label::new(RichText::new(text.as_str()).monospace()).selectable(true));
                                }
                            });
                    }
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

/// LLPreview::refreshFromItem / onCommit (indra/newview/llpreview.cpp):
/// descriptions are item metadata, independent of the notecard asset draft.
fn notecard_description(
    ui: &mut egui::Ui,
    inv: &Inventory,
    st: &mut InventoryUi,
    it: &aurora_net::inventory::InvItem,
    prefs: &InventoryPreferences,
    actions: &mut Vec<InvAction>,
) {
    let description = st.preview_description.get_or_insert_with(|| super::document::Description {
        item: it.id,
        original: it.desc.clone(),
        text: it.desc.clone(),
    });
    if description.item != it.id || description.text == description.original || description.text == it.desc {
        description.item = it.id;
        description.original.clone_from(&it.desc);
        description.text.clone_from(&it.desc);
    }
    let can_edit =
        rules::mutable(inv, it.id, &prefs.protected) && it.owner_mask & rules::MODIFY != 0 && st.pending.is_none() && !st.save_pending;
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("common-description"));
        let response = ui.add_enabled(
            can_edit,
            egui::TextEdit::singleline(&mut description.text)
                .id(egui::Id::unique(("notecard_description", st.ui_id, it.id)))
                .interactive(can_edit)
                .char_limit(127)
                .desired_width(ui.available_width()),
        );
        if can_edit
            && response.lost_focus()
            && description.text != it.desc
            && let Ok(change) = rules::patch(inv, it.id, llsd_map! { "desc" => description.text.clone() })
        {
            actions.push(InvAction::Edit(change));
        }
    });
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
    .cascade(crate::ui::widgets::CascadeGroup::Animation)
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
        document_frame_with_preferences(ctx, inv, st, &mut InventoryPreferences::default(), events)
    }

    fn document_frame_with_preferences(
        ctx: &egui::Context,
        inv: &Inventory,
        st: &mut InventoryUi,
        prefs: &mut InventoryPreferences,
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
                    prefs,
                    &Facts {
                        agent: Uuid::from_u128(2),
                        worn: Default::default(),
                        worn_labels: Default::default(),
                        worn_at: Default::default(),
                        points: Vec::new(),
                        appearance_busy: false,
                        active_gestures: Default::default(),
                        touchable: Default::default(),
                        names: Default::default(),
                        ..Default::default()
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
                    prefs,
                    &Default::default(),
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

    /// A notecard keeps its size with a message under its Save button.
    #[test]
    fn a_notecard_keeps_its_size_with_a_message() {
        let item = Uuid::from_u128(8103);
        let ui_id = Uuid::from_u128(77);
        let message = "The notecard could not be saved: the inventory did not answer in time, try again later.";
        crate::ui::widgets::steady_tests::assert_window_steady(
            &super::super::document::preview_window_id(ui_id, item, 7),
            &[egui::vec2(560.0, 520.0), egui::vec2(320.0, 300.0)],
            message,
            || {
                let (inv, mut st) = note_fixture();
                st.ui_id = ui_id;
                st.preview_message = message.into();
                (inv, st)
            },
            |(inv, st), ctx, p| {
                let mut actions = Vec::new();
                preview(
                    ctx,
                    p,
                    &Icons::default(),
                    inv,
                    inv,
                    st,
                    &Default::default(),
                    &mut InventoryPreferences::default(),
                    &Default::default(),
                    &mut actions,
                );
            },
        );
    }

    #[test]
    fn notecard_description_commits_metadata_in_every_view_without_saving_the_body() {
        use aurora_net::inventory::operations::Operation;
        for view in [NotecardView::Classic, NotecardView::Markdown, NotecardView::Edit] {
            let ctx = egui::Context::default();
            let (mut inv, mut st) = note_fixture();
            let item = st.preview.expect("note");
            inv.items.get_mut(&item).expect("item").desc = "Original description".into();
            st.notecard_view = Some(view);
            for _ in 0..5 {
                document_frame(&ctx, &inv, &mut st, vec![]);
            }
            let (output, _) = document_frame(&ctx, &inv, &mut st, vec![]);
            label_center(&output, aurora_i18n::tr!("common-description"));
            label_center(&output, "Original description");
            let field = egui::Id::unique(("notecard_description", st.ui_id, item));
            ctx.memory_mut(|memory| memory.request_focus(field));
            document_frame(&ctx, &inv, &mut st, vec![]);
            document_frame(&ctx, &inv, &mut st, vec![egui::Event::Text(" revised".into())]);
            let draft = st.preview_description.as_ref().expect("description").text.clone();
            assert_ne!(draft, "Original description");
            let (_, actions) = document_frame(
                &ctx,
                &inv,
                &mut st,
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
            );
            assert!(matches!(&actions[..], [InvAction::Edit(change)]
                if matches!(&change.operations[..], [Operation::Patch { id, folder: false, body }]
                    if *id == item && body["desc"].as_str() == draft && body.as_map().expect("patch").len() == 1)));
            assert!(!st.preview_dirty);
            assert!(!st.save_pending);
            assert_eq!(st.preview_text, Some(Ok("Avant".into())));
            document_frame(&ctx, &inv, &mut st, vec![]);
            assert_eq!(st.preview_description.as_ref().expect("pending draft").text, draft);
            inv.items.get_mut(&item).expect("item").desc = draft;
            document_frame(&ctx, &inv, &mut st, vec![]);
            inv.items.get_mut(&item).expect("item").desc = "Updated elsewhere".into();
            document_frame(&ctx, &inv, &mut st, vec![]);
            assert_eq!(
                st.preview_description.as_ref().expect("refreshed metadata").text,
                "Updated elsewhere"
            );
        }
    }

    #[test]
    fn notecard_description_uses_modify_rights_independently_of_content_copy_rights() {
        for (mask, protected, pending, allowed) in [
            (rules::COPY, false, false, false),
            (rules::MODIFY, false, false, true),
            (rules::COPY | rules::MODIFY, true, false, false),
            (rules::COPY | rules::MODIFY, false, true, false),
        ] {
            let ctx = egui::Context::default();
            let (mut inv, mut st) = note_fixture();
            let item = st.preview.expect("note");
            inv.items.get_mut(&item).expect("item").owner_mask = mask;
            inv.items.get_mut(&item).expect("item").desc = "Visible description".into();
            st.preview_can_edit = false;
            st.pending = pending.then(Uuid::new_v4);
            let mut prefs = InventoryPreferences::default();
            if protected {
                prefs.protected.insert(item);
            }
            for _ in 0..5 {
                document_frame_with_preferences(&ctx, &inv, &mut st, &mut prefs, vec![]);
            }
            let field = egui::Id::unique(("notecard_description", st.ui_id, item));
            ctx.memory_mut(|memory| memory.request_focus(field));
            document_frame_with_preferences(&ctx, &inv, &mut st, &mut prefs, vec![egui::Event::Text(" changed".into())]);
            let changed = st.preview_description.as_ref().expect("description").text != "Visible description";
            assert_eq!(changed, allowed, "mask={mask:#x}, protected={protected}, pending={pending}");
        }
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
            .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("notecard_copy_confirm", st.ui_id))))
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
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique(("inventory_document_text", st.ui_id, st.preview.expect("note")))));
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
        assert!(super::document_save_focused(&ctx), "native Ctrl+S belongs to this notecard");
        let (_, actions) = document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::S)]);
        assert!(matches!(&actions[..], [InvAction::SaveContent { editor, item, text, close: false }]
            if *editor == st.ui_id && Some(*item) == st.preview && text == "Avant\nÉté 🐺"));
        ctx.memory_mut(|m| m.surrender_focus(m.focused().expect("document focus")));
        assert!(
            !super::document_save_focused(&ctx),
            "the world shortcut works again outside the document"
        );
    }

    #[test]
    fn notecard_view_choices_seed_new_windows_without_overwriting_open_drafts() {
        use super::super::NotecardView;

        let ctx = egui::Context::default();
        let (inv, mut st) = note_fixture();
        let mut prefs = InventoryPreferences::default();
        for _ in 0..5 {
            document_frame_with_preferences(&ctx, &inv, &mut st, &mut prefs, vec![]);
        }
        assert_eq!(st.notecard_view, Some(NotecardView::Classic));
        let draft = "# Brouillon **Été 🐺**";
        st.preview_text = Some(Ok(draft.into()));
        st.document_changed();

        for (view, label) in [
            (NotecardView::Markdown, aurora_i18n::tr!("notecard-view-markdown")),
            (NotecardView::Classic, aurora_i18n::tr!("notecard-view-classic")),
            (NotecardView::Edit, aurora_i18n::tr!("notecard-view-source")),
        ] {
            let (output, _) = document_frame_with_preferences(&ctx, &inv, &mut st, &mut prefs, vec![]);
            let pos = label_center(&output, label);
            for pressed in [true, false] {
                document_frame_with_preferences(
                    &ctx,
                    &inv,
                    &mut st,
                    &mut prefs,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            assert_eq!(prefs.notecard_view, view, "the explicit tab choice updates the shared preference");
            assert_eq!(st.notecard_view, Some(view));
            assert_eq!(st.preview_text, Some(Ok(draft.into())));
            assert!(st.preview_dirty);

            // Both new inventory windows and object documents use this entry
            // point. No constructor-specific default may hide the preference.
            let fresh_ctx = egui::Context::default();
            let mut inventory = InventoryUi::default();
            let item = st.preview.expect("item");
            inventory.open_document(&fresh_ctx, item, 7);
            let new_note = &mut inventory.windows[0].state;
            new_note.load_document(7, true, &Ok(b"New note".to_vec()));
            let mut object_note = InventoryUi::document(item);
            object_note.load_document(7, true, &Ok(b"Object note".to_vec()));
            let mut output = fresh_ctx.run_ui(egui::RawInput::default(), |ui| {
                for note in [&mut **new_note, &mut object_note] {
                    show_document(
                        ui.ctx(),
                        &crate::theme::Theme::default().palette(),
                        &Icons::default(),
                        &inv,
                        note,
                        &inv,
                        &Default::default(),
                        &mut prefs,
                        &Default::default(),
                    );
                }
            });
            output.textures_delta.clear();
            assert_eq!(new_note.notecard_view, Some(view));
            assert_eq!(object_note.notecard_view, Some(view));
            // Painting an older window must not turn its local mode into a
            // new global choice. Only a user tab change does that.
            prefs.notecard_view = if view == NotecardView::Edit {
                NotecardView::Classic
            } else {
                NotecardView::Edit
            };
            let last_choice = prefs.notecard_view;
            document_frame_with_preferences(&ctx, &inv, &mut st, &mut prefs, vec![]);
            assert_eq!(prefs.notecard_view, last_choice);
            assert_eq!(st.notecard_view, Some(view));
            assert_eq!(st.preview_text, Some(Ok(draft.into())));

            object_note.reset_document();
            document_frame_with_preferences(&fresh_ctx, &inv, &mut object_note, &mut prefs, vec![]);
            assert_eq!(object_note.notecard_view, Some(last_choice));
        }
    }

    #[test]
    fn notecard_view_switches_save_the_source_and_attachments_and_keep_failed_drafts() {
        use super::super::markdown::View;
        use aurora_net::inventory::notecard::Notecard;

        for (view, label) in [
            (View::Classic, aurora_i18n::tr!("notecard-view-classic")),
            (View::Edit, aurora_i18n::tr!("notecard-view-source")),
            (View::Markdown, aurora_i18n::tr!("notecard-view-markdown")),
        ] {
            let ctx = egui::Context::default();
            let (inv, mut st) = note_fixture();
            st.load_document(7, true, &Ok(crate::world::inventory::demo::note_asset(true)));
            let original = st.preview_source.clone().expect("note");
            let draft = format!("# Été 🐺\n\n**Source** *unchanged*\n\n{}", original.text);
            st.notecard_view = Some(View::Edit);
            for _ in 0..5 {
                document_frame(&ctx, &inv, &mut st, vec![]);
            }
            let raw_editor = egui::Id::unique(("inventory_document_text", st.ui_id, st.preview.expect("item")));
            ctx.memory_mut(|m| m.request_focus(raw_editor));
            document_frame(
                &ctx,
                &inv,
                &mut st,
                vec![
                    egui::Event::Key {
                        key: if cfg!(target_os = "macos") {
                            egui::Key::ArrowUp
                        } else {
                            egui::Key::Home
                        },
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers {
                            ctrl: !cfg!(target_os = "macos"),
                            mac_cmd: cfg!(target_os = "macos"),
                            command: true,
                            ..Default::default()
                        },
                    },
                    egui::Event::Text("# Été 🐺\n\n**Source** *unchanged*\n\n".into()),
                ],
            );
            assert_eq!(st.preview_text, Some(Ok(draft.clone())));
            let (output, _) = document_frame(&ctx, &inv, &mut st, vec![]);
            let pos = label_center(&output, label);
            click_document(&ctx, &inv, &mut st, pos);
            assert_eq!(st.notecard_view, Some(view));
            let editor = egui::Id::unique(("inventory_document_text", st.ui_id, st.preview.expect("item")));
            let editor = if view == View::Markdown { editor.with("markdown") } else { editor };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                // The previous preview stays visible while parsing the edited
                // draft. Its cached rectangle alone cannot prove that the next
                // click will hit the attachment's current position.
                let current = view == View::Classic
                    || st
                        .notecard_markdown
                        .get(&ctx, &draft)
                        .is_some_and(|rendered| rendered.source == draft);
                document_frame(&ctx, &inv, &mut st, vec![]);
                if current && ctx.data_mut(|data| data.get_temp::<egui::Rect>(editor.with("demo_rect"))).is_some() {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "{view:?}: current Markdown preview did not finish"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_eq!(st.preview_text, Some(Ok(draft.clone())));
            assert!(st.preview_dirty);
            let attachment = ctx
                .data_mut(|data| data.get_temp::<egui::Rect>(editor.with(("demo_attachment", '\u{100000}'))))
                .expect("attachment in every view");
            let actions = click_document(&ctx, &inv, &mut st, attachment.center());
            assert!(
                matches!(
                    &actions[..],
                    [InvAction::Embedded {
                        marker: '\u{100000}',
                        copy: false,
                        ..
                    }]
                ),
                "{view:?}: attachment click at {attachment:?} emitted {} actions",
                actions.len()
            );
            // Markdown uses its own immutable buffer and selection indices;
            // saving while focused there must still emit the complete source.
            ctx.memory_mut(|m| m.request_focus(editor));
            let (_, actions) = document_frame(
                &ctx,
                &inv,
                &mut st,
                vec![egui::Event::Key {
                    key: egui::Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        ctrl: true,
                        command: true,
                        ..Default::default()
                    },
                }],
            );
            assert!(matches!(&actions[..], [InvAction::SaveContent { text, close: false, .. }] if text == &draft));
            let (output, _) = document_frame(&ctx, &inv, &mut st, vec![]);
            let save = label_center(&output, aurora_i18n::tr!("notecard-save-button"));
            assert!(matches!(&click_document(&ctx, &inv, &mut st, save)[..], [InvAction::SaveContent { text, .. }] if text == &draft));
            let request = Uuid::new_v4();
            let data = st.start_content_save(request, 7, draft.clone(), false).expect("save source");
            let saved = Notecard::decode(&data).expect("Linden notecard");
            assert_eq!(saved.text, draft);
            for marker in original.text.chars().filter(|c| original.attachment(*c).is_some()) {
                assert_eq!(
                    saved.attachment(marker).expect("retained attachment").item.id,
                    original.attachment(marker).expect("original").item.id
                );
            }
            assert!(st.finish_content_save(request, Some("offline failure")));
            assert_eq!(st.notecard_view, Some(view));
            assert_eq!(st.preview_text, Some(Ok(draft.clone())));
            assert!(st.preview_dirty && !st.save_pending);
            let request = Uuid::new_v4();
            st.start_content_save(request, 7, draft, false).expect("retry");
            assert!(st.finish_content_save(request, None));
            assert_eq!(st.notecard_view, Some(view));
            assert!(!st.preview_dirty);
        }
    }

    #[test]
    fn markdown_views_keep_attachments_on_separate_source_lines() {
        use super::super::markdown::View;
        for view in [View::Markdown, View::Edit] {
            let ctx = egui::Context::default();
            let (inv, mut st) = note_fixture();
            st.load_document(7, true, &Ok(crate::world::inventory::demo::note_asset(true)));
            st.notecard_view = Some(view);
            let original = st.preview_text.clone();
            let preview = egui::Id::unique(("inventory_document_text", st.ui_id, st.preview.expect("item"))).with("markdown");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                document_frame(&ctx, &inv, &mut st, vec![]);
                let rects = ctx.data_mut(|data| {
                    ['\u{100000}', '\u{100001}', '\u{100002}']
                        .map(|marker| data.get_temp::<egui::Rect>(preview.with(("demo_attachment", marker))))
                });
                if let [Some(first), Some(second), Some(third)] = rects {
                    assert!(second.top() >= first.bottom(), "second attachment must start on its own row");
                    assert!(third.top() >= second.bottom(), "third attachment must start on its own row");
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "attachment preview did not finish");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_eq!(st.preview_text, original);
            assert!(!st.preview_dirty);
        }
    }

    #[test]
    fn notecard_lsl_keyboard_preserves_source_history_and_saved_attachments() {
        use egui::text::{CCursor, CCursorRange};
        let ctx = egui::Context::default();
        let (inv, mut st) = note_fixture();
        let card = aurora_net::inventory::notecard::Notecard::decode(&crate::world::inventory::demo::note_asset(true)).expect("card");
        let original = "Prose \"{ Été 🐺\n\n```lsl\ndefault\n{\n    state_entry()\n    {\n    }\n}\n```\n\u{100000}\n\u{100001}";
        st.load_document(7, true, &Ok(card.encode(original).expect("asset")));
        st.notecard_view = Some(NotecardView::Edit);
        let editor = egui::Id::unique(("inventory_document_text", st.ui_id, st.preview.expect("item")));
        for _ in 0..5 {
            document_frame(&ctx, &inv, &mut st, vec![]);
        }
        let at = original.find("    {").expect("event opener") + 5;
        let caret = original[..at].chars().count();
        let mut state = egui::TextEdit::load_state(&ctx, editor).expect("source editor");
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
        state.store(&ctx, editor);
        ctx.memory_mut(|m| m.request_focus(editor));
        document_frame(&ctx, &inv, &mut st, vec![]);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let command = "llOwnerSay(\"Été 🐺\");";
        document_frame(
            &ctx,
            &inv,
            &mut st,
            vec![
                key(egui::Key::Enter, egui::Modifiers::NONE),
                key(egui::Key::Tab, egui::Modifiers::NONE),
                egui::Event::Text(command.into()),
            ],
        );
        let mut expected = original.to_owned();
        expected.insert_str(at, &format!("\n            {command}"));
        assert_eq!(st.preview_text, Some(Ok(expected.clone())));
        assert!(st.preview_dirty && ctx.memory(|m| m.has_focus(editor)));
        document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::Tab, egui::Modifiers::SHIFT)]);
        expected = expected.replace(&format!("            {command}"), &format!("        {command}"));
        assert_eq!(st.preview_text, Some(Ok(expected.clone())));
        for modifiers in [egui::Modifiers::COMMAND, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT] {
            document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::K, modifiers)]);
            assert_ne!(st.preview_text, Some(Ok(expected.clone())));
            document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::K, modifiers)]);
            assert_eq!(st.preview_text, Some(Ok(expected.clone())));
        }
        let line_start = expected.find("        llOwnerSay").expect("inserted line");
        let line_end = line_start + expected[line_start..].find('\n').expect("next line") + 1;
        let mut state = egui::TextEdit::load_state(&ctx, editor).expect("editor");
        state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(expected[..line_end].chars().count()),
            CCursor::new(expected[..line_start].chars().count()),
        )));
        state.store(&ctx, editor);
        document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
        assert_ne!(st.preview_text, Some(Ok(expected.clone())));
        document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::Tab, egui::Modifiers::SHIFT)]);
        assert_eq!(st.preview_text, Some(Ok(expected.clone())));
        document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::Z, egui::Modifiers::COMMAND)]);
        assert_ne!(st.preview_text, Some(Ok(expected.clone())));
        document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::Y, egui::Modifiers::COMMAND)]);
        assert_eq!(st.preview_text, Some(Ok(expected.clone())));
        let (_, actions) = document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::S, egui::Modifiers::COMMAND)]);
        assert!(matches!(&actions[..], [InvAction::SaveContent {text, ..}] if text == &expected));
        let bytes = st.start_content_save(Uuid::new_v4(), 7, expected.clone(), false).expect("save");
        let saved = aurora_net::inventory::notecard::Notecard::decode(&bytes).expect("saved notecard");
        assert_eq!(saved.text, expected);
        for marker in ['\u{100000}', '\u{100001}'] {
            assert_eq!(
                saved.attachment(marker).expect("saved item").item.id,
                card.attachment(marker).expect("original item").item.id
            );
        }
    }

    #[test]
    fn notecard_lsl_braces_tabs_comments_and_paste_keep_their_scopes() {
        use egui::text::{CCursor, CCursorRange};
        for (before, event, after) in [
            (
                "Text \"{\n\n```lsl\ndefault\n{\n        ",
                egui::Event::Text("}".into()),
                "Text \"{\n\n```lsl\ndefault\n{\n}",
            ),
            (
                "Text \"{\n\n```lsl\ndefault\n{\n        ",
                egui::Event::Paste("}\n\t// exact 🐺".into()),
                "Text \"{\n\n```lsl\ndefault\n{\n        }\n\t// exact 🐺",
            ),
            ("Prose {\n    ", egui::Event::Text("}".into()), "Prose {\n    }"),
            (
                "```lsl\n    ",
                egui::Event::Key {
                    key: egui::Key::Tab,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                "```lsl\n        ",
            ),
            (
                "```lsl\n        ",
                egui::Event::Key {
                    key: egui::Key::Backspace,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                "```lsl\n    ",
            ),
            (
                "Prose \"{\n```lsl\n/*\n  comment 🐺\n*/\n```",
                egui::Event::Key {
                    key: egui::Key::K,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                },
                "Prose \"{\n```lsl\n\n  comment 🐺\n\n```",
            ),
        ] {
            let ctx = egui::Context::default();
            let (inv, mut st) = note_fixture();
            st.load_document(7, true, &Ok(before.as_bytes().to_vec()));
            st.notecard_view = Some(NotecardView::Edit);
            for _ in 0..5 {
                document_frame(&ctx, &inv, &mut st, vec![]);
            }
            let id = egui::Id::unique(("inventory_document_text", st.ui_id, st.preview.expect("item")));
            let mut state = egui::TextEdit::load_state(&ctx, id).expect("editor");
            let caret = if matches!(&event, egui::Event::Key { key: egui::Key::K, .. }) {
                before[..before.find("comment").expect("comment") + 3].chars().count()
            } else {
                before.chars().count()
            };
            state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
            state.store(&ctx, id);
            ctx.memory_mut(|m| m.request_focus(id));
            document_frame(&ctx, &inv, &mut st, vec![]);
            document_frame(&ctx, &inv, &mut st, vec![event]);
            assert_eq!(st.preview_text, Some(Ok(after.into())));
        }
    }

    #[test]
    fn notecard_toolbar_preserves_unicode_selection_history_and_saved_attachments() {
        use egui::text::{CCursor, CCursorRange};
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let (inv, mut st) = note_fixture();
        let card = aurora_net::inventory::notecard::Notecard::decode(&crate::world::inventory::demo::note_asset(true)).expect("card");
        let original = "Été 🐺\n\u{100000}\n\u{100001}\n\u{100002}";
        st.load_document(7, true, &Ok(card.encode(original).expect("asset")));
        st.notecard_view = Some(super::super::markdown::View::Edit);
        let editor = egui::Id::unique(("inventory_document_text", st.ui_id, st.preview.expect("item")));
        for _ in 0..5 {
            document_frame(&ctx, &inv, &mut st, vec![]);
        }
        ctx.memory_mut(|memory| memory.request_focus(editor));
        document_frame(&ctx, &inv, &mut st, vec![]);
        let mut state = egui::TextEdit::load_state(&ctx, editor).expect("editor");
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(5), CCursor::new(0))));
        state.store(&ctx, editor);
        ctx.memory_mut(|memory| memory.request_focus(editor));
        let (output, _) = document_frame(&ctx, &inv, &mut st, vec![]);
        let button = label_center(&output, aurora_i18n::tr!("notecard-format-bold"));
        click_document(&ctx, &inv, &mut st, button);
        let draft = "**Été 🐺**\n\u{100000}\n\u{100001}\n\u{100002}";
        assert_eq!(st.preview_text, Some(Ok(draft.into())));
        assert!(st.preview_dirty);
        assert!(ctx.memory(|memory| memory.has_focus(editor)));
        let range = egui::TextEdit::load_state(&ctx, editor)
            .expect("editor")
            .cursor
            .char_range()
            .expect("selection");
        assert_eq!(range.primary.index.0, 2);
        assert_eq!(range.secondary.index.0, 7);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while egui::TextEdit::load_state(&ctx, editor.with("markdown")).is_none() {
            document_frame(&ctx, &inv, &mut st, vec![]);
            assert!(std::time::Instant::now() < deadline, "preview did not finish");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        ctx.memory_mut(|memory| memory.request_focus(editor.with("markdown")));
        let (output, _) = document_frame(&ctx, &inv, &mut st, vec![]);
        click_document(&ctx, &inv, &mut st, label_center(&output, aurora_i18n::tr!("notecard-format-bold")));
        assert_eq!(
            st.preview_text,
            Some(Ok(draft.into())),
            "preview selection must never format a stale source selection"
        );
        ctx.memory_mut(|memory| memory.request_focus(editor));
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
        document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::Z)]);
        assert_eq!(st.preview_text, Some(Ok(original.into())));
        assert!(!st.preview_dirty);
        document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::Y)]);
        assert_eq!(st.preview_text, Some(Ok(draft.into())));
        let (_, actions) = document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::S)]);
        let [InvAction::SaveContent { text, .. }] = &actions[..] else {
            panic!("save current source")
        };
        assert_eq!(text, draft);
        let saved = st.preview_source.as_ref().expect("source").encode(text).expect("encoded");
        let saved = aurora_net::inventory::notecard::Notecard::decode(&saved).expect("saved card");
        assert_eq!(saved.text, draft);
        for marker in ['\u{100000}', '\u{100001}', '\u{100002}'] {
            assert_eq!(
                saved.attachment(marker).expect("saved attachment").item.id,
                card.attachment(marker).expect("attachment").item.id
            );
        }
    }

    #[test]
    fn notecard_toolbar_is_disabled_for_read_only_and_pending_saves_but_help_is_available() {
        for pending in [false, true] {
            let ctx = egui::Context::default();
            let (inv, mut st) = note_fixture();
            st.load_document(7, pending, &Ok(b"Source".to_vec()));
            st.save_pending = pending;
            st.notecard_view = Some(super::super::markdown::View::Edit);
            for _ in 0..5 {
                document_frame(&ctx, &inv, &mut st, vec![]);
            }
            let (output, _) = document_frame(&ctx, &inv, &mut st, vec![]);
            click_document(&ctx, &inv, &mut st, label_center(&output, aurora_i18n::tr!("notecard-format-bold")));
            assert_eq!(st.preview_text, Some(Ok("Source".into())));
            assert!(!st.preview_dirty);
            let (output, _) = document_frame(&ctx, &inv, &mut st, vec![]);
            click_document(
                &ctx,
                &inv,
                &mut st,
                label_center(&output, aurora_i18n::tr!("notecard-markdown-help")),
            );
            let (output, _) = document_frame(&ctx, &inv, &mut st, vec![]);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.text() == aurora_i18n::tr!("notecard-markdown-help-extra"))));
        }
    }

    #[test]
    fn notecard_split_edit_geometry_stays_stable() {
        for scale in [1.0, 1.125, 1.25, 1.5, 2.0] {
            for long in [false, true] {
                let ctx = egui::Context::default();
                crate::theme::Theme::default().apply(&ctx, 1.0);
                ctx.set_pixels_per_point(scale);
                let (inv, mut st) = note_fixture();
                st.load_document(7, true, &Ok(crate::world::inventory::demo::note_asset(true)));
                let text = st.preview_text.as_mut().expect("loaded").as_mut().expect("text");
                text.insert_str(0, "# Heading\n\n**Bold** and `code`.\n\n");
                if long {
                    text.push_str(&"\nMore text".repeat(100));
                }
                st.notecard_view = Some(super::super::markdown::View::Edit);
                let editor = egui::Id::unique(("inventory_document_text", st.ui_id, st.preview.expect("item")));
                let panes = [editor, editor.with("markdown")];
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while st.notecard_markdown.get(&ctx, text).is_none() {
                    assert!(std::time::Instant::now() < deadline, "preview did not finish");
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                let draft = st.preview_text.clone();
                for focus in panes {
                    ctx.memory_mut(|memory| memory.request_focus(focus));
                    let mut sizes = [Vec::new(), Vec::new()];
                    for frame in 0..240 {
                        document_frame(&ctx, &inv, &mut st, vec![]);
                        if frame >= 120 {
                            for (pane, sizes) in panes.into_iter().zip(&mut sizes) {
                                sizes.push(
                                    ctx.data_mut(|data| data.get_temp::<egui::Rect>(pane.with("demo_rect")))
                                        .expect("pane")
                                        .size(),
                                );
                            }
                        }
                    }
                    for sizes in sizes {
                        let min = sizes.iter().copied().reduce(egui::Vec2::min).expect("frames");
                        let max = sizes.iter().copied().reduce(egui::Vec2::max).expect("frames");
                        assert!(
                            (max - min).length() < 0.1,
                            "pane oscillates at scale {scale}, long={long}: {min:?}..{max:?}"
                        );
                    }
                    if !long {
                        for pane in panes {
                            let (viewport, content) = ctx
                                .data_mut(|data| data.get_temp::<(egui::Rect, egui::Vec2)>(pane.with("test_scroll_geometry")))
                                .expect("scroll area");
                            assert!(
                                content.x <= viewport.width().ceil() && content.y <= viewport.height().ceil(),
                                "short note overflows at {scale}: {viewport:?}, {content:?}"
                            );
                        }
                    }
                }
                assert_eq!(st.preview_text, draft);
                assert!(!st.preview_dirty);
            }
        }
    }

    #[test]
    fn notecard_split_edit_refreshes_preview_and_saves_from_either_pane() {
        let ctx = egui::Context::default();
        let (inv, mut st) = note_fixture();
        st.load_document(7, true, &Ok(b"# Before\n\nBody".to_vec()));
        st.notecard_view = Some(super::super::markdown::View::Edit);
        let editor = egui::Id::unique(("inventory_document_text", st.ui_id, st.preview.expect("item")));
        let preview = editor.with("markdown");
        let wait_for = |st: &mut InventoryUi, expected: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let (output, _) = document_frame(&ctx, &inv, st, vec![]);
                if output.shapes.iter().any(|shape| {
                    matches!(&shape.shape,
                    egui::epaint::Shape::Text(text) if text.galley.text() == expected)
                }) {
                    return;
                }
                assert!(std::time::Instant::now() < deadline, "live preview did not refresh");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        wait_for(&mut st, "Before\n\nBody\n\n");
        let rect = |id: egui::Id| {
            ctx.data_mut(|data| data.get_temp::<egui::Rect>(id.with("demo_rect")))
                .expect("pane")
        };
        let left = rect(editor);
        let right = rect(preview);
        assert!(left.right() <= right.left(), "source and preview must be side by side");
        assert!((left.top() - right.top()).abs() < 1.0);
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
        ctx.memory_mut(|memory| memory.request_focus(editor));
        document_frame(
            &ctx,
            &inv,
            &mut st,
            vec![key(egui::Key::End), egui::Event::Text("\n\n## Été 🐺\n\n**Nouveau**".into())],
        );
        let draft = "# Before\n\nBody\n\n## Été 🐺\n\n**Nouveau**";
        assert_eq!(st.preview_text, Some(Ok(draft.into())));
        assert!(st.preview_dirty);
        wait_for(&mut st, "Before\n\nBody\n\nÉté 🐺\n\nNouveau\n\n");
        ctx.memory_mut(|memory| memory.request_focus(preview));
        document_frame(&ctx, &inv, &mut st, vec![egui::Event::Text("read-only preview".into())]);
        assert_eq!(st.preview_text, Some(Ok(draft.into())));
        for pane in [editor, preview] {
            ctx.memory_mut(|memory| memory.request_focus(pane));
            let (_, actions) = document_frame(&ctx, &inv, &mut st, vec![key(egui::Key::S)]);
            assert!(matches!(&actions[..], [InvAction::SaveContent { text, .. }] if text == draft));
        }
    }

    #[test]
    fn notecard_views_never_enable_writing_or_saving_a_read_only_document() {
        use super::super::markdown::View;

        for view in [View::Classic, View::Edit, View::Markdown] {
            let ctx = egui::Context::default();
            let (inv, mut st) = note_fixture();
            st.load_document(7, false, &Ok(b"# Read only\n**Source**".to_vec()));
            st.notecard_view = Some(view);
            let original = st.preview_text.clone();
            for _ in 0..8 {
                document_frame(&ctx, &inv, &mut st, vec![]);
            }
            let editor = egui::Id::unique(("inventory_document_text", st.ui_id, st.preview.expect("item")));
            ctx.memory_mut(|m| m.request_focus(if view == View::Markdown { editor.with("markdown") } else { editor }));
            let (_, actions) = document_frame(&ctx, &inv, &mut st, vec![egui::Event::Text("forbidden".into())]);
            assert!(actions.is_empty());
            assert_eq!(st.preview_text, original);
            assert!(!st.preview_dirty);
            assert!(st.start_content_save(Uuid::new_v4(), 7, "forbidden".into(), false).is_err());
        }
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
                            worn_at: Default::default(),
                            points: Vec::new(),
                            appearance_busy: false,
                            active_gestures: Default::default(),
                            touchable: Default::default(),
                            names: Default::default(),
                            ..Default::default()
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

    /// The give confirmation (#627): the resident by name, the entries with
    /// the one that cannot be copied marked and counted, Give gives, Escape
    /// gives nothing.
    #[test]
    fn the_give_confirmation_names_the_resident_and_marks_no_copy_entries() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let agent = Uuid::from_u128(2);
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, agent);
        let resident = Uuid::from_u128(77);
        let worn = HashSet::new();
        let mut ids: Vec<Uuid> = inv
            .items
            .values()
            .filter(|it| it.owner_mask & rules::COPY != 0 && rules::offer(&inv, it.id, &HashSet::new(), &worn).is_ok())
            .map(|it| it.id)
            .collect();
        ids.sort();
        ids.truncate(2);
        assert_eq!(ids.len(), 2, "two givable demo items");
        inv.items.get_mut(&ids[1]).expect("item").owner_mask &= !rules::COPY;
        let mut facts = Facts {
            agent,
            ..Default::default()
        };
        facts.names.insert(resident, "Nova Exemple".into());
        let frame = |st: &mut InventoryUi, events: Vec<egui::Event>| {
            let mut actions = Vec::new();
            let output = headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |_ui| {
                    show(
                        &ctx,
                        &p,
                        &Icons::default(),
                        &inv,
                        st,
                        &InventoryPreferences::default(),
                        &facts,
                        &Default::default(),
                        &mut actions,
                    )
                },
            ));
            let texts: Vec<String> = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
            (texts, actions)
        };
        // and the demo's object without transfer permission
        let no_transfer = Uuid::from_u128(8110);
        assert_eq!(inv.items[&no_transfer].owner_mask & rules::TRANSFER, 0);
        let mut asked = ids.clone();
        asked.push(no_transfer);
        let mut st = InventoryUi {
            dialog: Some(EditDialog::Share(asked.clone(), resident)),
            ..Default::default()
        };
        let mut texts = Vec::new();
        for _ in 0..4 {
            texts = frame(&mut st, Vec::new()).0;
        }
        let shown = |s: &str| texts.iter().any(|t| t == s);
        assert!(shown("Nova Exemple"), "the resident by name: {texts:?}");
        assert!(!texts.iter().any(|t| t.contains(&resident.to_string())), "never the resident's id");
        for id in &ids {
            assert!(shown(&rules::name(&inv, *id)), "each entry listed");
        }
        assert!(shown(aurora_i18n::tr!("ui-inventory-dialogs-no-copy")), "the no-copy entry marked");
        assert!(shown(&aurora_i18n::tr!(
            "ui-inventory-dialogs-no-copy-will-leave",
            count = 1usize,
            number = "1"
        )));
        assert!(
            shown(aurora_i18n::tr!("ui-inventory-dialogs-no-transfer")),
            "the no-transfer entry marked"
        );
        assert!(shown(&aurora_i18n::tr!(
            "ui-inventory-dialogs-not-given",
            count = 1usize,
            number = "1"
        )));
        // Give
        let give = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("inventory_share_give", st.ui_id))))
            .expect("give button")
            .center();
        let button = |pressed| egui::Event::PointerButton {
            pos: give,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut st, vec![egui::Event::PointerMoved(give), button(true)]);
        let (_, actions) = frame(&mut st, vec![button(false)]);
        assert!(
            matches!(&actions[..], [InvAction::Share { items, resident: to }] if *items == ids && *to == resident),
            "Give gives all but the entry left out"
        );
        assert!(st.dialog.is_none());
        // nothing that can be given: Enter gives nothing
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        st.dialog = Some(EditDialog::Share(vec![no_transfer], resident));
        for _ in 0..3 {
            frame(&mut st, Vec::new());
        }
        let (_, actions) = frame(&mut st, vec![enter]);
        assert!(actions.is_empty() && st.dialog.is_some(), "nothing to give");
        // Escape
        st.dialog = Some(EditDialog::Share(ids.clone(), resident));
        for _ in 0..3 {
            frame(&mut st, Vec::new());
        }
        let (_, actions) = frame(
            &mut st,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        frame(&mut st, Vec::new());
        assert!(actions.is_empty() && st.dialog.is_none(), "Escape gives nothing");
    }
}
