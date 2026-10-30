//! LLOutfitGalleryContextMenu / LLOutfitContextMenu (Firestorm
//! lloutfitgallery.cpp, lloutfitslist.cpp, menu_gallery_outfit_tab.xml, LGPL 2.1).

use super::{Action, AppearanceUi, model};
use crate::{
    theme::Palette,
    ui::{menu, texture_picker::TexturePicker, widgets},
    world::{World, inventory::Inventory},
};
use aurora_net::outfits::categories::Update;
use egui::RichText;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct State {
    rename: Option<(Uuid, String)>,
    focus: bool,
    confirm: Option<(Uuid, bool)>,
    picker: TexturePicker,
    image_for: Option<Uuid>,
}

/// What LLOutfitContextMenu::onEnable / onVisible allow on an outfit folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MenuRules {
    pub replace: bool,
    pub add: bool,
    pub remove: bool,
    /// Image, favorites and rename: the folder is the user's own outfit.
    pub folder: bool,
    pub favorite: bool,
    /// "Edit outfit" shows on the current outfit only.
    pub current: bool,
    pub save: bool,
    /// `None` hides "Delete outfit" and its separator (getCanRemoveOutfit).
    pub delete: Option<bool>,
}

pub(super) fn menu_rules(inv: &Inventory, worn: &HashSet<Uuid>, busy: bool, id: Uuid) -> Option<MenuRules> {
    let folder = inv.folders.get(&id)?;
    let owned = !folder.library && folder.info.type_default == model::FT_OUTFIT;
    let idle = !busy && owned;
    let current = model::base(inv) == Some(id);
    let ready = idle && model::complete(inv, id) && model::cof(inv).is_some_and(|cof| model::complete(inv, cof));
    let links = model::folder_links(inv, id);
    let nonempty = links.iter().any(|l| !l.folder);
    // LLFindWearablesEx skips body parts: they are swapped, never taken off
    let can_add = links.iter().any(|l| !l.folder && !worn.contains(&l.target));
    let can_remove = links
        .iter()
        .any(|l| worn.contains(&l.target) && inv.items.get(&l.target).is_some_and(|it| it.asset_type != 13));
    let removable = owned && !current && model::system_folder(inv, 14).is_some();
    Some(MenuRules {
        replace: ready && nonempty && (!current || model::dirty(inv)),
        add: ready && can_add,
        remove: ready && can_remove,
        folder: idle,
        favorite: folder.info.favorite,
        current,
        save: ready,
        delete: removable.then_some(ready),
    })
}

