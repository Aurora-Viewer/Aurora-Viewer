//! Texture tab (FSPanelFace, indra/newview/fspanelface.cpp and
//! panel_fs_tools_texture.xml of the Legacy Viewer, originally LGPL 2.1): the PBR,
//! Blinn-Phong and Media sub-tabs of the selected faces, copy / paste of
//! face parameters and the face options under them (repeats per meter,
//! hide water, mapping, material sync).
//!
//! Every value is read over the selected faces: when they differ the
//! control shows a mixed value ("Multiple").

#[path = "legacy_texture.rs"]
mod legacy_texture;
#[cfg(test)]
#[path = "texture_paste_tests.rs"]
mod paste_tests;
#[path = "texture_drop.rs"]
pub(super) mod texture_drop;

use super::common::{check, combo, row, same, small, spin, swatch};
use super::{Confirm, Env, FaceClip, PickTarget};
use crate::build::materials::{GltfUpdate, MF_HAS_MEDIA};
use crate::build::{BuildSettings, BuildTool, Tool, texture_copy};
use crate::scene::legacy_mat::{LegacyMaterial, alpha_mode};
use crate::theme::Palette;
use crate::ui::widgets::tabs;
use crate::world::World;
use aurora_assets::material::{AlphaMode, OVERRIDE_NULL_UUID, PbrOverride, TextureTransform};
use aurora_prim::te::{DEFAULT_TEXTURE, TextureFace};
use egui::RichText;
use uuid::Uuid;

/// IMG_ALPHA_GRAD: the diffuse of a "hide water" face.
const IMG_ALPHA_GRAD: Uuid = aurora_prim::te::WATER_EXCLUSION_TEXTURE;
/// Default normal map of the pickers.
pub const DEFAULT_NORMAL: Uuid = Uuid::from_u128(0x85f28839_7a1c_b4e3_d71d_967792970a7b);
/// Default specular map.
pub const DEFAULT_SPECULAR: Uuid = Uuid::from_u128(0x87e0e8f7_8729_1ea8_cfc9_8915773009db);

/// "Bumpiness" (bump codes 0..17).
fn bumps() -> [&'static str; 18] {
    [
        aurora_i18n::tr!("common-none"),
        aurora_i18n::tr!("build-ui-texture-shininess"),
        aurora_i18n::tr!("build-ui-texture-darkness"),
        aurora_i18n::tr!("build-ui-texture-wood-grain"),
        aurora_i18n::tr!("build-ui-texture-bark"),
        "briques",
        "damier",
        aurora_i18n::tr!("build-ui-texture-concrete"),
        aurora_i18n::tr!("build-ui-texture-crusty"),
        aurora_i18n::tr!("build-ui-texture-cut-stone"),
        "disques",
        "gravier",
        aurora_i18n::tr!("build-ui-texture-trouble"),
        "bardage",
        aurora_i18n::tr!("build-ui-texture-stone-tiles"),
        "stuc",
        "aspiration",
        "trame",
    ]
}
/// Index of "Use texture" (BUMPY_TEXTURE / SHINY_TEXTURE).
const BUMPY_TEXTURE: u8 = 18;
const SHINY_TEXTURE: u8 = 4;
fn shines() -> [&'static str; 4] {
    [
        aurora_i18n::tr!("common-none-2"),
        aurora_i18n::tr!("build-ui-texture-low"),
        aurora_i18n::tr!("common-medium"),
        aurora_i18n::tr!("build-ui-texture-high"),
    ]
}
fn alpha_modes() -> [(&'static str, u8); 4] {
    [
        (aurora_i18n::tr!("common-none"), alpha_mode::NONE),
        (aurora_i18n::tr!("build-ui-texture-alpha-blending"), alpha_mode::BLEND),
        (aurora_i18n::tr!("build-ui-texture-alpha-masking"), alpha_mode::MASK),
        (aurora_i18n::tr!("common-emissive-mask"), alpha_mode::EMISSIVE),
    ]
}

/// TEM_* bits of the bump / shiny / fullbright byte.
const TEM_BUMP_MASK: u8 = 0x1f;
const TEM_FULLBRIGHT: u8 = 0x20;
const TEM_SHINY_SHIFT: u8 = 6;
/// Texgen bits of the media byte (0 default, 2 planar).
const TEM_TEX_GEN_MASK: u8 = 0x06;
const TEX_GEN_PLANAR: u8 = 0x02;

/// One selected face.
#[derive(Debug, Clone, Copy)]
pub(super) struct Sel {
    pub idx: usize,
    pub face: u8,
    pub te: TextureFace,
}

const W: f32 = 64.0;

/// LLPrimitive::getTESTAxes: the object axes a face's s and t run along.
pub fn te_st_axes(face: u8) -> (usize, usize) {
    match face {
        0 => (0, 1),
        1 | 3 => (0, 2),
        2 | 4 => (1, 2),
        _ => (0, 1),
    }
}

/// LLGLTFMaterial::convertTextureTransformToPBR.
pub fn convert_to_pbr(scale: [f32; 2], offset: [f32; 2], rotation: f32) -> TextureTransform {
    let pbr_rot = -rotation;
    let (c2, s2) = (rotation.cos().powi(2), rotation.sin().powi(2));
    let sc = [scale[0] * c2 + scale[1] * s2, scale[0] * s2 + scale[1] * c2];
    let (pos_s, pos_t) = (0.5 * (1.0 - sc[0]) - 0.5, 0.5 * (1.0 - sc[1]) - 0.5);
    let (c, s) = (pbr_rot.cos(), pbr_rot.sin());
    let rot_s = pos_s * c + pos_t * s;
    let rot_t = -pos_s * s + pos_t * c;
    TextureTransform {
        offset: [rot_s + 0.5 + offset[0], rot_t + 0.5 - offset[1]],
        scale: sc,
        rotation: pbr_rot,
    }
}

impl BuildTool {
    /// Faces targeted by a library click. Only the active Edit tool can
    /// modify a selection; keep each prim's selected faces and real bounds.
    pub fn inventory_surface_selection(&self, world: &World) -> Vec<(usize, Vec<u8>)> {
        if !self.open || self.tool != Tool::Edit {
            return Vec::new();
        }
        self.sel_prims(world)
            .into_iter()
            .filter(|idx| self.can_modify(world, *idx))
            .filter_map(|idx| {
                let object = world.objects.get(idx)?;
                if object.is_avatar() || object.is_tree() {
                    return None;
                }
                let te = object.te.as_ref()?;
                let count = self.num_faces(world, idx).min(te.faces.len());
                let mut faces = self.sel_faces(world, idx);
                faces.retain(|face| (*face as usize) < count);
                faces.sort_unstable();
                faces.dedup();
                (!faces.is_empty()).then_some((idx, faces))
            })
            .collect()
    }

    /// Whether a texture dropped on `faces` of `idx` lands on one of them:
    /// a PBR face takes it into its override only when `override_ok` (copy
    /// and transfer, dropTextureOneFace), unless Alt removes the material or
    /// the Blinn-Phong preview paints the diffuse. One object's faces, on a
    /// drop or while one is held over it.
    pub fn surface_takes_texture(world: &World, idx: usize, faces: &[u8], override_ok: bool, remove_pbr: bool) -> bool {
        override_ok
            || remove_pbr
            || world.objects.get(idx).is_some_and(|o| o.render.blinn_phong_preview)
            || faces.iter().any(|face| BuildTool::gltf_asset(world, idx, *face).is_none())
    }

    /// `apply_inventory_surface_with` for an item that may go on PBR faces
    /// (the tests' shorthand).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn apply_inventory_surface(
        &mut self,
        world: &mut World,
        env: &mut Env,
        idx: usize,
        faces: &[u8],
        asset: Uuid,
        material: bool,
        remove_pbr: bool,
    ) {
        self.apply_inventory_surface_with(world, env, idx, faces, asset, material, remove_pbr, true);
    }

    /// Apply inventory cargo to explicitly picked faces, leaving selection and
    /// other texture-entry properties unchanged. Follow the displayed shading
    /// mode: a Blinn-Phong preview edits its diffuse while retaining PBR.
    /// Otherwise a texture on PBR replaces its base map; Alt removes PBR,
    /// as dropTextureOneFace does. `override_ok` is false for a texture the
    /// agent may not both copy and transfer: its PBR faces are left as they
    /// are (`surface_takes_texture`).
    #[allow(clippy::too_many_arguments)]
    pub fn apply_inventory_surface_with(
        &mut self,
        world: &mut World,
        env: &mut Env,
        idx: usize,
        faces: &[u8],
        asset: Uuid,
        material: bool,
        remove_pbr: bool,
        override_ok: bool,
    ) {
        if !self.can_modify(world, idx) {
            return;
        }
        // An outstanding picker preview must not later revert this commit.
        if self.ui.picker.open || self.ui.texture_edit.is_some() {
            self.dismiss_texture_picker();
        }
        let count = self.num_faces(world, idx);
        let selected: Vec<Sel> = faces
            .iter()
            .filter(|face| (**face as usize) < count)
            .filter_map(|face| {
                let te = *world.objects.get(idx)?.te.as_ref()?.faces.get(*face as usize)?;
                Some(Sel { idx, face: *face, te })
            })
            .collect();
        if material {
            self.apply_texture(world, env, PickTarget::PbrMaterial, asset, &selected);
            return;
        }
        let blinn_phong = world.objects.get(idx).is_some_and(|o| o.render.blinn_phong_preview);
        let mut legacy = Vec::new();
        let mut updates = Vec::new();
        for face in selected {
            if BuildTool::gltf_asset(world, idx, face.face).is_some() {
                if remove_pbr {
                    updates.push(GltfUpdate {
                        object: idx,
                        face: face.face,
                        asset: Some(Uuid::nil()),
                        over: None,
                    });
                    legacy.push(face);
                } else if blinn_phong {
                    legacy.push(face);
                } else if override_ok {
                    let mut over = BuildTool::gltf_override(world, idx, face.face);
                    over.textures[0] = asset;
                    updates.push(GltfUpdate {
                        object: idx,
                        face: face.face,
                        asset: None,
                        over: Some(over),
                    });
                }
            } else {
                legacy.push(face);
            }
        }
        if !updates.is_empty() {
            self.gltf_update(world, updates);
        }
        self.apply_texture(world, env, PickTarget::Diffuse, asset, &legacy);
    }

    /// Blinn-Phong material of a face as the panel sees it: the one just
    /// sent (until the simulator's new material is loaded), else the
    /// region's, else none.
    pub(super) fn face_legacy(&mut self, env: &Env, world: &World, idx: usize, face: u8, te: &TextureFace) -> Option<LegacyMaterial> {
        let id = world.objects.get(idx)?.full_id;
        if let Some((old, m)) = self.ui.legacy_pending.get(&(id, face))
            && *old == te.material_id
        {
            return m.clone();
        }
        let current = if te.material_id.is_nil() {
            None
        } else {
            env.scene.legacy_mats.peek(&te.material_id)
        };
        if !te.material_id.is_nil() && current.is_none() {
            // ObjectUpdate can precede RenderMaterials. Keep the submitted
            // maps/controls and the focused input while the new ID loads.
            return self.ui.legacy_pending.get(&(id, face)).and_then(|(_, m)| m.clone());
        }
        self.ui.legacy_pending.remove(&(id, face));
        current.map(|m| (*m).clone())
    }

    /// Change the Blinn-Phong material of the selected faces (LLSelectedTEMaterial::
    /// set*): remove only the default material, preserving UVs of empty maps.
    fn edit_legacy(&mut self, env: &Env, world: &World, sel: &[Sel], f: impl Fn(&mut LegacyMaterial)) {
        let mut puts = Vec::new();
        for s in sel {
            let default = default_legacy(env, &s.te);
            let mut m = self
                .face_legacy(env, world, s.idx, s.face, &s.te)
                .unwrap_or_else(|| default.clone());
            f(&mut m);
            let empty = m == default;
            let m = (!empty).then_some(m);
            if let Some(o) = world.objects.get(s.idx) {
                self.ui.legacy_pending.insert((o.full_id, s.face), (s.te.material_id, m.clone()));
            }
            puts.push((s.idx, s.face, m));
        }
        self.legacy_put(world, puts);
    }

    /// Change the GLTF override of the selected faces with a material
    /// (FSRenderMaterialOverrideFunctor): only the faces with a base material.
    fn edit_gltf(&mut self, world: &mut World, sel: &[Sel], f: impl Fn(&mut PbrOverride)) {
        self.edit_gltf_faces(world, sel, |_, o| f(o));
    }

    /// `edit_gltf` with the index of each face in `sel`.
    fn edit_gltf_faces(&mut self, world: &mut World, sel: &[Sel], f: impl Fn(usize, &mut PbrOverride)) {
        let updates: Vec<GltfUpdate> = sel
            .iter()
            .enumerate()
            .filter(|(_, s)| BuildTool::gltf_asset(world, s.idx, s.face).is_some())
            .map(|(k, s)| {
                let mut o = BuildTool::gltf_override(world, s.idx, s.face);
                f(k, &mut o);
                GltfUpdate {
                    object: s.idx,
                    face: s.face,
                    asset: None,
                    over: Some(o),
                }
            })
            .collect();
        if !updates.is_empty() {
            self.gltf_update(world, updates);
        }
    }

    /// Apply a picker choice to the dialog's original faces.
    pub(super) fn apply_texture(&mut self, world: &mut World, env: &mut Env, target: PickTarget, id: Uuid, sel: &[Sel]) {
        match target {
            PickTarget::Diffuse => self.set_diffuse(world, sel, |_, f| f.texture = id),
            PickTarget::Normal => self.edit_legacy(env, world, sel, |m| m.normal_map = id),
            PickTarget::Specular => self.edit_legacy(env, world, sel, |m| m.specular_map = id),
            PickTarget::PbrMaterial => {
                // keep the transform of the override, else the Blinn-Phong one
                let updates = sel
                    .iter()
                    .map(|s| {
                        let old = BuildTool::gltf_override(world, s.idx, s.face);
                        let blank = old.transforms.iter().all(|t| *t == TextureTransform::default());
                        let t = if blank {
                            convert_to_pbr([s.te.scale_s, s.te.scale_t], [s.te.offset_s, s.te.offset_t], s.te.rotation)
                        } else {
                            old.transforms[0]
                        };
                        let keep = PbrOverride {
                            transforms: if blank { [t; 4] } else { old.transforms },
                            ..Default::default()
                        };
                        GltfUpdate {
                            object: s.idx,
                            face: s.face,
                            asset: Some(id),
                            over: Some(keep),
                        }
                    })
                    .collect();
                self.gltf_update(world, updates);
                // ObjectImage after the material (selectionSetGLTFMaterial)
                for idx in sel.iter().map(|s| s.idx).collect::<std::collections::BTreeSet<_>>() {
                    self.send_te(world, idx);
                }
            }
            PickTarget::PbrMap(i) => {
                let tex = if id.is_nil() { OVERRIDE_NULL_UUID } else { id };
                self.edit_gltf(world, sel, |o| o.textures[i] = tex);
            }
            PickTarget::Sculpt => {
                if let Some(idx) = sel.first().map(|s| s.idx)
                    && let Some(mut sc) = world.objects.get(idx).and_then(|o| o.extra.sculpt)
                {
                    sc.texture = id;
                    self.set_extra(world, idx, crate::build::edits::Extra::Sculpt(Some(sc)));
                }
            }
            PickTarget::LightTexture => {
                if let Some(idx) = sel.first().map(|s| s.idx) {
                    let current = world.objects.get(idx).and_then(|o| o.extra.light_image);
                    let e = if id.is_nil() {
                        None
                    } else {
                        Some(aurora_prim::extra::LightImageParams {
                            texture: id,
                            ..current.unwrap_or(crate::build::edits::LIGHT_IMAGE_DEFAULT)
                        })
                    };
                    self.set_extra(world, idx, crate::build::edits::Extra::LightImage(e));
                }
            }
        }
    }
}

