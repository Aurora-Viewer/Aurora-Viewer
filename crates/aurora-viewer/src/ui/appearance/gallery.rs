//! LLOutfitGalleryContextMenu / LLOutfitContextMenu (Firestorm
//! lloutfitgallery.cpp, lloutfitslist.cpp, menu_gallery_outfit_tab.xml, LGPL 2.1).

use super::{Action, AppearanceUi, model};
use crate::{
    theme::Palette,
    ui::{menu, texture_picker::TexturePicker, widgets},
    world::World,
};
use aurora_net::outfits::categories::Update;
use egui::RichText;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct State {
    rename: Option<(Uuid, String)>,
    focus: bool,
    confirm: Option<(Uuid, bool)>,
    picker: TexturePicker,
    image_for: Option<Uuid>,
}

pub(super) fn context_menu(
    response: &egui::Response,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    id: Uuid,
    actions: &mut Vec<Action>,
) {
    menu::context_menu(response, p, |ui| {
        let inv = &world.inventory;
        let Some(folder) = inv.folders.get(&id) else { return };
        let owned = !folder.library && folder.info.type_default == model::FT_OUTFIT;
        let idle = st.pending.is_none() && owned;
        let current = model::base(inv) == Some(id);
        let ready = model::complete(inv, id) && model::cof(inv).is_some_and(|cof| model::complete(inv, cof));
        let worn = super::items::worn_items(world);
        let links = model::folder_links(inv, id);
        let nonempty = links.iter().any(|l| !l.folder);
        if menu::item_if(
            ui,
            p,
            "t-shirt",
            "Remplacer votre tenue",
            idle && ready && nonempty && (!current || model::dirty(inv)),
        ) {
            actions.push(Action::Wear(id, false));
        }
        let can_add = links.iter().any(|l| !l.folder && !worn.contains(&l.target));
        if menu::item_if(ui, p, "plus", "Ajouter à votre tenue", idle && ready && can_add) {
            actions.push(Action::Wear(id, true));
        }
        let can_remove = links
            .iter()
            .any(|l| worn.contains(&l.target) && inv.items.get(&l.target).is_some_and(|it| it.asset_type != 13));
        if menu::item_if(ui, p, "minus", "Retirer de votre tenue", idle && ready && can_remove) {
            actions.push(Action::RemoveOutfit(id));
        }
        menu::separator(ui, p);
        if menu::item_if(ui, p, "image", "Image…", idle) {
            st.gallery
                .picker
                .open_with_defaults("Image de la tenue", folder.info.thumbnail, None, true);
            st.gallery.image_for = Some(id);
        }
        if menu::item_if(
            ui,
            p,
            "star",
            if folder.info.favorite {
                "Retirer des favoris"
            } else {
                "Ajouter aux favoris"
            },
            idle,
        ) {
            actions.push(Action::Category(id, Update::Favorite(!folder.info.favorite)));
        }
        if current && menu::item_if(ui, p, "wrench", "Modifier votre tenue", idle) {
            st.open(0, true);
        }
        if menu::item_if(ui, p, "pencil-simple", "Renommer votre tenue", idle) {
            st.gallery.rename = Some((id, folder.info.name.clone()));
            st.gallery.focus = true;
        }
        if menu::item_if(ui, p, "floppy-disk", "Enregistrer dans cette tenue", idle && ready) {
            st.gallery.confirm = Some((id, false));
        }
        menu::separator(ui, p);
        if menu::item_if(
            ui,
            p,
            "trash",
            "Supprimer la tenue",
            idle && ready && !current && model::system_folder(inv, 14).is_some(),
        ) {
            st.gallery.confirm = Some((id, true));
        }
        menu::separator(ui, p);
        menu::submenu(ui, p, "t-shirt", "Nouveau vêtement", true, |ui| {
            for label in [
                "Nouvelle chemise",
                "Nouveaux pantalons",
                "Nouvelles chaussures",
                "Nouvelles chaussettes",
                "Nouvelle veste",
                "Nouvelle jupe",
                "Nouveaux gants",
                "Nouveau T-shirt",
                "Nouveau sous-vêtement",
                "Nouvelle couche alpha",
                "Nouveau physique",
                "Nouveau tatouage",
            ] {
                menu::todo(ui, p, "t-shirt", label);
            }
        });
        menu::submenu(ui, p, "person", "Nouvelle partie du corps", true, |ui| {
            for label in ["Nouvelle silhouette", "Nouvelle peau", "Nouveaux cheveux", "Nouveaux yeux"] {
                menu::todo(ui, p, "person", label);
            }
        });
    });
}

