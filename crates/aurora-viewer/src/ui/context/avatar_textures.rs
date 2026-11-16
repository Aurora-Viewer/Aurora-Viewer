//! Read-only baked texture inspection after LLFloaterAvatarTextures::refresh
//! (indra/newview/llfloateravatartextures.cpp, originally LGPL 2.1).
//! Match its masked controls: no asset picker, export or full UUID disclosure,
//! and every picture small, greyed and crossed out, ours as well as anyone
//! else's (LLFloaterAvatarTextures::postBuild masks and disables every
//! control, without an exception for the agent).
use crate::{scene::avatar, theme::Palette, ui::widgets, world::World};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The picture's size in LLTextureCtrl::draw: the 92×103 `texture_picker`
/// of floater_avatar_textures.xml, less its 1 px border and the 23 px
/// caption (BTN_HEIGHT_SMALL) under it. A larger preview would hand a
/// creator's texture out at a resolution the Legacy Viewer never shows.
const PREVIEW: egui::Vec2 = egui::vec2(90.0, 78.0);

/// The mask LLTextureCtrl::draw lays over a masked control's picture
/// (mIsMasked): a grey veil, LLColor4(0.5, 0.5, 0.5, 0.44), then the black
/// cross of gl_draw_x from corner to corner.
fn mask_shapes(rect: egui::Rect) -> [egui::Shape; 3] {
    // theme-guard: content — the Legacy Viewer's protection veil, LLColor4(0.5, 0.5, 0.5, 0.44)
    let veil = egui::Color32::from_rgba_unmultiplied(128, 128, 128, 112);
    // theme-guard: content — gl_draw_x's black cross over a protected picture
    let cross = egui::Stroke::new(1.0, egui::Color32::BLACK);
    [
        egui::Shape::rect_filled(rect, 0.0, veil),
        egui::Shape::line_segment([rect.left_top(), rect.right_bottom()], cross),
        egui::Shape::line_segment([rect.right_top(), rect.left_bottom()], cross),
    ]
}

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
            egui::vec2(340.0, 640.0),
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
                                ui.allocate_ui(PREVIEW, |ui| {
                                    if let Some(texture) = texture {
                                        self.wanted_images.insert(texture);
                                        if let Some(image) = images.get(&texture) {
                                            let shown = ui.add(egui::Image::new(image).fit_to_exact_size(PREVIEW));
                                            ui.painter().extend(mask_shapes(shown.rect));
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

    #[test]
    fn previews_keep_the_legacy_size() {
        // texture_picker 92×103 less the border (2) and the caption (23)
        assert_eq!(PREVIEW, egui::vec2(92.0 - 2.0, 103.0 - 2.0 - 23.0));
    }

    #[test]
    fn the_mask_veils_the_whole_picture_and_crosses_it_corner_to_corner() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), PREVIEW);
        let [veil, first, second] = mask_shapes(rect);
        let egui::Shape::Rect(veil) = veil else {
            panic!("the veil is a filled rectangle")
        };
        assert_eq!(veil.rect, rect);
        assert_eq!(veil.fill, egui::Color32::from_rgba_unmultiplied(128, 128, 128, 112));
        let points = |shape: egui::Shape| match shape {
            egui::Shape::LineSegment { points, stroke } => {
                assert_eq!(stroke.color, egui::Color32::BLACK);
                points
            }
            other => panic!("a cross line, not {other:?}"),
        };
        assert_eq!(points(first), [rect.left_top(), rect.right_bottom()]);
        assert_eq!(points(second), [rect.right_top(), rect.left_bottom()]);
    }
}