/// A face without a legacy material derives its alpha mode from the diffuse.
pub(super) fn default_legacy(env: &Env, te: &TextureFace) -> LegacyMaterial {
    LegacyMaterial {
        diffuse_alpha_mode: if env.scene.textures.has_alpha(&te.texture).unwrap_or(false) {
            alpha_mode::BLEND
        } else {
            alpha_mode::NONE
        },
        ..Default::default()
    }
}

/// The selected faces of modifiable volume prims.
pub(super) fn selected(tool: &BuildTool, world: &World) -> Vec<Sel> {
    tool.sel_te(world)
        .into_iter()
        .filter(|(i, _)| tool.can_modify(world, *i))
        .filter_map(|(idx, face)| {
            let te = *world.objects.get(idx)?.te.as_ref()?.face(face as usize);
            Some(Sel { idx, face, te })
        })
        .collect()
}

pub fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env) {
    if super::legacy::active(ui) {
        legacy_texture::show(ui, p, tool, world, s, env);
        return;
    }
    let sel = selected(tool, world);
    ui.spacing_mut().item_spacing.y = 3.0;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 52.0).max(120.0);
        ui.allocate_ui_with_layout(egui::vec2(width, 30.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            tabs(
                ui,
                p,
                &mut tool.ui.tex_tab,
                &[
                    (aurora_i18n::tr!("build-ui-texture-blinn-phong"), true),
                    ("PBR", true),
                    (aurora_i18n::tr!("extra-media"), true),
                ],
            );
        });
        super::compact::actions(ui, |ui| {
            if super::common::icon_button(
                ui,
                p,
                "clipboard-text-filled",
                aurora_i18n::tr!("build-ui-texture-paste-texture-settings-from-the-clipboard"),
                !sel.is_empty() && tool.ui.face_clipboard.is_some(),
            ) {
                paste_faces(tool, world, env, &sel);
            }
            if super::common::icon_button(
                ui,
                p,
                "copy-filled",
                aurora_i18n::tr!("build-ui-texture-copy-texture-settings-to-clipboard"),
                copy_allowed(tool, world, &sel),
            ) {
                tool.ui.face_clipboard = Some(copy_faces(tool, env, world, &sel));
            }
        });
    });
    if sel.is_empty() {
        small(ui, p, aurora_i18n::tr!("build-ui-texture-no-editable-face-selected"));
        return;
    }
    ui.scope(|ui| {
        if tool.ui.tex_tab != 2 {
            ui.spacing_mut().interact_size.y = 22.0;
            ui.spacing_mut().button_padding = egui::vec2(4.0, 3.0);
        }
        match tool.ui.tex_tab {
            0 => blinn_phong(ui, p, tool, world, s, env, &sel),
            1 => pbr(ui, p, tool, world, env, &sel),
            _ => media(ui, p, tool, world, env, &sel),
        }
        if tool.ui.tex_tab != 2 {
            face_options(ui, p, tool, world, s, env, &sel);
        }
    });
}