pub(super) fn dialogs(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    images: &HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<Action>,
) {
    if let Some((id, mut name)) = st.gallery.rename.clone() {
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("outfit_rename")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(RichText::new("Renommer votre tenue").size(17.0).color(p.ink));
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
                if ui.add_enabled(valid, egui::Button::new("OK").fill(p.violet)).clicked()
                    || (valid && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    actions.push(Action::Category(id, Update::Rename(name.trim().into())));
                    close = true;
                }
                if widgets::flat_button(ui, p, "Annuler").clicked() {
                    close = true;
                }
            });
        });
        st.gallery.rename = if close || response.should_close() { None } else { Some((id, name)) };
    }
    if let Some((id, delete)) = st.gallery.confirm {
        let mut close = false;
        let name = world.inventory.folders.get(&id).map(|f| f.info.name.as_str()).unwrap_or("");
        let response = egui::Modal::new(egui::Id::new("outfit_confirm")).show(ctx, |ui| {
            ui.set_width(350.0);
            ui.label(
                RichText::new(if delete {
                    "Supprimer la tenue"
                } else {
                    "Enregistrer dans cette tenue"
                })
                .size(17.0)
                .color(p.ink),
            );
            ui.add_space(8.0);
            ui.label(if delete {
                format!("Déplacer « {name} » dans la corbeille ?")
            } else {
                format!("Remplacer le contenu de « {name} » par ce que vous portez actuellement ?")
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        st.pending.is_none(),
                        egui::Button::new(if delete { "Supprimer" } else { "Enregistrer" }).fill(p.violet),
                    )
                    .clicked()
                {
                    if delete {
                        if let Some(trash) = model::system_folder(&world.inventory, 14) {
                            actions.push(Action::Category(id, Update::Trash(trash)));
                        }
                    } else {
                        actions.push(Action::SaveTo(id));
                    }
                    close = true;
                }
                if widgets::flat_button(ui, p, "Annuler").clicked() {
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
        let mut st = AppearanceUi {
            selected_outfit: Some(Uuid::from_u128(702)),
            ..Default::default()
        };
        for _ in 0..4 {
            frame(&ctx, &mut world, &mut st, vec![]);
        }
        assert!(click(&ctx, &mut world, &mut st, egui::pos2(295.0, 300.0), PointerButton::Secondary).is_empty());
        assert_eq!(st.selected_outfit, Some(Uuid::from_u128(703)));
        let mut shapes = Vec::new();
        for _ in 0..4 {
            shapes = frame(&ctx, &mut world, &mut st, vec![]).1;
        }
        for label in [
            "Remplacer votre tenue",
            "Ajouter à votre tenue",
            "Retirer de votre tenue",
            "Image…",
            "Ajouter aux favoris",
            "Renommer votre tenue",
            "Enregistrer dans cette tenue",
            "Supprimer la tenue",
            "Nouveau vêtement",
            "Nouvelle partie du corps",
        ] {
            assert!(shapes.iter().any(|s| text_position(&s.shape, label).is_some()), "missing {label}");
        }
        let pos = shapes
            .iter()
            .find_map(|s| text_position(&s.shape, "Ajouter aux favoris"))
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
}