/// The outfit menu of the gallery tiles and of the list headers; only the
/// gallery adds the New clothing / New body part submenus.
pub(super) fn context_menu(
    response: &egui::Response,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    id: Uuid,
    actions: &mut Vec<Action>,
    new_parts: bool,
) {
    menu::context_menu(response, p, |ui| {
        let inv = &world.inventory;
        let Some(folder) = inv.folders.get(&id) else { return };
        let worn = super::items::worn_items(world);
        let Some(rules) = menu_rules(inv, &worn, st.pending.is_some(), id) else {
            return;
        };
        let outfits = st.shown_outfits(inv);
        let selection = st.outfit_selection.ordered(&outfits);
        let multiple = selection.len() > 1;
        let all_rules: Option<Vec<_>> = selection
            .iter()
            .map(|id| menu_rules(inv, &worn, st.pending.is_some(), *id))
            .collect();
        let ready = all_rules.as_ref().is_some_and(|rules| rules.iter().all(|rule| rule.folder))
            && selection.iter().all(|id| model::complete(inv, *id));
        let enabled = |single, test: fn(&MenuRules) -> bool| {
            if multiple {
                ready && all_rules.as_ref().is_some_and(|rules| rules.iter().any(test))
            } else {
                single
            }
        };
        if menu::item_if(
            ui,
            p,
            "shirt-filled",
            aurora_i18n::tr!("ui-appearance-gallery-replace-your-outfit"),
            enabled(rules.replace, |rule| rule.replace),
        ) {
            actions.push(st.wear_selection(&outfits, false));
        }
        if menu::item_if(
            ui,
            p,
            "plus",
            aurora_i18n::tr!("ui-appearance-gallery-add-to-your-outfit"),
            enabled(rules.add, |rule| rule.add),
        ) {
            actions.push(st.wear_selection(&outfits, true));
        }
        if menu::item_if(
            ui,
            p,
            "minus",
            aurora_i18n::tr!("ui-appearance-gallery-take-off"),
            enabled(rules.remove, |rule| rule.remove),
        ) {
            actions.push(if multiple {
                Action::RemoveOutfits(selection.clone())
            } else {
                Action::RemoveOutfit(id)
            });
        }
        if multiple {
            if ready {
                menu::separator(ui, p);
                super::folders::move_menu(ui, p, inv, st, &selection, actions);
            }
            return;
        }
        menu::separator(ui, p);
        if menu::item_if(ui, p, "photo-filled", aurora_i18n::tr!("common-image-2"), rules.folder) {
            st.gallery.picker.open_with_defaults(
                aurora_i18n::tr!("ui-appearance-gallery-outfit-picture"),
                folder.info.thumbnail,
                None,
                true,
            );
            st.gallery.image_for = Some(id);
        }
        if menu::item_if(
            ui,
            p,
            "star-filled",
            if rules.favorite {
                aurora_i18n::tr!("ui-appearance-gallery-remove-from-favorites")
            } else {
                aurora_i18n::tr!("common-add-to-favorites")
            },
            rules.folder,
        ) {
            actions.push(Action::Category(id, Update::Favorite(!rules.favorite)));
        }
        if rules.current
            && menu::item_if(
                ui,
                p,
                "edit-filled",
                aurora_i18n::tr!("ui-appearance-gallery-edit-your-outfit"),
                rules.folder,
            )
        {
            st.open(0, true);
        }
        if menu::item_if(
            ui,
            p,
            "pencil-filled",
            aurora_i18n::tr!("ui-appearance-gallery-rename-your-outfit"),
            rules.folder,
        ) {
            st.gallery.rename = Some((id, folder.info.name.clone()));
            st.gallery.focus = true;
        }
        if menu::item_if(
            ui,
            p,
            "device-floppy-filled",
            aurora_i18n::tr!("ui-appearance-gallery-save-to-this-outfit"),
            rules.save,
        ) {
            st.gallery.confirm = Some((id, false));
        }
        // an Aurora addition, with the groups of My Outfits (#1002)
        if rules.folder {
            super::folders::move_menu(ui, p, inv, st, &[id], actions);
        }
        if let Some(enabled) = rules.delete {
            menu::separator(ui, p);
            if menu::item_if(
                ui,
                p,
                "trash-filled",
                aurora_i18n::tr!("ui-appearance-gallery-delete-outfit"),
                enabled,
            ) {
                st.gallery.confirm = Some((id, true));
            }
        }
        if !new_parts {
            return;
        }
        menu::separator(ui, p);
        menu::submenu(
            ui,
            p,
            "shirt-filled",
            aurora_i18n::tr!("ui-appearance-gallery-new-clothing"),
            true,
            |ui| {
                for label in [
                    aurora_i18n::tr!("common-new-shirt"),
                    aurora_i18n::tr!("ui-appearance-gallery-new-pants"),
                    aurora_i18n::tr!("common-new-shoes"),
                    aurora_i18n::tr!("common-new-socks"),
                    aurora_i18n::tr!("common-new-jacket"),
                    aurora_i18n::tr!("common-new-skirt"),
                    aurora_i18n::tr!("common-new-gloves"),
                    aurora_i18n::tr!("ui-appearance-gallery-new-t-shirt"),
                    aurora_i18n::tr!("ui-appearance-gallery-new-underwear"),
                    aurora_i18n::tr!("ui-appearance-gallery-new-alpha-layer"),
                ] {
                    menu::todo(ui, p, "shirt-filled", label);
                }
                // created, worn, then edited (wear_and_edit_cb)
                if menu::item_if(
                    ui,
                    p,
                    "shirt-filled",
                    aurora_i18n::tr!("ui-appearance-gallery-new-physics"),
                    st.pending.is_none(),
                ) {
                    crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::NewPhysics);
                }
                menu::todo(ui, p, "shirt-filled", aurora_i18n::tr!("common-new-tattoo"));
            },
        );
        menu::submenu(
            ui,
            p,
            "man-filled",
            aurora_i18n::tr!("ui-appearance-gallery-new-body-part"),
            true,
            |ui| {
                for label in [
                    aurora_i18n::tr!("common-new-shape"),
                    aurora_i18n::tr!("common-new-skin"),
                    aurora_i18n::tr!("common-new-hair"),
                    aurora_i18n::tr!("common-new-eyes"),
                ] {
                    menu::todo(ui, p, "man-filled", label);
                }
            },
        );
    });
}