fn glow(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, sel: &[Sel]) {
    let (value, mixed) = same(sel.iter().map(|x| x.te.glow));
    let mut value = value.unwrap_or(0.0);
    let width = ui.available_width() * 0.5;
    let field_width = (ui.available_width() - width - ui.spacing().item_spacing.x).max(54.0);
    row(ui, p, aurora_i18n::tr!("build-ui-texture-glow"), width, |ui| {
        if spin(ui, &mut value, 0.1, 0.0..=1.0, 3, field_width, mixed).commit {
            tool.edit_faces(world, |f, _, _| f.glow = value);
        }
    });
}

/// The requested compact layout uses three legacy previews or two PBR maps per row.
fn preview_size(ui: &egui::Ui, columns: usize) -> f32 {
    ((ui.available_width() - 10.0 * (columns - 1) as f32) / columns as f32).clamp(32.0, if columns == 3 { 82.0 } else { 128.0 })
}

#[allow(clippy::too_many_arguments)]
fn material_tint(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    env: &mut Env,
    label: &str,
    target: super::ColorTarget,
    rgb: [f32; 3],
    enabled: bool,
    disabled_hint: &str,
) {
    let label_width = ui.available_width() * 0.5;
    ui.add_enabled_ui(enabled, |ui| {
        row(ui, p, label, label_width, |ui| {
            let size = egui::vec2((ui.available_width() - 2.0).max(24.0), 26.0);
            let response = super::common::color_swatch_sized(ui, p, rgb, "", size, enabled);
            let response = if disabled_hint.is_empty() {
                response
            } else {
                response.on_disabled_hover_text(disabled_hint)
            };
            tool.color_swatch_response(world, env, target, rgb, response);
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn material_asset(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &World,
    asset: Option<Uuid>,
    mixed: bool,
    model: Option<std::sync::Arc<aurora_render::preview::Model>>,
    size: f32,
) {
    ui.vertical(|ui| {
        ui.set_width(size);
        ui.spacing_mut().item_spacing.y = if super::legacy::active(ui) { 0.0 } else { 6.0 };
        let loading = model.is_none() && asset.is_some();
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique("build_material_model_ready"), model.is_some()));
        let response = if let Some(model) = model {
            aurora_render::preview::paint_hint(
                ui,
                ui.scope_id().with("build-material-preview"),
                model,
                crate::ui::texture_library::previews::view(p),
                egui::Vec2::splat(size),
            )
        } else {
            swatch(ui, p, &Default::default(), &mut Default::default(), None, "", size, false, true)
        };
        ui.painter().rect_stroke(
            response.rect,
            ui.visuals().window_corner_radius,
            egui::Stroke::new(1.0, if response.hovered() { p.violet_light } else { p.raised }),
            egui::StrokeKind::Inside,
        );
        if mixed || loading {
            // Mixed selection belongs to the whole material, not its maps.
            ui.painter().text(
                response.rect.center(),
                egui::Align2::CENTER_CENTER,
                if mixed {
                    aurora_i18n::tr!("build-ui-common-multiple")
                } else {
                    aurora_i18n::tr!("common-loading-2")
                },
                egui::FontId::proportional(super::common::SMALL),
                p.ink,
            );
        }
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique(("material_preview", "PbrMaterial".to_string())), response.rect));
        crate::ui::widgets::local_texture::asset(ui, p, response.rect, asset);
        texture_drop::slot(ui, p, tool, world, response.rect, PickTarget::PbrMaterial, true);
        let caption = aurora_i18n::tr!("build-ui-texture-material");
        ui.add_sized(
            [size, if super::legacy::active(ui) { 14.0 } else { 16.0 }],
            egui::Label::new(RichText::new(caption).size(super::common::SMALL).color(p.muted)).truncate(),
        );
        // the material's id when the asset id rule allows it, as the picker
        let tip = match asset {
            Some(id) if response.hovered() && crate::world::inventory::asset_ids::AssetIds::per_frame(world).shown(&id) => id.to_string(),
            _ => caption.to_string(),
        };
        if response.on_hover_text(tip).clicked() {
            tool.ui
                .picker
                .open_materials(aurora_i18n::tr!("build-ui-texture-choose-material"), asset.unwrap_or_default());
            tool.ui.pick_target = Some(PickTarget::PbrMaterial);
        }
    });
}

fn material_map(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &World,
    env: &mut Env,
    id: Option<Uuid>,
    mixed: bool,
    caption: &str,
    target: PickTarget,
    enabled: bool,
    size: f32,
) {
    ui.vertical(|ui| {
        ui.set_width(size);
        ui.spacing_mut().item_spacing.y = if super::legacy::active(ui) { 0.0 } else { 6.0 };
        // A material asset is glTF, not an image: only its maps enter the image loader.
        let response = swatch(ui, p, env.images, &mut tool.ui.wanted_images, id, "", size, mixed, enabled);
        texture_drop::slot(ui, p, tool, world, response.rect, target, enabled);
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique(("material_preview", format!("{target:?}"))), response.rect));
        ui.add_sized(
            [size, if super::legacy::active(ui) { 14.0 } else { 16.0 }],
            egui::Label::new(RichText::new(caption).size(super::common::SMALL).color(p.muted)).truncate(),
        )
        .on_hover_text(caption);
        if response.on_hover_text(caption).clicked() {
            match target {
                PickTarget::PbrMaterial => tool
                    .ui
                    .picker
                    .open_materials(aurora_i18n::tr!("build-ui-texture-choose-material"), id.unwrap_or_default()),
                PickTarget::Diffuse => tool.ui.picker.open_with_defaults(
                    aurora_i18n::tr!("build-ui-texture-choose-texture"),
                    id.unwrap_or_default(),
                    Some(DEFAULT_TEXTURE),
                    false,
                ),
                PickTarget::Normal => tool.ui.picker.open_with_defaults(
                    aurora_i18n::tr!("build-ui-texture-choose-normal-map"),
                    id.unwrap_or_default(),
                    Some(DEFAULT_NORMAL),
                    true,
                ),
                PickTarget::Specular => tool.ui.picker.open_with_defaults(
                    aurora_i18n::tr!("build-ui-texture-choose-specular-map"),
                    id.unwrap_or_default(),
                    Some(DEFAULT_SPECULAR),
                    true,
                ),
                _ => tool.ui.picker.open_with_defaults(
                    &aurora_i18n::tr!("build-ui-texture-choose", cap = caption.to_owned()),
                    id.unwrap_or_default(),
                    None,
                    true,
                ),
            }
            tool.ui.pick_target = Some(target);
        }
    });
}

/// Two UV rows, with scale/flip, offset and rotation visible together.
///
/// Returns the components to write to every selected face: one committed
/// with a new value, or typed over a mixed ("—") field even when it equals
/// the first face's hidden value. LLPanelFace sends each spinner's commit on
/// its own (onCommitTextureScaleX / onCommitGLTFTextureScaleU…,
/// indra/newview/llpanelface.cpp): only that component reaches the faces and
/// the other, possibly mixed, components keep each face's value.
fn uv_controls(
    ui: &mut egui::Ui,
    p: &Palette,
    values: &mut [f32; 5],
    mixed: [bool; 5],
    decimals: usize,
    enabled: bool,
    axes: [&str; 2],
) -> [bool; 5] {
    let mut write = [false; 5];
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (column, width, title) in [
                (0, 103.0, aurora_i18n::tr!("common-scale")),
                (1, 75.0, aurora_i18n::tr!("build-ui-texture-offset")),
                (2, 71.0, aurora_i18n::tr!("build-ui-texture-rotation")),
            ] {
                let height = if super::legacy::active(ui) { 60.0 } else { 78.0 };
                ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.label(RichText::new(title).size(super::common::SMALL).color(p.muted));
                    for (axis, name) in axes.iter().enumerate().take(if column == 2 { 1 } else { 2 }) {
                        let index = if column == 2 { 4 } else { column * 2 + axis };
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            if column != 2 {
                                ui.label(RichText::new(*name).size(super::common::SMALL).color(p.muted));
                            }
                            let (increment, range, d) = match column {
                                0 => (0.1, -10000.0..=10000.0, decimals),
                                1 => (0.01, -1.0..=1.0, decimals),
                                _ => (1.0, -360.0..=360.0, decimals.min(2)),
                            };
                            let before = values[index];
                            let edit = super::common::spin_with_min_decimals(
                                ui,
                                &mut values[index],
                                increment,
                                range,
                                d,
                                if column == 2 { 71.0 } else { 65.0 },
                                mixed[index],
                                d.min(3),
                            );
                            // A release that moved nothing sends nothing; a
                            // number typed over "—" is sent as it is.
                            if edit.typed_over_mixed || (edit.commit && (values[index] - before).abs() > 1e-6) {
                                write[index] = true;
                            }
                            // FSPanelFace::onCommitFlip negates the shown value
                            // and forces the spinner's commit.
                            if column == 0
                                && super::compact::icon_button(ui, p, "arrows-exchange", aurora_i18n::tr!("common-rotate"), enabled)
                                    .clicked()
                            {
                                values[index] = -values[index];
                                write[index] = true;
                            }
                        });
                    }
                });
            }
        });
    });
    write
}

// ---------------------------------------------------------------- PBR

