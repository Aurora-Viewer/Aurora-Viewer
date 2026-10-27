//! Read-only baked texture inspection after LLFloaterAvatarTextures::refresh
//! (indra/newview/llfloateravatartextures.cpp, originally LGPL 2.1).
//! Match its masked controls: no asset picker, export or full UUID disclosure.
use crate::{scene::avatar, theme::Palette, ui::widgets, world::World};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Default)]
pub struct AvatarTexturesUi {
    pub(crate) avatar: Option<Uuid>,
    pub(crate) wanted_images: HashSet<Uuid>,
}

fn rows(world: &World, id: Uuid) -> Vec<(&'static str, Option<Uuid>)> {
    let index = world.objects.index_of_uuid(&id);
    let valid = index.map_or_else(Vec::new, |index| avatar::refreshable_bakes(|slot| world.bake_texture(index, slot)));
    avatar::BAKES
        .iter()
        .map(|(slot, name, _)| {
            let texture = index
                .and_then(|index| world.bake_texture(index, *slot))
                .filter(|id| valid.contains(id));
            (*name, texture)
        })
        .collect()
}

fn label(slot: &str) -> &'static str {
    aurora_i18n::text(match slot {
        "head" => "picker-bom-head",
        "upper" => "picker-bom-upper",
        "lower" => "picker-bom-lower",
        "eyes" => "picker-bom-eyes",
        "skirt" => "picker-bom-skirt",
        "hair" => "picker-bom-hair",
        "leftarm" => "picker-bom-leftarm",
        "leftleg" => "picker-bom-leftleg",
        "aux1" => "picker-bom-aux1",
        "aux2" => "picker-bom-aux2",
        "aux3" => "picker-bom-aux3",
        _ => "scene-menu-baked-textures",
    })
}

impl AvatarTexturesUi {
    pub(crate) fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World, images: &HashMap<Uuid, egui::TextureHandle>) {
        let Some(avatar) = self.avatar else { return };
        let mut open = true;
        widgets::Floater::new(
            "avatar_textures",
            aurora_i18n::tr!("ui-context-show-textures"),
            egui::pos2(240.0, 120.0),
            egui::vec2(530.0, 620.0),
        )
        .show(ctx, p, &mut open, |ui| {
            ui.label(aurora_i18n::tr!("scene-menu-baked-textures"));
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("avatar_baked_texture_grid")
                    .num_columns(3)
                    .spacing(egui::vec2(12.0, 12.0))
                    .show(ui, |ui| {
                        for (i, (slot, texture)) in rows(world, avatar).into_iter().enumerate() {
                            ui.vertical(|ui| {
                                ui.label(label(slot));
                                ui.allocate_ui(egui::vec2(144.0, 144.0), |ui| {
                                    if let Some(texture) = texture {
                                        self.wanted_images.insert(texture);
                                        if let Some(image) = images.get(&texture) {
                                            ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(144.0, 144.0)));
                                        } else {
                                            ui.label(aurora_i18n::tr!("common-loading"));
                                        }
                                    } else {
                                        ui.label(aurora_i18n::tr!("common-none"));
                                    }
                                });
                                if let Some(texture) = texture {
                                    ui.label(egui::RichText::new(format!("{}…", &texture.to_string()[..7])).small());
                                }
                            });
                            if i % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
            });
        });
        if !open {
            self.avatar = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_avatar_keeps_all_bake_slots_empty_without_requesting_magic_assets() {
        let world = World::new(std::sync::Arc::new(avatar::AvatarLibrary::load()));
        let rows = rows(&world, Uuid::from_u128(1));
        assert_eq!(rows.len(), avatar::BAKES.len());
        assert!(rows.iter().all(|(_, texture)| texture.is_none()));
    }
}