pub(super) fn dialogs(
    ctx: &egui::Context,
    p: &Palette,
    world: &mut World,
    st: &mut AppearanceUi,
    images: &HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<Action>,
) {
    if let Some((id, mut name)) = st.gallery.rename.clone() {
        let mut close = false;
        let response = crate::ui::widgets::Modal::new(egui::Id::unique("outfit_rename")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-appearance-gallery-rename-your-outfit"))
                    .size(17.0)
                    .color(p.ink),
            );
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(&mut name).char_limit(63).desired_width(f32::INFINITY));
            if st.gallery.focus {
                r.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ctx, r.id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(name.chars().count()),
                    )));
                    state.store(ctx, r.id);
                }
                st.gallery.focus = false;
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let valid = !name.trim().is_empty() && st.pending.is_none();
                use widgets::{ActionKind, action_button, action_button_if};
                let size = egui::Vec2::new(0.0, 26.0);
                if action_button_if(ui, p, "check", aurora_i18n::tr!("common-ok"), ActionKind::Primary, size, valid).clicked()
                    || (valid && crate::ui::widgets::enter_pressed(ui))
                {
                    actions.push(Action::Category(id, Update::Rename(name.trim().into())));
                    close = true;
                }
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    close = true;
                }
            });
        });
        st.gallery.rename = if close || response.should_close() { None } else { Some((id, name)) };
    }
    if let Some((id, delete)) = st.gallery.confirm {
        let mut close = false;
        let name = world.inventory.folders.get(&id).map(|f| f.info.name.as_str()).unwrap_or("");
        let response = crate::ui::widgets::Modal::new(egui::Id::unique("outfit_confirm")).show(ctx, |ui| {
            ui.set_width(350.0);
            ui.label(
                RichText::new(if delete {
                    aurora_i18n::tr!("ui-appearance-gallery-delete-outfit")
                } else {
                    aurora_i18n::tr!("ui-appearance-gallery-save-to-this-outfit")
                })
                .size(17.0)
                .color(p.ink),
            );
            ui.add_space(8.0);
            ui.label(if delete {
                aurora_i18n::tr!("ui-appearance-gallery-move-to-trash", name = name.to_string())
            } else {
                aurora_i18n::tr!(
                    "ui-appearance-gallery-replace-contents-with-what-you-are-currently-wearing",
                    name = name.to_string()
                )
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                use widgets::{ActionKind, action_button, action_button_if};
                let size = egui::Vec2::new(0.0, 26.0);
                // moving an outfit to the trash is the destructive answer
                let (icon, label, kind) = if delete {
                    ("trash-filled", aurora_i18n::tr!("common-delete"), ActionKind::Danger)
                } else {
                    ("device-floppy-filled", aurora_i18n::tr!("common-save"), ActionKind::Primary)
                };
                if action_button_if(ui, p, icon, label, kind, size, st.pending.is_none()).clicked() {
                    if delete {
                        if let Some(trash) = model::system_folder(&world.inventory, 14) {
                            actions.push(Action::Category(id, Update::Trash(trash)));
                        }
                    } else {
                        actions.push(Action::SaveTo(id));
                    }
                    close = true;
                }
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    close = true;
                }
            });
        });
        if close || response.should_close() {
            st.gallery.confirm = None;
        }
    }
    if let Some(image) = st.gallery.picker.show(ctx, p, world, images)
        && let Some(id) = st.gallery.image_for.take()
    {
        actions.push(Action::Category(id, Update::Thumbnail(image)));
    }
    if !st.gallery.picker.open {
        st.gallery.image_for = None;
    }
    st.wanted_images.extend(st.gallery.picker.wanted_images.drain());
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, epaint::Shape};

    fn frame(
        ctx: &egui::Context,
        world: &mut World,
        st: &mut AppearanceUi,
        events: Vec<Event>,
    ) -> (Vec<Action>, Vec<egui::epaint::ClippedShape>) {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                actions = super::super::show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    world,
                    st,
                    &mut true,
                    41_748,
                    &HashMap::new(),
                );
            },
        );
        // Headless tests do not submit texture uploads to a renderer.
        output.textures_delta.clear();
        (actions, output.shapes)
    }

    fn text_position(shape: &Shape, label: &str) -> Option<Pos2> {
        match shape {
            Shape::Text(text) if text.galley.text() == label => Some(text.pos + egui::vec2(3.0, 5.0)),
            Shape::Vec(shapes) => shapes.iter().find_map(|s| text_position(s, label)),
            _ => None,
        }
    }

    fn click(ctx: &egui::Context, world: &mut World, st: &mut AppearanceUi, pos: Pos2, button: PointerButton) -> Vec<Action> {
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(
                frame(
                    ctx,
                    world,
                    st,
                    vec![
                        Event::PointerMoved(pos),
                        Event::PointerButton {
                            pos,
                            button,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                )
                .0,
            );
        }
        actions
    }

    #[test]
    fn gallery_menu_lists_missing_options_and_favorites_the_clicked_outfit() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = crate::ui::icons::Icons::load(&ctx, None);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
        let mut st = AppearanceUi::default();
        st.outfit_selection.single(Uuid::from_u128(702));
        for _ in 0..4 {
            frame(&ctx, &mut world, &mut st, vec![]);
        }
        let label = world.inventory.folders[&Uuid::from_u128(703)].info.name.clone();
        let shapes = frame(&ctx, &mut world, &mut st, vec![]).1;
        let pos = shapes.iter().find_map(|s| text_position(&s.shape, &label)).expect("outfit tile") + egui::vec2(10.0, -50.0);
        assert!(click(&ctx, &mut world, &mut st, pos, PointerButton::Secondary).is_empty());
        assert_eq!(st.outfit_selection.focus, Some(Uuid::from_u128(703)));
        let mut shapes = Vec::new();
        for _ in 0..4 {
            shapes = frame(&ctx, &mut world, &mut st, vec![]).1;
        }
        for label in [
            aurora_i18n::tr!("ui-appearance-gallery-replace-your-outfit"),
            aurora_i18n::tr!("ui-appearance-gallery-add-to-your-outfit"),
            aurora_i18n::tr!("ui-appearance-gallery-take-off"),
            aurora_i18n::tr!("common-image-2"),
            aurora_i18n::tr!("common-add-to-favorites"),
            aurora_i18n::tr!("ui-appearance-gallery-rename-your-outfit"),
            aurora_i18n::tr!("ui-appearance-gallery-save-to-this-outfit"),
            aurora_i18n::tr!("ui-appearance-gallery-delete-outfit"),
            aurora_i18n::tr!("ui-appearance-gallery-new-clothing"),
            aurora_i18n::tr!("ui-appearance-gallery-new-body-part"),
        ] {
            assert!(shapes.iter().any(|s| text_position(&s.shape, label).is_some()), "missing {label}");
        }
        let pos = shapes
            .iter()
            .find_map(|s| text_position(&s.shape, aurora_i18n::tr!("common-add-to-favorites")))
            .expect("favorite option");
        let actions = click(&ctx, &mut world, &mut st, pos, PointerButton::Primary);
        assert!(
            matches!(&actions[..], [Action::Category(id, Update::Favorite(true))] if *id == Uuid::from_u128(703)),
            "{actions:?}"
        );
        assert!(
            !world.inventory.folders[&Uuid::from_u128(703)].info.favorite,
            "no optimistic update"
        );
    }

    #[test]
    fn overwrite_and_delete_wait_for_confirmation_and_can_be_cancelled() {
        for delete in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
            let mut st = AppearanceUi::default();
            st.gallery.confirm = Some((Uuid::from_u128(703), delete));
            for _ in 0..4 {
                assert!(frame(&ctx, &mut world, &mut st, vec![]).0.is_empty());
            }
            let (actions, _) = frame(
                &ctx,
                &mut world,
                &mut st,
                vec![Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
            );
            assert!(actions.is_empty());
            assert!(st.gallery.confirm.is_none());
        }
    }

    fn demo_inventory() -> (Inventory, HashSet<Uuid>) {
        let mut inv = Inventory::default();
        model::seed_demo(&mut inv, Uuid::from_u128(1));
        let cof = model::cof(&inv).expect("current outfit");
        let worn = model::folder_links(&inv, cof)
            .into_iter()
            .filter(|l| !l.folder)
            .map(|l| l.target)
            .collect();
        (inv, worn)
    }

    #[test]
    fn outfit_menu_follows_the_legacy_enable_and_visible_rules() {
        let (mut inv, mut worn) = demo_inventory();
        let rules = |inv: &Inventory, worn: &HashSet<Uuid>, busy, id| menu_rules(inv, worn, busy, Uuid::from_u128(id)).expect("outfit");
        // an empty outfit: nothing to wear or take off, but it can be deleted
        let empty = rules(&inv, &worn, false, 702);
        assert!(!empty.replace && !empty.add && !empty.remove);
        assert!(empty.folder && empty.save && !empty.current);
        assert_eq!(empty.delete, Some(true));
        // everything of it is worn: no add, take off allowed
        let worn_outfit = rules(&inv, &worn, false, 703);
        assert!(worn_outfit.replace && !worn_outfit.add && worn_outfit.remove);
        // an object of it is not worn
        assert!(rules(&inv, &worn, false, 704).add);
        // the current outfit: Edit shows, Delete and its separator are hidden
        let current = rules(&inv, &worn, false, 705);
        assert!(current.current);
        assert_eq!(current.delete, None);
        assert!(current.replace, "the worn items differ from the saved outfit");
        // only body parts of it worn: they cannot be taken off
        worn.retain(|id| inv.items.get(id).is_some_and(|it| it.asset_type == 13));
        let parts = rules(&inv, &worn, false, 703);
        assert!(!parts.remove && parts.add);
        // an outfit change in progress greys everything out, Delete stays visible
        let busy = rules(&inv, &worn, true, 703);
        assert!(!busy.replace && !busy.add && !busy.remove && !busy.folder && !busy.save);
        assert_eq!(busy.delete, Some(false));
        // the favorite entry names the folder's state
        inv.folders.get_mut(&Uuid::from_u128(703)).expect("outfit").info.favorite = true;
        assert!(rules(&inv, &worn, false, 703).favorite);
        // a Library outfit is not ours: no rename, image, favorite or delete
        inv.folders.get_mut(&Uuid::from_u128(703)).expect("outfit").library = true;
        let library = rules(&inv, &worn, false, 703);
        assert!(!library.folder && !library.replace && !library.save);
        assert_eq!(library.delete, None);
        assert!(menu_rules(&inv, &worn, false, Uuid::from_u128(9999)).is_none());
    }

    #[test]
    fn list_header_menu_has_the_outfit_entries_without_new_parts() {
        for (id, current) in [(704, false), (705, true)] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let _icons = crate::ui::icons::Icons::load(&ctx, None);
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
            let mut st = AppearanceUi::default();
            st.open(1, false);
            for _ in 0..4 {
                frame(&ctx, &mut world, &mut st, vec![]);
            }
            let label = world.inventory.folders[&Uuid::from_u128(id)].info.name.clone();
            let shapes = frame(&ctx, &mut world, &mut st, vec![]).1;
            // the current outfit's name also heads the window: the header is the last one
            let pos = shapes
                .iter()
                .rev()
                .find_map(|s| text_position(&s.shape, &label))
                .expect("outfit header");
            assert!(click(&ctx, &mut world, &mut st, pos, PointerButton::Secondary).is_empty());
            assert_eq!(st.outfit_selection.focus, Some(Uuid::from_u128(id)));
            let mut shapes = Vec::new();
            for _ in 0..4 {
                shapes = frame(&ctx, &mut world, &mut st, vec![]).1;
            }
            let shown = |label: &str| shapes.iter().any(|s| text_position(&s.shape, label).is_some());
            for label in [
                aurora_i18n::tr!("ui-appearance-gallery-replace-your-outfit"),
                aurora_i18n::tr!("ui-appearance-gallery-add-to-your-outfit"),
                aurora_i18n::tr!("ui-appearance-gallery-take-off"),
                aurora_i18n::tr!("common-image-2"),
                aurora_i18n::tr!("common-add-to-favorites"),
                aurora_i18n::tr!("ui-appearance-gallery-rename-your-outfit"),
                aurora_i18n::tr!("ui-appearance-gallery-save-to-this-outfit"),
            ] {
                assert!(shown(label), "missing {label}");
            }
            assert_eq!(shown(aurora_i18n::tr!("ui-appearance-gallery-delete-outfit")), !current);
            assert_eq!(shown(aurora_i18n::tr!("ui-appearance-gallery-edit-your-outfit")), current);
            assert!(!shown(aurora_i18n::tr!("ui-appearance-gallery-new-clothing")));
            assert!(!shown(aurora_i18n::tr!("ui-appearance-gallery-new-body-part")));
            let pos = shapes
                .iter()
                .find_map(|s| text_position(&s.shape, aurora_i18n::tr!("ui-appearance-gallery-take-off")))
                .expect("take off");
            let actions = click(&ctx, &mut world, &mut st, pos, PointerButton::Primary);
            assert!(
                matches!(&actions[..], [Action::RemoveOutfit(o)] if *o == Uuid::from_u128(id)),
                "{actions:?}"
            );
        }
    }
}