fn pbr(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &mut Env, sel: &[Sel]) {
    let (asset, asset_mixed) = same(sel.iter().map(|x| {
        world
            .objects
            .get(x.idx)
            .and_then(|object| object.local_texture(x.face as usize, crate::world::objects::LocalTextureSlot::Material))
            .or_else(|| BuildTool::gltf_asset(world, x.idx, x.face))
    }));
    let asset = asset.flatten();
    let has_all = sel.iter().all(|x| BuildTool::gltf_asset(world, x.idx, x.face).is_some());
    // effective material of the first face: base asset + override
    let first = sel[0];
    let over = BuildTool::gltf_override(world, first.idx, first.face);
    let base = asset.and_then(|a| env.scene.materials.get(&a));
    let loaded = base.is_some();
    let mut mat = base.map(|m| (*m).clone()).unwrap_or_default();
    if !asset.is_some_and(|asset| world.local_assets.contains_key(&asset)) {
        mat.apply_override(&over);
    }
    if let Some(appearance) = world
        .objects
        .get(first.idx)
        .and_then(|object| object.local_appearance(first.face as usize))
    {
        appearance.apply_pbr(&mut mat);
    }
    let face_transforms: Vec<[TextureTransform; 4]> = sel.iter().map(|x| face_pbr_transforms(world, env, x)).collect();
    // What each face's server override inherits; None while a material is
    // still loading: the UV fields wait for it rather than guess the pair.
    let inherited: Option<Vec<[TextureTransform; 4]>> = sel.iter().map(|x| face_server_pbr_transforms(world, env, x)).collect();
    // LLMaterialEditor::canModifyObjectsMaterial; not editable, the maps show no id
    let faces: Vec<(usize, u8)> = sel.iter().map(|x| (x.idx, x.face)).collect();
    let editable = has_all && tool.material_edit_allowed(world, &faces);
    let shown = |id: Option<Uuid>| id.filter(|_| editable);
    tool.ui.material_previews.poll();
    let model = asset.filter(|_| loaded && !asset_mixed).and_then(|asset| {
        tool.ui.material_previews.material_model(
            asset,
            std::sync::Arc::new(mat.clone()),
            crate::imports::preview::MaterialShape::Sphere,
            env.scene,
            &mut tool.ui.wanted_images,
            ui.ctx(),
        )
    });
    if super::legacy::active(ui) {
        legacy_texture::pbr_surface(ui, p, tool, world, env, sel, &mat, asset, asset_mixed, model, editable);
    } else {
        let size = preview_size(ui, 3);
        egui::Grid::new("pbr_map_grid")
            .num_columns(3)
            .spacing(egui::vec2(10.0, 10.0))
            .show(ui, |ui| {
                material_asset(ui, p, tool, world, asset, asset_mixed, model, size);
                for (index, (id, caption, channel)) in [
                    (mat.base_color_texture, aurora_i18n::tr!("common-color"), 0),
                    (mat.normal_texture, aurora_i18n::tr!("build-ui-texture-normal"), 1),
                    (mat.emissive_texture, aurora_i18n::tr!("build-ui-texture-emissive"), 3),
                    (mat.metallic_roughness_texture, "(O)RM", 2),
                ]
                .into_iter()
                .enumerate()
                {
                    material_map(
                        ui,
                        p,
                        tool,
                        world,
                        env,
                        shown(id),
                        false,
                        caption,
                        PickTarget::PbrMap(channel),
                        editable,
                        size,
                    );
                    if (index + 2) % 3 == 0 {
                        ui.end_row();
                    }
                }
            });
    }
    if !super::legacy::active(ui) {
        pbr_factors(ui, p, tool, world, env, sel, &mat, editable);
    }
    ui.add_space(5.0);
    ui.scope(|ui| {
        if !super::legacy::active(ui) {
            super::common::section(ui, p, aurora_i18n::tr!("build-material-placement"));
        }
        // texture transforms
        ui.add_space(4.0);
        let mut ch = tool.ui.pbr_channel;
        let channels = [
            (aurora_i18n::tr!("common-all"), 0usize),
            (aurora_i18n::tr!("common-color"), 1),
            (aurora_i18n::tr!("build-ui-texture-normal"), 2),
            ("(O)RM", 3),
            (aurora_i18n::tr!("build-ui-texture-emissive"), 4),
        ];
        let labels: Vec<(&str, bool)> = channels.iter().map(|(name, _)| (*name, true)).collect();
        if super::legacy::active(ui) {
            crate::ui::widgets::compact_tabs(ui, p, &mut ch, &labels);
        } else {
            tabs(ui, p, &mut ch, &labels);
        }
        tool.ui.pbr_channel = ch;
        // "All" covers the four channels and shows the base color's; the
        // others in the panel's order: base, normal, ORM, emissive. A
        // component is mixed when it differs between the selected faces or,
        // under "All", between the channels
        // (LLPanelFace::setMaterialOverridesFromSelection).
        let channels = if ch == 0 { 0..4 } else { ch - 1..ch };
        let components = |t: &TextureTransform| [t.scale[0], t.scale[1], t.offset[0], t.offset[1], t.rotation];
        let shown = components(&mat.transforms[channels.start]);
        let mixed: [bool; 5] = std::array::from_fn(|i| {
            face_transforms
                .iter()
                .flat_map(|transforms| &transforms[channels.clone()])
                .any(|t| (components(t)[i] - shown[i]).abs() > 1e-5)
        });
        let mut values = shown;
        values[4] = values[4].to_degrees();
        let write = uv_controls(ui, p, &mut values, mixed, 5, editable && inherited.is_some(), ["U", "V"]);
        if let Some(inherited) = inherited.as_deref()
            && write.contains(&true)
        {
            // LLPanelFace::updateGLTFTextureTransform: the committed
            // component only, on each face's override and each channel.
            // apply_override takes a scale or an offset as a whole pair, so
            // a pair with a committed component starts from what that face
            // and channel inherit on the server: the other component keeps
            // its value, the base material's when the override has none.
            // Better than LLPanelFace, which starts from the raw override
            // and resets that component to the identity. Like it, a pair
            // that ends at the identity (scale 1 1, offset 0 0) is "no
            // override" to applyOverride: the material's own pair shows.
            let new = [values[0], values[1], values[2], values[3], values[4].to_radians()];
            tool.edit_gltf_faces(world, sel, |k, o| {
                for channel in channels.clone() {
                    let shown = inherited[k][channel];
                    let t = &mut o.transforms[channel];
                    if write[0] || write[1] {
                        t.scale = shown.scale;
                    }
                    if write[2] || write[3] {
                        t.offset = shown.offset;
                    }
                    let mut a = components(t);
                    for i in (0..5).filter(|&i| write[i]) {
                        a[i] = new[i];
                    }
                    *t = TextureTransform {
                        scale: [a[0], a[1]],
                        offset: [a[2], a[3]],
                        rotation: a[4],
                    };
                }
            });
        }
    });
}

/// The texture transforms a face's server override applies over: its
/// server material's (never a local material's preview, which only this
/// viewer shows) under that override (LLGLTFMaterial::applyOverride).
/// None while the material loads; a material that cannot be had renders
/// as the default one, so the override applies over the identity.
fn face_server_pbr_transforms(world: &World, env: &mut Env, x: &Sel) -> Option<[TextureTransform; 4]> {
    let mut mat = aurora_assets::PbrMaterial::default();
    if let Some(asset) = BuildTool::gltf_asset(world, x.idx, x.face) {
        match env.scene.materials.resolve(&asset) {
            crate::scene::meshes::MaterialStatus::Loaded(m) => mat.transforms = m.transforms,
            crate::scene::meshes::MaterialStatus::Pending => return None,
            crate::scene::meshes::MaterialStatus::Missing => {}
        }
    }
    mat.apply_override(&BuildTool::gltf_override(world, x.idx, x.face));
    Some(mat.transforms)
}

/// The texture transforms a face renders with: its material's under its
/// override (LLGLTFMaterial::applyOverride), as `pbr` builds the first face's.
fn face_pbr_transforms(world: &World, env: &mut Env, x: &Sel) -> [TextureTransform; 4] {
    let asset = world
        .objects
        .get(x.idx)
        .and_then(|object| object.local_texture(x.face as usize, crate::world::objects::LocalTextureSlot::Material))
        .or_else(|| BuildTool::gltf_asset(world, x.idx, x.face));
    let mut mat = aurora_assets::PbrMaterial {
        transforms: asset
            .and_then(|asset| env.scene.materials.get(&asset))
            .map(|m| m.transforms)
            .unwrap_or_default(),
        ..Default::default()
    };
    if !asset.is_some_and(|asset| world.local_assets.contains_key(&asset)) {
        mat.apply_override(&BuildTool::gltf_override(world, x.idx, x.face));
    }
    mat.transforms
}

#[allow(clippy::too_many_arguments)]
fn pbr_factors(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    env: &mut Env,
    sel: &[Sel],
    mat: &aurora_assets::material::PbrMaterial,
    editable: bool,
) {
    ui.vertical(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        super::common::section(ui, p, aurora_i18n::tr!("build-material-surface"));
        for (label, target, color) in [
            (
                aurora_i18n::tr!("build-material-base-tint"),
                super::ColorTarget::PbrBase,
                [mat.base_color_factor[0], mat.base_color_factor[1], mat.base_color_factor[2]],
            ),
            (
                aurora_i18n::tr!("build-material-emissive-tint"),
                super::ColorTarget::PbrEmissive,
                mat.emissive_factor,
            ),
        ] {
            material_tint(
                ui,
                p,
                tool,
                world,
                env,
                label,
                target,
                color.map(super::common::linear_to_srgb),
                editable,
                aurora_i18n::tr!("build-material-tint-needs-pbr"),
            );
        }
        glow(ui, p, tool, world, sel);
        if let Some(v) = check(ui, p, mat.double_sided, false, aurora_i18n::tr!("common-double-sided"), editable) {
            tool.edit_gltf(world, sel, |o| o.double_sided = Some(v));
        }
        ui.add_enabled_ui(editable, |ui| {
            // LLPanelFace keeps its numeric controls inside the fixed panel.
            // Scaled fonts can make DragValue wider than the nominal 54 pts;
            // reserve its measured text, padding and increment buttons first.
            let value_text = RichText::new(aurora_i18n::number_text("0.000")).text_style(ui.style().drag_value_text_style.clone());
            let value_width = egui::WidgetText::from(value_text)
                .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body)
                .size()
                .x;
            let field_width =
                (value_width + 2.0 * ui.spacing().button_padding.x + 13.0).max(ui.available_width() * 0.5 - ui.spacing().item_spacing.x);
            let width = (ui.available_width() - field_width - ui.spacing().item_spacing.x).max(0.0);
            let bc = mat.base_color_factor;
            let mut alpha = bc[3];
            row(ui, p, aurora_i18n::tr!("common-alpha"), width, |ui| {
                if spin(ui, &mut alpha, 0.1, 0.0..=1.0, 3, field_width, false).commit {
                    tool.edit_gltf(world, sel, |o| o.base_color_factor = Some([bc[0], bc[1], bc[2], alpha]));
                }
            });
            let modes = [
                (aurora_i18n::tr!("common-opaque"), AlphaMode::Opaque),
                (aurora_i18n::tr!("build-ui-texture-blend"), AlphaMode::Blend),
                (aurora_i18n::tr!("build-ui-texture-masking"), AlphaMode::Mask),
            ];
            row(ui, p, aurora_i18n::tr!("build-ui-texture-mode"), width, |ui| {
                if let Some(mode) = combo(
                    ui,
                    "pbr_alpha_mode",
                    Some(mat.alpha_mode),
                    &modes,
                    (ui.available_width() - 4.0).max(70.0),
                    editable,
                ) {
                    tool.edit_gltf(world, sel, |o| o.alpha_mode = Some(mode));
                }
            });
            if mat.alpha_mode == AlphaMode::Mask {
                let mut cutoff = mat.alpha_cutoff;
                row(ui, p, aurora_i18n::tr!("build-ui-texture-cut"), width, |ui| {
                    if spin(ui, &mut cutoff, 0.1, 0.0..=1.0, 3, field_width, false).commit {
                        tool.edit_gltf(world, sel, |o| o.alpha_cutoff = Some(cutoff));
                    }
                });
            }
            super::common::section(ui, p, aurora_i18n::tr!("build-material-lighting"));
            for (name, value, metallic) in [
                (aurora_i18n::tr!("build-ui-texture-metallic"), mat.metallic_factor, true),
                (aurora_i18n::tr!("build-ui-texture-roughness"), mat.roughness_factor, false),
            ] {
                let mut v = value;
                row(ui, p, name, width, |ui| {
                    if spin(ui, &mut v, 0.1, 0.0..=1.0, 3, field_width, false).commit {
                        tool.edit_gltf(world, sel, |o| {
                            if metallic {
                                o.metallic_factor = Some(v)
                            } else {
                                o.roughness_factor = Some(v)
                            }
                        });
                    }
                });
            }
        });
    });
}

// --------------------------------------------------------- Blinn-Phong

#[allow(clippy::too_many_arguments)]
fn blinn_phong(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env, sel: &[Sel]) {
    let legacy: Vec<Option<LegacyMaterial>> = sel
        .iter()
        .map(|x| {
            let mut material = tool.face_legacy(env, world, x.idx, x.face, &x.te);
            if let Some(appearance) = world.objects.get(x.idx).and_then(|object| object.local_appearance(x.face as usize)) {
                let value = material.get_or_insert_with(|| default_legacy(env, &x.te));
                appearance.apply_legacy(value);
            }
            material
        })
        .collect();
    let (tex, tex_mixed) = same(sel.iter().map(|x| {
        world
            .objects
            .get(x.idx)
            .map_or(x.te.texture, |object| object.visual_texture_face(x.face as usize).texture)
    }));
    let (normal, normal_mixed) = same(legacy.iter().map(|m| m.as_ref().map(|m| m.normal_map).unwrap_or_default()));
    let (spec, spec_mixed) = same(legacy.iter().map(|m| m.as_ref().map(|m| m.specular_map).unwrap_or_default()));
    let normal = normal.filter(|n| !n.is_nil());
    let spec = spec.filter(|n| !n.is_nil());
    let water = sel.iter().all(|x| is_hide_water(&x.te, legacy[0].as_ref()));
    let editable = !water;
    let size = preview_size(ui, 3);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        for (id, mixed, caption, target) in [
            (tex, tex_mixed, aurora_i18n::tr!("build-ui-texture-diffuse"), PickTarget::Diffuse),
            (
                normal,
                normal_mixed,
                aurora_i18n::tr!("build-ui-texture-normal"),
                PickTarget::Normal,
            ),
            (
                spec,
                spec_mixed,
                aurora_i18n::tr!("build-ui-texture-specular"),
                PickTarget::Specular,
            ),
        ] {
            material_map(ui, p, tool, world, env, id, mixed, caption, target, editable, size);
        }
    });
    ui.vertical(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        super::common::section(ui, p, aurora_i18n::tr!("build-material-surface"));
        let (col, _) = same(sel.iter().map(|x| [x.te.color[0], x.te.color[1], x.te.color[2]]));
        material_tint(
            ui,
            p,
            tool,
            world,
            env,
            aurora_i18n::tr!("build-material-diffuse-tint"),
            super::ColorTarget::Diffuse,
            col.unwrap_or([1.0; 3]),
            editable,
            "",
        );
        let label_width = ui.available_width() * 0.5;
        let field_width = (ui.available_width() - label_width - ui.spacing().item_spacing.x - 16.0).max(54.0);
        let (fb, fb_mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright & TEM_FULLBRIGHT != 0));
        if let Some(v) = check(
            ui,
            p,
            fb.unwrap_or(false),
            fb_mixed,
            aurora_i18n::tr!("build-ui-texture-fullbright"),
            editable,
        ) {
            tool.edit_faces(world, |f, _, _| {
                f.bump_shiny_fullbright = if v {
                    f.bump_shiny_fullbright | TEM_FULLBRIGHT
                } else {
                    f.bump_shiny_fullbright & !TEM_FULLBRIGHT
                }
            });
        }
        glow(ui, p, tool, world, sel);
        let (alpha, alpha_mixed) = same(sel.iter().map(|x| x.te.color[3]));
        let mut transp = (1.0 - alpha.unwrap_or(1.0)) * 100.0;
        row(ui, p, aurora_i18n::tr!("build-ui-texture-transparency"), label_width, |ui| {
            if spin(ui, &mut transp, 2.0, 0.0..=100.0, 0, field_width, alpha_mixed).commit {
                let a = (100.0 - transp) / 100.0;
                tool.edit_faces(world, |f, _, _| f.color[3] = a);
            }
            ui.label("%");
        });
        // alpha mode, mask cutoff
        let has_alpha = tex.and_then(|t| env.scene.textures.has_alpha(&t)).unwrap_or(false);
        let transp_zero = sel.iter().all(|x| x.te.color[3] >= 1.0);
        let default_mode = if has_alpha { alpha_mode::BLEND } else { alpha_mode::NONE };
        let (mode, mode_mixed) = same(
            legacy
                .iter()
                .map(|m| m.as_ref().map(|m| m.diffuse_alpha_mode).unwrap_or(default_mode)),
        );
        let mode = if !transp_zero { Some(alpha_mode::BLEND) } else { mode };
        row(ui, p, aurora_i18n::tr!("build-ui-texture-alpha-mode"), label_width, |ui| {
            let current = if mode_mixed { None } else { mode };
            if let Some(m) = combo(
                ui,
                "bp_alpha_mode",
                current,
                &alpha_modes(),
                field_width + 16.0,
                editable && has_alpha && transp_zero,
            ) {
                tool.edit_legacy(env, world, sel, |x| x.diffuse_alpha_mode = m);
            }
        });
        if mode == Some(alpha_mode::MASK) {
            let (cut, cut_mixed) = same(legacy.iter().map(|m| m.as_ref().map(|m| m.alpha_cutoff as i32).unwrap_or(127)));
            let mut cut = cut.unwrap_or(127) as f32;
            row(ui, p, aurora_i18n::tr!("build-ui-texture-mask-cutoff"), label_width, |ui| {
                ui.add_enabled_ui(editable && mode == Some(alpha_mode::MASK), |ui| {
                    if spin(ui, &mut cut, 1.0, 0.0..=255.0, 0, field_width, cut_mixed).commit {
                        tool.edit_legacy(env, world, sel, |x| x.alpha_cutoff = cut as u8);
                    }
                });
            });
        }

        super::common::section(ui, p, aurora_i18n::tr!("build-material-relief-reflection"));
        let (bump, bump_mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright & TEM_BUMP_MASK));
        let mut items: Vec<(&str, u8)> = bumps().iter().enumerate().map(|(i, n)| (*n, i as u8)).collect();
        let shown = if normal.is_some() {
            items.push((aurora_i18n::tr!("build-ui-texture-use-texture"), BUMPY_TEXTURE));
            Some(BUMPY_TEXTURE)
        } else if bump_mixed {
            None
        } else {
            bump
        };
        row(ui, p, aurora_i18n::tr!("build-ui-texture-bumpiness"), label_width, |ui| {
            if let Some(b) = combo(ui, "bumpiness", shown, &items, field_width + 16.0, editable)
                && b != BUMPY_TEXTURE
            {
                tool.edit_faces(world, |f, _, _| {
                    f.bump_shiny_fullbright = (f.bump_shiny_fullbright & !TEM_BUMP_MASK) | b
                });
                // a bump code replaces the normal map (FSPanelFace::sendBump)
                if normal.is_some() {
                    tool.edit_legacy(env, world, sel, |m| m.normal_map = Uuid::nil());
                }
            }
        });
        let (shiny, shiny_mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright >> TEM_SHINY_SHIFT));
        let mut items: Vec<(&str, u8)> = shines().iter().enumerate().map(|(i, n)| (*n, i as u8)).collect();
        let shown = if spec.is_some() {
            items.push((aurora_i18n::tr!("build-ui-texture-use-texture"), SHINY_TEXTURE));
            Some(SHINY_TEXTURE)
        } else if shiny_mixed {
            None
        } else {
            shiny
        };
        row(ui, p, aurora_i18n::tr!("common-brightness"), label_width, |ui| {
            if let Some(v) = combo(ui, "shininess", shown, &items, field_width + 16.0, editable)
                && v != SHINY_TEXTURE
            {
                tool.edit_faces(world, |f, _, _| {
                    f.bump_shiny_fullbright = (f.bump_shiny_fullbright & !(3 << TEM_SHINY_SHIFT)) | (v << TEM_SHINY_SHIFT)
                });
                if spec.is_some() {
                    tool.edit_legacy(env, world, sel, |m| m.specular_map = Uuid::nil());
                }
            }
        });

        let sc = legacy[0].as_ref().map(|m| m.specular_color).unwrap_or([255; 4]);
        let rgb = [sc[0] as f32 / 255.0, sc[1] as f32 / 255.0, sc[2] as f32 / 255.0];
        material_tint(
            ui,
            p,
            tool,
            world,
            env,
            aurora_i18n::tr!("build-material-specular-tint"),
            super::ColorTarget::Specular,
            rgb,
            editable && spec.is_some(),
            aurora_i18n::tr!("build-material-tint-needs-specular"),
        );
        if spec.is_some() {
            let (gl, gl_mixed) = same(legacy.iter().map(|m| m.as_ref().map(|m| m.specular_exp).unwrap_or(51)));
            let (env_i, env_mixed) = same(legacy.iter().map(|m| m.as_ref().map(|m| m.env_intensity).unwrap_or(0)));
            let mut g = gl.unwrap_or(51) as f32;
            let mut en = env_i.unwrap_or(0) as f32;
            ui.add_enabled_ui(editable && spec.is_some(), |ui| {
                row(ui, p, aurora_i18n::tr!("build-ui-texture-shininess"), label_width, |ui| {
                    if spin(ui, &mut g, 1.0, 0.0..=255.0, 0, field_width, gl_mixed).commit {
                        tool.edit_legacy(env, world, sel, |m| m.specular_exp = g as u8);
                    }
                });
                row(ui, p, aurora_i18n::tr!("common-environment"), label_width, |ui| {
                    if spin(ui, &mut en, 1.0, 0.0..=255.0, 0, field_width, env_mixed).commit {
                        tool.edit_legacy(env, world, sel, |m| m.env_intensity = en as u8);
                    }
                });
            });
        }
    });
    ui.add_space(5.0);
    bp_transforms(ui, p, tool, world, s, env, sel, &legacy, normal, spec, editable);
}

/// Shared UV commits for the Aurora controls and the Legacy vertical rows.
#[allow(clippy::too_many_arguments)]
fn bp_transforms(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    env: &mut Env,
    sel: &[Sel],
    legacy: &[Option<LegacyMaterial>],
    normal: Option<Uuid>,
    spec: Option<Uuid>,
    editable: bool,
) {
    ui.scope(|ui| {
        if !super::legacy::active(ui) {
            super::common::section(ui, p, aurora_i18n::tr!("build-material-placement"));
        }
        // UV of the diffuse / normal / specular maps
        ui.add_space(4.0);
        let mut ch = tool.ui.bp_channel;
        let labels = [
            (aurora_i18n::tr!("build-ui-texture-diffuse"), true),
            (aurora_i18n::tr!("build-ui-texture-normal"), true),
            (aurora_i18n::tr!("build-ui-texture-specular"), true),
        ];
        if super::legacy::active(ui) {
            crate::ui::widgets::compact_tabs(ui, p, &mut ch, &labels);
        } else {
            tabs(ui, p, &mut ch, &labels);
        }
        tool.ui.bp_channel = ch;
        let planar = sel.iter().all(|x| x.te.media_flags & TEM_TEX_GEN_MASK == TEX_GEN_PLANAR);
        // planar mapping shows twice the scale (FSPanelFace::updateUI)
        let k = if planar { 2.0 } else { 1.0 };
        let read = |x: &Sel, m: &Option<LegacyMaterial>| -> [f32; 5] {
            match ch {
                1 => m
                    .as_ref()
                    .map(|m| [m.normal_st[0], m.normal_st[1], m.normal_st[2], m.normal_st[3], m.normal_rot]),
                2 => m.as_ref().map(|m| {
                    [
                        m.specular_st[0],
                        m.specular_st[1],
                        m.specular_st[2],
                        m.specular_st[3],
                        m.specular_rot,
                    ]
                }),
                _ => Some([x.te.scale_s, x.te.scale_t, x.te.offset_s, x.te.offset_t, x.te.rotation]),
            }
            .unwrap_or([1.0, 1.0, 0.0, 0.0, 0.0])
        };
        let vals: Vec<[f32; 5]> = sel.iter().zip(legacy).map(|(x, m)| read(x, m)).collect();
        let mixed = |i: usize| vals.iter().any(|v| (v[i] - vals[0][i]).abs() > 1e-5);
        let mut v = vals[0];
        v[0] *= k;
        v[1] *= k;
        v[4] = v[4].to_degrees();
        let decimals = s.decimal_precision.min(7) as usize;

        let axes = if super::legacy::active(ui) { ["U", "V"] } else { ["H", "V"] };
        let write = uv_controls(ui, p, &mut v, std::array::from_fn(mixed), decimals, editable, axes);

        if write.contains(&true) {
            // only the committed components; the others keep each face's value
            let changed: Vec<usize> = (0..5).filter(|&i| write[i]).collect();
            let new = [v[0] / k, v[1] / k, v[2], v[3], v[4].to_radians()];
            let set = |a: &mut [f32; 5]| {
                for &i in &changed {
                    a[i] = new[i];
                }
            };
            let sync = s.sync_materials;
            if ch == 0 {
                tool.edit_faces(world, |f, _, _| {
                    let mut a = [f.scale_s, f.scale_t, f.offset_s, f.offset_t, f.rotation];
                    set(&mut a);
                    [f.scale_s, f.scale_t, f.offset_s, f.offset_t, f.rotation] = a;
                });
            }
            if ch == 1 || (ch == 0 && sync && normal.is_some()) {
                tool.edit_legacy(env, world, sel, |m| {
                    let mut a = [m.normal_st[0], m.normal_st[1], m.normal_st[2], m.normal_st[3], m.normal_rot];
                    set(&mut a);
                    m.normal_st = [a[0], a[1], a[2], a[3]];
                    m.normal_rot = a[4];
                });
            }
            if ch == 2 || (ch == 0 && sync && spec.is_some()) {
                tool.edit_legacy(env, world, sel, |m| {
                    let mut a = [
                        m.specular_st[0],
                        m.specular_st[1],
                        m.specular_st[2],
                        m.specular_st[3],
                        m.specular_rot,
                    ];
                    set(&mut a);
                    m.specular_st = [a[0], a[1], a[2], a[3]];
                    m.specular_rot = a[4];
                });
            }
        }
    });
}

/// A "hide water" face (FSPanelFace: alpha gradient diffuse, opaque, no maps).
fn is_hide_water(te: &TextureFace, m: Option<&LegacyMaterial>) -> bool {
    te.texture == IMG_ALPHA_GRAD && te.color[3] >= 1.0 && m.is_none_or(|m| m.normal_map.is_nil() && m.specular_map.is_nil())
}

#[cfg(test)]
mod empty_map_tests {
    use super::*;

    #[test]
    fn empty_map_uvs_survive_material_submission_and_later_texture_assignment() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::face_selection_events(false)) {
            world.apply(event);
        }
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(99620)).unwrap();
        let mut tool = BuildTool::default();
        tool.click_select(&mut world, &BuildSettings::default(), Some(idx), false);
        let sel = selected(&tool, &world);
        assert!(!sel.is_empty());
        let mut scene = crate::scene::Scene::new(std::env::temp_dir(), world.avatar_lib.clone());
        let mut media = Default::default();
        let mut env = Env {
            images: &Default::default(),
            media: &mut media,
            scene: &mut scene,
            zoom_fraction: 0.0,
            property_lines: &mut false,
            demo: false,
        };
        tool.out.clear();
        tool.edit_legacy(&env, &world, &sel, |m| {
            m.normal_st = [2.0, 3.0, 0.1, -0.2];
            m.normal_rot = 0.5;
            m.specular_st = [4.0, 5.0, -0.3, 0.4];
            m.specular_rot = -0.5;
        });
        let material = tool.face_legacy(&env, &world, idx, sel[0].face, &sel[0].te).unwrap();
        assert!(material.normal_map.is_nil() && material.specular_map.is_nil());
        let body = tool
            .out
            .iter()
            .find_map(|command| match command {
                aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Cap { cap, body, .. }) if cap == "RenderMaterials" => Some(body),
                _ => None,
            })
            .unwrap();
        let payload = crate::scene::legacy_mat::unzip_llsd(body.get("Zipped").as_binary()).unwrap();
        let encoded = payload.get("FullMaterialsPerFace").as_array()[0].get("Material");
        assert_eq!(LegacyMaterial::from_llsd(encoded), material);
        tool.apply_texture(&mut world, &mut env, PickTarget::Normal, DEFAULT_NORMAL, &sel);
        tool.apply_texture(&mut world, &mut env, PickTarget::Specular, DEFAULT_SPECULAR, &sel);
        let assigned = tool.face_legacy(&env, &world, idx, sel[0].face, &sel[0].te).unwrap();
        assert_eq!(assigned.normal_map, DEFAULT_NORMAL);
        assert_eq!(assigned.specular_map, DEFAULT_SPECULAR);
        assert_eq!((assigned.normal_st, assigned.normal_rot), (material.normal_st, material.normal_rot));
        assert_eq!(
            (assigned.specular_st, assigned.specular_rot),
            (material.specular_st, material.specular_rot)
        );
        tool.apply_texture(&mut world, &mut env, PickTarget::Normal, Uuid::nil(), &sel);
        tool.apply_texture(&mut world, &mut env, PickTarget::Specular, Uuid::nil(), &sel);
        assert_eq!(tool.face_legacy(&env, &world, idx, sel[0].face, &sel[0].te), Some(material));
    }
}

/// Options under the PBR / Blinn-Phong sub-tabs.
fn repeats_meter(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, sel: &[Sel]) {
    let planar = sel.iter().all(|x| x.te.media_flags & TEM_TEX_GEN_MASK == TEX_GEN_PLANAR);
    ui.vertical(|ui| {
        // repeats per meter: max(scale_s / object s size, scale_t / object t size)
        let rpm_of = |x: &Sel| -> f32 {
            let sc = world.objects.get(x.idx).map(|o| o.scale).unwrap_or(glam::Vec3::ONE);
            let (sa, ta) = te_st_axes(x.face);
            (x.te.scale_s / sc[sa].max(0.001)).max(x.te.scale_t / sc[ta].max(0.001))
        };
        let (rpm, rpm_mixed) = same(sel.iter().map(|x| (rpm_of(x) * 10.0).round() / 10.0));
        let mut r = rpm.unwrap_or(1.0);
        let width = (ui.available_width() - 69.0).max(90.0);
        row(ui, p, aurora_i18n::tr!("build-ui-texture-repeats-meter"), width, |ui| {
            ui.add_enabled_ui(!planar, |ui| {
                if spin(ui, &mut r, 0.1, -100.0..=100.0, 1, W, rpm_mixed).commit {
                    // selectionTexScaleAutofit
                    let scales: Vec<(usize, glam::Vec3)> = sel
                        .iter()
                        .filter_map(|x| world.objects.get(x.idx).map(|o| (x.idx, o.scale)))
                        .collect();
                    tool.edit_faces(world, |f, idx, face| {
                        if let Some((_, sc)) = scales.iter().find(|(i, _)| *i == idx) {
                            let (sa, ta) = te_st_axes(face);
                            f.scale_s = sc[sa] * r;
                            f.scale_t = sc[ta] * r;
                        }
                    });
                }
            });
        });
    });
}

fn face_options(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    env: &mut Env,
    sel: &[Sel],
) {
    let planar = sel.iter().all(|x| x.te.media_flags & TEM_TEX_GEN_MASK == TEX_GEN_PLANAR);
    repeats_meter(ui, p, tool, world, sel);
    ui.add_space(8.0);
    ui.vertical(|ui| {
        ui.vertical(|ui| {
            let legacy0 = tool.face_legacy(env, world, sel[0].idx, sel[0].face, &sel[0].te);
            let water = sel.iter().all(|x| is_hide_water(&x.te, legacy0.as_ref()));
            if let Some(v) = check(ui, p, water, false, aurora_i18n::tr!("build-ui-texture-occlude-water"), true) {
                if v {
                    tool.ui.confirm = Some(Confirm::HideWater);
                } else {
                    tool.edit_faces(world, |f, _, _| f.texture = DEFAULT_TEXTURE);
                }
            }
        });
        ui.vertical(|ui| {
            if let Some(v) = check(
                ui,
                p,
                s.sync_materials,
                false,
                aurora_i18n::tr!("build-ui-texture-synchronize-materials"),
                true,
            ) {
                s.sync_materials = v;
            }
            let mapping = [
                (aurora_i18n::tr!("common-default"), 0u8),
                (aurora_i18n::tr!("build-ui-texture-planar"), TEX_GEN_PLANAR),
            ];
            let (m, m_mixed) = same(sel.iter().map(|x| x.te.media_flags & TEM_TEX_GEN_MASK));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(v) = combo(ui, "texgen", if m_mixed { None } else { m }, &mapping, 90.0, true) {
                    tool.edit_faces(world, |f, _, _| f.media_flags = (f.media_flags & !TEM_TEX_GEN_MASK) | v);
                }
            });
        });
        ui.horizontal(|ui| {
            let mut align = tool.ui.planar_align;
            if ui
                .add_enabled_ui(planar, |ui| {
                    let label = RichText::new(aurora_i18n::tr!("build-ui-texture-align-planar-faces")).size(12.0);
                    crate::ui::widgets::checkbox(ui, p, &mut align, label)
                })
                .inner
                .changed()
            {
                tool.ui.planar_align = align;
            }
            let objects = sel.iter().map(|x| x.idx).collect::<std::collections::BTreeSet<_>>().len();
            if ui
                .add_enabled(
                    planar && tool.ui.planar_align && objects > 1,
                    egui::Button::new(RichText::new(aurora_i18n::tr!("common-align")).size(12.0)),
                )
                .on_hover_text(aurora_i18n::tr!("build-ui-texture-aligns-the-available-texture-layers"))
                .clicked()
            {
                tool.align_planar_faces(world, sel.iter().map(|x| (x.idx, x.face)).collect());
            }
        });
        if tool.ui.tex_tab == 1 {
            ui.add_enabled(false, egui::Button::new(aurora_i18n::tr!("common-save")))
                .on_disabled_hover_text(aurora_i18n::tr!(
                    "build-ui-texture-saving-materials-to-inventory-is-not-available-yet"
                ));
        }
    });
}

/// Whether "Copy" may run (FSPanelFace::updateCopyTexButton): the faces of
/// one object, whose material, if any, may be copied
/// (`texture_copy::material_copy_allowed`).
pub(super) fn copy_allowed(tool: &BuildTool, world: &World, sel: &[Sel]) -> bool {
    let Some(first) = sel.first() else { return false };
    let faces: Vec<u8> = sel.iter().map(|x| x.face).collect();
    sel.iter().all(|x| x.idx == first.idx) && texture_copy::material_copy_allowed(tool, world, first.idx, &faces)
}

/// "Copy" (FSPanelFace::onCopyTexture, LLPanelFace::onCopyTexture): each
/// id with what a paste may count on, and the normal and specular maps the
/// agent may not copy left out, for the target to keep its own.
pub(super) fn copy_faces(tool: &mut BuildTool, env: &Env, world: &World, sel: &[Sel]) -> Vec<FaceClip> {
    let (inv, agent) = (&world.inventory, world.agent_id);
    let Some(source) = sel.first().map(|x| x.idx) else {
        return Vec::new();
    };
    let creator = texture_copy::creator_owned(tool, world, source);
    let avail = |tool: &BuildTool, asset: Uuid| {
        texture_copy::availability(inv, agent, asset, creator, texture_copy::held(tool, world, source, asset))
    };
    sel.iter()
        .map(|x| {
            let mut legacy = tool.face_legacy(env, world, x.idx, x.face, &x.te);
            let (mut normal_kept, mut specular_kept) = (false, false);
            if let Some(m) = legacy.as_mut() {
                if !texture_copy::can_copy(inv, agent, m.normal_map) {
                    m.normal_map = Uuid::nil();
                    normal_kept = true;
                }
                if !texture_copy::can_copy(inv, agent, m.specular_map) {
                    m.specular_map = Uuid::nil();
                    specular_kept = true;
                }
            }
            let gltf = BuildTool::gltf_asset(world, x.idx, x.face).map(|a| (a, BuildTool::gltf_override(world, x.idx, x.face)));
            let (material, maps) = match &gltf {
                Some((asset, over)) => (avail(tool, *asset), over.textures.map(|t| avail(tool, t))),
                None => Default::default(),
            };
            FaceClip {
                face: x.te,
                legacy,
                gltf,
                image: avail(tool, x.te.texture),
                material,
                maps,
                normal_kept,
                specular_kept,
            }
        })
        .collect()
}

/// "Paste" (FSPanelFace::onPasteFaces): the copied faces in order onto
/// the selected ones; a single copied face goes on all of them. The ids
/// follow LLPanelFace::onPasteTexture: a limited one needs its inventory
/// item (else nothing is pasted), which goes into the target's contents
/// when it cannot be transferred; a material whose override maps are not
/// all free stays off the face, the target keeps its own.
fn paste_faces(tool: &mut BuildTool, world: &mut World, env: &Env, sel: &[Sel]) {
    use crate::world::notice_topics::Topic;
    let Some(clip) = tool.ui.face_clipboard.clone() else { return };
    if clip.is_empty() {
        return;
    }
    let pick = |i: usize| if clip.len() == 1 { &clip[0] } else { &clip[i.min(clip.len() - 1)] };
    let ids = |c: &FaceClip| {
        let material = c.gltf.is_some().then(|| std::iter::once(c.material).chain(c.maps));
        std::iter::once(c.image).chain(material.into_iter().flatten()).collect::<Vec<_>>()
    };
    if clip.iter().any(|c| ids(c).iter().any(|a| !a.usable(&world.inventory))) {
        world.system_notice(aurora_i18n::tr!("build-ui-texture-paste-not-in-inventory"), Topic::Items);
        return;
    }
    // handleDropMaterialProtections asks for a target's contents first
    let waiting = {
        let (inv, agent) = (&world.inventory, world.agent_id);
        let moves = |a: &texture_copy::Availability| {
            !a.full
                && a.item
                    .and_then(|id| inv.items.get(&id))
                    .is_some_and(|it| texture_copy::requires_contents(inv, it, agent))
        };
        let mut waiting = false;
        for (i, x) in sel.iter().enumerate() {
            let c = pick(i);
            if (moves(&c.image) || c.gltf.is_some() && moves(&c.material)) && !tool.contents_ready(world, x.idx) {
                waiting = true;
            }
        }
        waiting
    };
    if waiting {
        world.system_notice(aurora_i18n::tr!("build-ui-texture-paste-wait-contents"), Topic::Items);
        return;
    }
    if clip.iter().any(|c| ids(c).iter().any(|a| !a.full)) {
        world.system_notice(aurora_i18n::tr!("build-ui-texture-paste-limited"), Topic::Items);
    }
    let through = |tool: &mut BuildTool, world: &World, idx: usize, a: texture_copy::Availability| {
        a.item
            .is_none_or(|id| tool.retain_for_paste(world, idx, id) == texture_copy::Retain::Done)
    };
    let mut image_ok = Vec::with_capacity(sel.len());
    let mut gltf_ok = Vec::with_capacity(sel.len());
    for (i, x) in sel.iter().enumerate() {
        let c = pick(i);
        image_ok.push(through(tool, world, x.idx, c.image));
        gltf_ok.push(c.gltf.is_none() || c.maps.iter().all(|m| m.full) && through(tool, world, x.idx, c.material));
    }
    let order: Vec<(usize, u8)> = sel.iter().map(|x| (x.idx, x.face)).collect();
    tool.edit_faces(world, |f, idx, face| {
        let i = order.iter().position(|o| *o == (idx, face)).unwrap_or(0);
        let c = pick(i).face;
        // the material id is the simulator's: the legacy material is put below
        let (mat, texture) = (f.material_id, f.texture);
        *f = c;
        f.material_id = mat;
        if !image_ok.get(i).copied().unwrap_or(false) {
            f.texture = texture;
        }
    });
    let mut puts = Vec::new();
    let mut gltf = Vec::new();
    for (i, x) in sel.iter().enumerate() {
        let c = pick(i);
        let mut legacy = c.legacy.clone();
        if let Some(m) = legacy.as_mut()
            && (c.normal_kept || c.specular_kept)
        {
            let own = tool.face_legacy(env, world, x.idx, x.face, &x.te);
            let own = own.as_ref();
            if c.normal_kept {
                m.normal_map = own.map_or(Uuid::nil(), |o| o.normal_map);
            }
            if c.specular_kept {
                m.specular_map = own.map_or(Uuid::nil(), |o| o.specular_map);
            }
        }
        puts.push((x.idx, x.face, legacy));
        if gltf_ok[i] {
            gltf.push(GltfUpdate {
                object: x.idx,
                face: x.face,
                asset: Some(c.gltf.as_ref().map(|g| g.0).unwrap_or_default()),
                over: c.gltf.as_ref().map(|g| g.1.clone()),
            });
        }
    }
    tool.legacy_put(world, puts);
    if !gltf.is_empty() {
        tool.gltf_update(world, gltf);
    }
}

// --------------------------------------------------------------- media

fn media(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &mut Env, sel: &[Sel]) {
    // ask for the media data of the selected objects once
    for idx in sel.iter().map(|x| x.idx).collect::<std::collections::BTreeSet<_>>() {
        if let Some(o) = world.objects.get(idx)
            && o.te
                .as_ref()
                .is_some_and(|te| te.faces.iter().any(|f| f.media_flags & MF_HAS_MEDIA != 0))
            && tool.object_media(env.media, &o.full_id).is_none()
            && !tool.ui.media_asked.contains(&o.full_id)
        {
            tool.ui.media_asked.insert(o.full_id);
            if !env.demo {
                tool.media_get(world, idx);
            }
        }
    }
    let entries: Vec<Option<crate::media::entry::MediaEntry>> = sel
        .iter()
        .map(|x| {
            let id = world.objects.get(x.idx)?.full_id;
            tool.object_media(env.media, &id)?.faces.get(x.face as usize)?.clone()
        })
        .collect();
    let (url, mixed) = same(entries.iter().map(|e| e.as_ref().map(|e| e.home_url.as_str())));
    let text = if mixed {
        aurora_i18n::tr!("build-ui-texture-multiple-media").to_owned()
    } else {
        url.flatten().unwrap_or("").to_owned()
    };
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new(if text.is_empty() {
                    aurora_i18n::tr!("build-ui-texture-the-selected-media-url-if-any-goes-here")
                } else {
                    &text
                })
                .size(12.0)
                .color(if text.is_empty() { p.muted_dim } else { p.ink }),
            );
        });
    let has_media = entries.iter().any(|e| e.is_some());
    ui.horizontal(|ui| {
        if ui
            .button(RichText::new(aurora_i18n::tr!("build-ui-texture-choose-2")).size(12.0))
            .clicked()
        {
            let first = entries.iter().flatten().next().cloned();
            tool.ui.media_settings = Some(super::MediaSettingsState::new(first));
        }
        if ui
            .add_enabled(
                has_media,
                egui::Button::new(RichText::new(aurora_i18n::tr!("common-delete")).size(12.0)),
            )
            .clicked()
        {
            tool.ui.confirm = Some(Confirm::DeleteMedia);
        }
        if ui
            .add_enabled(
                has_media,
                egui::Button::new(RichText::new(aurora_i18n::tr!("common-align")).size(12.0)),
            )
            .on_hover_text(aurora_i18n::tr!("build-ui-texture-fit-media-texture-to-face"))
            .clicked()
        {
            media_autofix(tool, world, env, sel);
        }
    });
}

/// "Align" of the Media tab (FSPanelFace::onClickAutoFix): scale the
/// face's texture to the media inside its power-of-two texture.
fn media_autofix(tool: &mut BuildTool, world: &mut World, env: &mut Env, sel: &[Sel]) {
    let sizes: Vec<((usize, u8), (f32, f32))> = sel
        .iter()
        .filter_map(|x| {
            let id = world.objects.get(x.idx)?.full_id;
            let m = env.media.find(&crate::media::MediaKey::Prim { object: id, face: x.face })?;
            let ((mw, mh), (tw, th)) = m.sizes()?;
            (tw > 0 && th > 0).then(|| ((x.idx, x.face), (mw as f32 / tw as f32, mh as f32 / th as f32)))
        })
        .collect();
    tool.edit_faces(world, |f, idx, face| {
        if let Some((_, (ss, st))) = sizes.iter().find(|(k, _)| *k == (idx, face)) {
            f.scale_s = *ss;
            f.scale_t = *st;
            f.offset_s = -(1.0 - ss) / 2.0;
            f.offset_t = -(1.0 - st) / 2.0;
        }
    });
}

impl BuildTool {
    /// Apply the media settings to the selected faces (LLSelectMgr::
    /// selectionSetMedia): media flag on each face, ObjectImage, then the
    /// ObjectMedia UPDATE of each object with every face's entry.
    pub fn apply_media(&mut self, world: &mut World, env: &mut Env, entry: Option<crate::media::entry::MediaEntry>) {
        let sel = selected(self, world);
        let on = entry.is_some();
        self.edit_faces(world, |f, _, _| {
            f.media_flags = if on {
                f.media_flags | MF_HAS_MEDIA
            } else {
                f.media_flags & !MF_HAS_MEDIA
            };
        });
        let mut by_object: std::collections::BTreeMap<usize, Vec<u8>> = Default::default();
        for x in &sel {
            by_object.entry(x.idx).or_default().push(x.face);
        }
        for (idx, faces) in by_object {
            let Some(o) = world.objects.get(idx) else { continue };
            let id = o.full_id;
            let n = self.num_faces(world, idx);
            let mut data = self.object_media(env.media, &id).cloned().unwrap_or_default();
            data.faces.resize(n, None);
            for f in faces {
                if let Some(slot) = data.faces.get_mut(f as usize) {
                    *slot = entry.clone();
                }
            }
            data.version += 1;
            self.media_update(world, idx, &data.faces);
            self.ui.media_data.insert(id, data.clone());
            env.media.set_object_media(id, data, env.demo);
        }
    }

    /// "Align planar faces": the planar-mapped faces take the
    /// repeats, rotation and offsets of the last selected one. Deviation:
    /// LLFace::calcAlignedPlanarTE also shifts the offsets by the faces'
    /// positions so that coplanar faces of different objects line up.
    pub fn align_planar_faces(&mut self, world: &mut World, faces: Vec<(usize, u8)>) {
        let Some(&(ref_idx, ref_face)) = faces.last() else { return };
        let Some(r) = world
            .objects
            .get(ref_idx)
            .and_then(|o| o.te.as_ref())
            .map(|te| *te.face(ref_face as usize))
        else {
            return;
        };
        self.edit_faces(world, |f, _, _| {
            f.scale_s = r.scale_s;
            f.scale_t = r.scale_t;
            f.rotation = r.rotation;
            f.offset_s = r.offset_s;
            f.offset_t = r.offset_t;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbr_conversion_identity() {
        let t = convert_to_pbr([1.0, 1.0], [0.0, 0.0], 0.0);
        assert!((t.offset[0]).abs() < 1e-6 && (t.offset[1]).abs() < 1e-6);
        assert_eq!(t.scale, [1.0, 1.0]);
        let t = convert_to_pbr([2.0, 4.0], [0.0, 0.0], std::f32::consts::FRAC_PI_2);
        assert!((t.scale[0] - 4.0).abs() < 1e-5 && (t.scale[1] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn st_axes() {
        assert_eq!(te_st_axes(0), (0, 1));
        assert_eq!(te_st_axes(2), (1, 2));
        assert_eq!(te_st_axes(7), (0, 1));
    }
}
