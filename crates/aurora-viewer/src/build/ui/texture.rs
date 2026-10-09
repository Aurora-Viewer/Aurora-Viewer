//! Texture tab (FSPanelFace, indra/newview/fspanelface.cpp and
//! panel_fs_tools_texture.xml of Firestorm, originally LGPL 2.1): the PBR,
//! Blinn-Phong and Media sub-tabs of the selected faces, copy / paste of
//! face parameters and the face options under them (repeats per meter,
//! hide water, mapping, material sync).
//!
//! Every value is read over the selected faces: when they differ the
//! control shows a mixed value (« Multiples »).

use super::common::{Edit, check, color_swatch, combo, label, row, same, small, spin, swatch};
use super::{Confirm, Env, FaceClip, PickTarget};
use crate::build::materials::{GltfUpdate, MF_HAS_MEDIA};
use crate::build::{BuildSettings, BuildTool};
use crate::scene::legacy_mat::{LegacyMaterial, alpha_mode};
use crate::theme::Palette;
use crate::ui::widgets::tabs;
use crate::world::World;
use aurora_assets::material::{AlphaMode, OVERRIDE_NULL_UUID, PbrOverride, TextureTransform};
use aurora_prim::te::{DEFAULT_TEXTURE, TextureFace};
use egui::RichText;
use uuid::Uuid;

/// IMG_ALPHA_GRAD: the diffuse of a « hide water » face.
const IMG_ALPHA_GRAD: Uuid = Uuid::from_u128(0xe97cf410_8e61_7005_ec06_629eba4cd1fb);
/// Default normal map of the pickers.
pub const DEFAULT_NORMAL: Uuid = Uuid::from_u128(0x85f28839_7a1c_b4e3_d71d_967792970a7b);
/// Default specular map.
pub const DEFAULT_SPECULAR: Uuid = Uuid::from_u128(0x87e0e8f7_8729_1ea8_cfc9_8915773009db);

/// « Relief » (bump codes 0..17).
const BUMPS: [&str; 18] = [
    "Aucun",
    "Brillance",
    "Sombreté",
    "grain de bois",
    "écorce",
    "briques",
    "damier",
    "béton",
    "croûteux",
    "pierre de taille",
    "disques",
    "gravier",
    "pétrin",
    "bardage",
    "carrelage en pierre",
    "stuc",
    "aspiration",
    "trame",
];
/// Index of « Utiliser la texture » (BUMPY_TEXTURE / SHINY_TEXTURE).
const BUMPY_TEXTURE: u8 = 18;
const SHINY_TEXTURE: u8 = 4;
const SHINES: [&str; 4] = ["Aucune", "Faible", "Moyenne", "Elevée"];
const ALPHA_MODES: [(&str, u8); 4] = [
    ("Aucun", alpha_mode::NONE),
    ("Fusion alpha", alpha_mode::BLEND),
    ("Masquage alpha", alpha_mode::MASK),
    ("Masque émissif", alpha_mode::EMISSIVE),
];

/// TEM_* bits of the bump / shiny / fullbright byte.
const TEM_BUMP_MASK: u8 = 0x1f;
const TEM_FULLBRIGHT: u8 = 0x20;
const TEM_SHINY_SHIFT: u8 = 6;
/// Texgen bits of the media byte (0 default, 2 planar).
const TEM_TEX_GEN_MASK: u8 = 0x06;
const TEX_GEN_PLANAR: u8 = 0x02;

/// One selected face.
#[derive(Debug, Clone, Copy)]
struct Sel {
    idx: usize,
    face: u8,
    te: TextureFace,
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
    /// Blinn-Phong material of a face as the panel sees it: the one just
    /// sent (until the simulator's new material id arrives), else the
    /// region's, else none.
    fn face_legacy(&self, env: &Env, world: &World, idx: usize, face: u8, te: &TextureFace) -> Option<LegacyMaterial> {
        let id = world.objects.get(idx)?.full_id;
        if let Some((old, m)) = self.ui.legacy_pending.get(&(id, face))
            && *old == te.material_id
        {
            return m.clone();
        }
        if te.material_id.is_nil() {
            return None;
        }
        env.scene.legacy_mats.peek(&te.material_id).map(|m| (*m).clone())
    }

    /// Change the Blinn-Phong material of the selected faces (LLSelectedTEMaterial::
    /// set*): the default material without maps is removed instead.
    fn edit_legacy(&mut self, env: &Env, world: &World, sel: &[Sel], f: impl Fn(&mut LegacyMaterial)) {
        let mut puts = Vec::new();
        for s in sel {
            let mut m = self.face_legacy(env, world, s.idx, s.face, &s.te).unwrap_or_default();
            f(&mut m);
            let default_mode = if env.scene.textures.has_alpha(&s.te.texture).unwrap_or(false) {
                alpha_mode::BLEND
            } else {
                alpha_mode::NONE
            };
            let empty = m.normal_map.is_nil() && m.specular_map.is_nil() && m.diffuse_alpha_mode == default_mode;
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
        let updates: Vec<GltfUpdate> = sel
            .iter()
            .filter(|s| BuildTool::gltf_asset(world, s.idx, s.face).is_some())
            .map(|s| {
                let mut o = BuildTool::gltf_override(world, s.idx, s.face);
                f(&mut o);
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

    /// A texture chosen in the picker for the Texture tab.
    pub fn on_texture_picked(&mut self, world: &mut World, env: &mut Env, target: PickTarget, id: Uuid) {
        let sel = selected(self, world);
        match target {
            PickTarget::Diffuse => self.edit_faces(world, |f, _, _| f.texture = id),
            PickTarget::Normal => self.edit_legacy(env, world, &sel, |m| m.normal_map = id),
            PickTarget::Specular => self.edit_legacy(env, world, &sel, |m| m.specular_map = id),
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
                self.edit_gltf(world, &sel, |o| o.textures[i] = tex);
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

/// The selected faces of modifiable volume prims.
fn selected(tool: &BuildTool, world: &World) -> Vec<Sel> {
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
    let sel = selected(tool, world);
    if sel.is_empty() {
        small(ui, p, "Aucune face modifiable sélectionnée.");
        return;
    }
    // ---- sub-tabs, copy / paste, glow
    ui.horizontal(|ui| {
        let mut t = tool.ui.tex_tab;
        tabs(ui, p, &mut t, &[("PBR", true), ("Blinn-Phong", true), ("Media", true)]);
        tool.ui.tex_tab = t;
    });
    ui.horizontal(|ui| {
        let (glow, glow_mixed) = same(sel.iter().map(|x| x.te.glow));
        let mut g = glow.unwrap_or(0.0);
        label(ui, p, "Lueur");
        if spin(ui, &mut g, 0.1, 0.0..=1.0, 3, W, glow_mixed).commit {
            tool.edit_faces(world, |f, _, _| f.glow = g);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if super::common::icon_button(
                ui,
                p,
                "clipboard-text",
                "Coller les paramètres de texture à partir du presse-papiers",
                tool.ui.face_clipboard.is_some(),
            ) {
                paste_faces(tool, world, &sel);
            }
            let one_object = sel.iter().all(|x| x.idx == sel[0].idx);
            if super::common::icon_button(ui, p, "copy", "Copier les paramètres de texture dans le presse-papiers", one_object) {
                let clip = sel
                    .iter()
                    .map(|x| FaceClip {
                        face: x.te,
                        legacy: tool.face_legacy(env, world, x.idx, x.face, &x.te),
                        gltf: BuildTool::gltf_asset(world, x.idx, x.face).map(|a| (a, BuildTool::gltf_override(world, x.idx, x.face))),
                    })
                    .collect();
                tool.ui.face_clipboard = Some(clip);
            }
        });
    });
    ui.add_space(4.0);
    match tool.ui.tex_tab {
        0 => pbr(ui, p, tool, world, env, &sel),
        1 => blinn_phong(ui, p, tool, world, s, env, &sel),
        _ => media(ui, p, tool, world, env, &sel),
    }
    if tool.ui.tex_tab != 2 {
        ui.separator();
        face_options(ui, p, tool, world, s, env, &sel);
    }
}

// ---------------------------------------------------------------- PBR

fn pbr(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &mut Env, sel: &[Sel]) {
    let (asset, asset_mixed) = same(sel.iter().map(|x| BuildTool::gltf_asset(world, x.idx, x.face)));
    let asset = asset.flatten();
    let has_all = sel.iter().all(|x| BuildTool::gltf_asset(world, x.idx, x.face).is_some());
    // effective material of the first face: base asset + override
    let first = sel[0];
    let over = BuildTool::gltf_override(world, first.idx, first.face);
    let mut mat = asset
        .and_then(|a| env.scene.materials.get(&a))
        .map(|m| (*m).clone())
        .unwrap_or_default();
    mat.apply_override(&over);
    let editable = has_all;
    ui.horizontal(|ui| {
        let r = swatch(
            ui,
            p,
            env.images,
            &mut tool.ui.wanted_images,
            None,
            "Matériau",
            56.0,
            asset_mixed,
            true,
        );
        if asset.is_some() {
            ui.painter().text(
                r.rect.center(),
                egui::Align2::CENTER_CENTER,
                "GLTF",
                egui::FontId::proportional(11.0),
                p.violet_light,
            );
        }
        if r.on_hover_text("Choisir un matériau PBR (« Vierge » : aucun)").clicked() {
            tool.ui.picker.open_materials("Choisir : matériau", asset.unwrap_or_default());
            tool.ui.pick_target = Some(PickTarget::PbrMaterial);
        }
        let maps: [(Option<Uuid>, &str, usize); 2] = [(mat.base_color_texture, "Couleur", 0), (mat.normal_texture, "Normale", 1)];
        for (id, cap, i) in maps {
            if swatch(ui, p, env.images, &mut tool.ui.wanted_images, id, cap, 56.0, false, editable).clicked() {
                tool.ui.picker.open(&format!("Choisir : {cap}"), id.unwrap_or_default());
                tool.ui.pick_target = Some(PickTarget::PbrMap(i));
            }
        }
        let bc = mat.base_color_factor;
        let srgb = [
            super::common::linear_to_srgb(bc[0]),
            super::common::linear_to_srgb(bc[1]),
            super::common::linear_to_srgb(bc[2]),
        ];
        if let Some(c) = color_swatch(ui, p, srgb, "Teinte", 40.0, editable) {
            let lin = [
                super::common::srgb_to_linear(c[0]),
                super::common::srgb_to_linear(c[1]),
                super::common::srgb_to_linear(c[2]),
            ];
            tool.edit_gltf(world, sel, |o| o.base_color_factor = Some([lin[0], lin[1], lin[2], bc[3]]));
        }
    });
    ui.horizontal(|ui| {
        let maps: [(Option<Uuid>, &str, usize); 2] = [(mat.emissive_texture, "Émissive", 3), (mat.metallic_roughness_texture, "(O)RM", 2)];
        for (id, cap, i) in maps {
            let r = swatch(ui, p, env.images, &mut tool.ui.wanted_images, id, cap, 56.0, false, editable);
            let r = if i == 2 {
                r.on_hover_text("Cliquez pour choisir la carte de rugosité-métallicité, qui peut contenir un canal d'occlusion optionnel.")
            } else {
                r
            };
            if r.clicked() {
                tool.ui.picker.open(&format!("Choisir : {cap}"), id.unwrap_or_default());
                tool.ui.pick_target = Some(PickTarget::PbrMap(i));
            }
        }
        let e = mat.emissive_factor;
        let srgb = [
            super::common::linear_to_srgb(e[0]),
            super::common::linear_to_srgb(e[1]),
            super::common::linear_to_srgb(e[2]),
        ];
        if let Some(c) = color_swatch(ui, p, srgb, "Teinte", 40.0, editable) {
            let lin = [
                super::common::srgb_to_linear(c[0]),
                super::common::srgb_to_linear(c[1]),
                super::common::srgb_to_linear(c[2]),
            ];
            tool.edit_gltf(world, sel, |o| o.emissive_factor = Some(lin));
        }
    });
    ui.add_enabled_ui(editable, |ui| {
        if let Some(v) = check(ui, mat.double_sided, false, "Double face", editable) {
            tool.edit_gltf(world, sel, |o| o.double_sided = Some(v));
        }
        let mut alpha = mat.base_color_factor[3];
        let bc = mat.base_color_factor;
        row(ui, p, "Alpha", 80.0, |ui| {
            if spin(ui, &mut alpha, 0.1, 0.0..=1.0, 3, W, false).commit {
                tool.edit_gltf(world, sel, |o| o.base_color_factor = Some([bc[0], bc[1], bc[2], alpha]));
            }
        });
        let modes = [
            ("Opaque", AlphaMode::Opaque),
            ("Fusion", AlphaMode::Blend),
            ("Masquage", AlphaMode::Mask),
        ];
        row(ui, p, "Mode", 80.0, |ui| {
            if let Some(m) = combo(ui, "pbr_alpha_mode", Some(mat.alpha_mode), &modes, 100.0, editable) {
                tool.edit_gltf(world, sel, |o| o.alpha_mode = Some(m));
            }
        });
        for (name, v, which) in [
            ("Coupure", mat.alpha_cutoff, 0),
            ("Métallicité", mat.metallic_factor, 1),
            ("Rugosité", mat.roughness_factor, 2),
        ] {
            let mut x = v;
            row(ui, p, name, 80.0, |ui| {
                let enabled = which != 0 || mat.alpha_mode == AlphaMode::Mask;
                ui.add_enabled_ui(enabled, |ui| {
                    if spin(ui, &mut x, 0.1, 0.0..=1.0, 3, W, false).commit {
                        tool.edit_gltf(world, sel, |o| match which {
                            0 => o.alpha_cutoff = Some(x),
                            1 => o.metallic_factor = Some(x),
                            _ => o.roughness_factor = Some(x),
                        });
                    }
                });
            });
        }
        // « Enregistrer » (LLMaterialEditor::saveObjectsMaterialAs): needs the
        // material upload to the inventory, not ported yet
        ui.add_enabled(false, egui::Button::new(RichText::new("Enregistrer").size(12.0)))
            .on_disabled_hover_text("L'enregistrement du matériau dans l'inventaire n'est pas encore disponible.");
    });
    // texture transforms
    ui.add_space(4.0);
    let mut ch = tool.ui.pbr_channel;
    tabs(
        ui,
        p,
        &mut ch,
        &[
            ("Tous", true),
            ("Couleur", true),
            ("Normale", true),
            ("(O)RM", true),
            ("Émissive", true),
        ],
    );
    tool.ui.pbr_channel = ch;
    // « Tous » shows the base color's, mixed when the channels differ;
    // the others in the panel's order: base, normal, ORM, emissive
    let (t, mixed) = if ch == 0 {
        (mat.transforms[0], mat.transforms.iter().any(|x| *x != mat.transforms[0]))
    } else {
        (mat.transforms[ch - 1], false)
    };
    let mut nt = t;
    let mut rot = nt.rotation.to_degrees();
    let mut e = Edit::default();
    let mut flip = [false; 2];
    ui.add_enabled_ui(editable, |ui| {
        ui.horizontal(|ui| {
            label(ui, p, "Échelle");
            for k in 0..2 {
                ui.label(RichText::new(["U", "V"][k]).size(11.0));
                e = e.or(spin(ui, &mut nt.scale[k], 0.1, -10000.0..=10000.0, 5, W, mixed));
                if super::common::icon_button(ui, p, "arrows-left-right", "Pivoter", true) {
                    flip[k] = true;
                }
            }
        });
        ui.horizontal(|ui| {
            label(ui, p, "Décalage");
            for k in 0..2 {
                ui.label(RichText::new(["U", "V"][k]).size(11.0));
                e = e.or(spin(ui, &mut nt.offset[k], 0.01, -1.0..=1.0, 5, W, mixed));
            }
        });
        row(ui, p, "Rotation", 60.0, |ui| {
            e = e.or(spin(ui, &mut rot, 1.0, -360.0..=360.0, 2, W, mixed))
        });
    });
    for (k, f) in flip.iter().enumerate() {
        if *f {
            nt.scale[k] = -nt.scale[k];
            e.commit = true;
        }
    }
    nt.rotation = rot.to_radians();
    if e.commit && nt != t {
        tool.edit_gltf(world, sel, |o| {
            if ch == 0 {
                o.transforms = [nt; 4];
            } else {
                o.transforms[ch - 1] = nt;
            }
        });
    }
}

// --------------------------------------------------------- Blinn-Phong

#[allow(clippy::too_many_arguments)]
fn blinn_phong(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env, sel: &[Sel]) {
    let legacy: Vec<Option<LegacyMaterial>> = sel.iter().map(|x| tool.face_legacy(env, world, x.idx, x.face, &x.te)).collect();
    let (tex, tex_mixed) = same(sel.iter().map(|x| x.te.texture));
    let (normal, normal_mixed) = same(legacy.iter().map(|m| m.as_ref().map(|m| m.normal_map).unwrap_or_default()));
    let (spec, spec_mixed) = same(legacy.iter().map(|m| m.as_ref().map(|m| m.specular_map).unwrap_or_default()));
    let normal = normal.filter(|n| !n.is_nil());
    let spec = spec.filter(|n| !n.is_nil());
    let water = sel.iter().all(|x| is_hide_water(&x.te, legacy[0].as_ref()));
    let editable = !water;
    ui.horizontal(|ui| {
        if swatch(
            ui,
            p,
            env.images,
            &mut tool.ui.wanted_images,
            tex,
            "Diffuse",
            56.0,
            tex_mixed,
            editable,
        )
        .clicked()
        {
            tool.ui
                .picker
                .open_with_defaults("Choisir : texture", tex.unwrap_or_default(), Some(DEFAULT_TEXTURE), false);
            tool.ui.pick_target = Some(PickTarget::Diffuse);
        }
        let (col, _) = same(sel.iter().map(|x| [x.te.color[0], x.te.color[1], x.te.color[2]]));
        let col = col.unwrap_or([1.0; 3]);
        if let Some(c) = color_swatch(ui, p, col, "Teinte", 40.0, editable) {
            // selectionSetColorOnly: the alpha stays
            tool.edit_faces(world, |f, _, _| {
                f.color[0] = c[0];
                f.color[1] = c[1];
                f.color[2] = c[2];
            });
        }
        ui.vertical(|ui| {
            let (fb, fb_mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright & TEM_FULLBRIGHT != 0));
            if let Some(v) = check(ui, fb.unwrap_or(false), fb_mixed, "Lumineux", editable) {
                tool.edit_faces(world, |f, _, _| {
                    f.bump_shiny_fullbright = if v {
                        f.bump_shiny_fullbright | TEM_FULLBRIGHT
                    } else {
                        f.bump_shiny_fullbright & !TEM_FULLBRIGHT
                    }
                });
            }
            let (alpha, alpha_mixed) = same(sel.iter().map(|x| x.te.color[3]));
            let mut transp = (1.0 - alpha.unwrap_or(1.0)) * 100.0;
            row(ui, p, "Transparence", 80.0, |ui| {
                if spin(ui, &mut transp, 2.0, 0.0..=100.0, 0, 52.0, alpha_mixed).commit {
                    let a = (100.0 - transp) / 100.0;
                    tool.edit_faces(world, |f, _, _| f.color[3] = a);
                }
                ui.label("%");
            });
        });
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
    row(ui, p, "Mode alpha", 90.0, |ui| {
        let current = if mode_mixed { None } else { mode };
        if let Some(m) = combo(
            ui,
            "bp_alpha_mode",
            current,
            &ALPHA_MODES,
            120.0,
            editable && has_alpha && transp_zero,
        ) {
            tool.edit_legacy(env, world, sel, |x| x.diffuse_alpha_mode = m);
        }
    });
    let (cut, cut_mixed) = same(legacy.iter().map(|m| m.as_ref().map(|m| m.alpha_cutoff as i32).unwrap_or(127)));
    let mut cut = cut.unwrap_or(127) as f32;
    row(ui, p, "Coupe masque", 90.0, |ui| {
        ui.add_enabled_ui(editable && mode == Some(alpha_mode::MASK), |ui| {
            if spin(ui, &mut cut, 1.0, 0.0..=255.0, 0, W, cut_mixed).commit {
                tool.edit_legacy(env, world, sel, |x| x.alpha_cutoff = cut as u8);
            }
        });
    });
    ui.add_space(4.0);
    // normal map, bump
    ui.horizontal(|ui| {
        if swatch(
            ui,
            p,
            env.images,
            &mut tool.ui.wanted_images,
            normal,
            "Normale",
            56.0,
            normal_mixed,
            editable,
        )
        .clicked()
        {
            tool.ui
                .picker
                .open_with_defaults("Choisir : normale", normal.unwrap_or_default(), Some(DEFAULT_NORMAL), true);
            tool.ui.pick_target = Some(PickTarget::Normal);
        }
        ui.vertical(|ui| {
            let (bump, bump_mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright & TEM_BUMP_MASK));
            let mut items: Vec<(&str, u8)> = BUMPS.iter().enumerate().map(|(i, n)| (*n, i as u8)).collect();
            let shown = if normal.is_some() {
                items.push(("Utiliser la texture", BUMPY_TEXTURE));
                Some(BUMPY_TEXTURE)
            } else if bump_mixed {
                None
            } else {
                bump
            };
            row(ui, p, "Relief", 70.0, |ui| {
                if let Some(b) = combo(ui, "bumpiness", shown, &items, 120.0, editable)
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
            let mut items: Vec<(&str, u8)> = SHINES.iter().enumerate().map(|(i, n)| (*n, i as u8)).collect();
            let shown = if spec.is_some() {
                items.push(("Utiliser la texture", SHINY_TEXTURE));
                Some(SHINY_TEXTURE)
            } else if shiny_mixed {
                None
            } else {
                shiny
            };
            row(ui, p, "Luminosité", 70.0, |ui| {
                if let Some(v) = combo(ui, "shininess", shown, &items, 120.0, editable)
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
        });
    });
    // specular map, its color, glossiness, environment
    ui.horizontal(|ui| {
        if swatch(
            ui,
            p,
            env.images,
            &mut tool.ui.wanted_images,
            spec,
            "Spéculaire",
            56.0,
            spec_mixed,
            editable,
        )
        .clicked()
        {
            tool.ui
                .picker
                .open_with_defaults("Choisir : spéculaire", spec.unwrap_or_default(), Some(DEFAULT_SPECULAR), true);
            tool.ui.pick_target = Some(PickTarget::Specular);
        }
        let sc = legacy[0].as_ref().map(|m| m.specular_color).unwrap_or([255; 4]);
        let rgb = [sc[0] as f32 / 255.0, sc[1] as f32 / 255.0, sc[2] as f32 / 255.0];
        if let Some(c) = color_swatch(ui, p, rgb, "Teinte", 40.0, editable && spec.is_some()) {
            let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            tool.edit_legacy(env, world, sel, |m| m.specular_color = [b(c[0]), b(c[1]), b(c[2]), sc[3]]);
        }
        ui.vertical(|ui| {
            let (gl, gl_mixed) = same(legacy.iter().map(|m| m.as_ref().map(|m| m.specular_exp).unwrap_or(51)));
            let (env_i, env_mixed) = same(legacy.iter().map(|m| m.as_ref().map(|m| m.env_intensity).unwrap_or(0)));
            let mut g = gl.unwrap_or(51) as f32;
            let mut en = env_i.unwrap_or(0) as f32;
            ui.add_enabled_ui(editable && spec.is_some(), |ui| {
                row(ui, p, "Brillance", 90.0, |ui| {
                    if spin(ui, &mut g, 1.0, 0.0..=255.0, 0, W, gl_mixed).commit {
                        tool.edit_legacy(env, world, sel, |m| m.specular_exp = g as u8);
                    }
                });
                row(ui, p, "Environnement", 90.0, |ui| {
                    if spin(ui, &mut en, 1.0, 0.0..=255.0, 0, W, env_mixed).commit {
                        tool.edit_legacy(env, world, sel, |m| m.env_intensity = en as u8);
                    }
                });
            });
        });
    });
    // UV of the diffuse / normal / specular maps
    ui.add_space(4.0);
    let mut ch = tool.ui.bp_channel;
    tabs(
        ui,
        p,
        &mut ch,
        &[("Diffuse", true), ("Normale", normal.is_some()), ("Spéculaire", spec.is_some())],
    );
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
    let vals: Vec<[f32; 5]> = sel.iter().zip(&legacy).map(|(x, m)| read(x, m)).collect();
    let mixed = |i: usize| vals.iter().any(|v| (v[i] - vals[0][i]).abs() > 1e-5);
    let mut v = vals[0];
    v[0] *= k;
    v[1] *= k;
    v[4] = v[4].to_degrees();
    let before = v;
    let decimals = s.decimal_precision.min(7) as usize;
    let mut e = Edit::default();
    let mut flip = [false; 2];
    ui.add_enabled_ui(editable, |ui| {
        ui.horizontal(|ui| {
            label(ui, p, "Échelle");
            for i in 0..2 {
                ui.label(RichText::new(["H", "V"][i]).size(11.0));
                e = e.or(spin(ui, &mut v[i], 0.1, -10000.0..=10000.0, decimals, W, mixed(i)));
                if super::common::icon_button(ui, p, "arrows-left-right", "Pivoter", true) {
                    flip[i] = true;
                }
            }
        });
        ui.horizontal(|ui| {
            label(ui, p, "Décalage");
            for i in 2..4 {
                ui.label(RichText::new(["H", "V"][i - 2]).size(11.0));
                e = e.or(spin(ui, &mut v[i], 0.01, -1.0..=1.0, decimals, W, mixed(i)));
            }
        });
        row(ui, p, "Rotation", 60.0, |ui| {
            e = e.or(spin(ui, &mut v[4], 1.0, -360.0..=360.0, decimals.min(2), W, mixed(4)))
        });
    });
    for (i, f) in flip.iter().enumerate() {
        if *f {
            v[i] = -v[i];
            e.commit = true;
        }
    }
    if e.commit && v != before {
        // only the values that moved are written (non-tentative spinners)
        let changed: Vec<usize> = (0..5).filter(|&i| (v[i] - before[i]).abs() > 1e-6).collect();
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
}

/// A « hide water » face (FSPanelFace: alpha gradient diffuse, opaque,
/// no maps).
fn is_hide_water(te: &TextureFace, m: Option<&LegacyMaterial>) -> bool {
    te.texture == IMG_ALPHA_GRAD && te.color[3] >= 1.0 && m.is_none_or(|m| m.normal_map.is_nil() && m.specular_map.is_nil())
}

/// Options under the PBR / Blinn-Phong sub-tabs.
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
    ui.horizontal(|ui| {
        // repeats per meter: max(scale_s / object s size, scale_t / object t size)
        let rpm_of = |x: &Sel| -> f32 {
            let sc = world.objects.get(x.idx).map(|o| o.scale).unwrap_or(glam::Vec3::ONE);
            let (sa, ta) = te_st_axes(x.face);
            (x.te.scale_s / sc[sa].max(0.001)).max(x.te.scale_t / sc[ta].max(0.001))
        };
        let (rpm, rpm_mixed) = same(sel.iter().map(|x| (rpm_of(x) * 10.0).round() / 10.0));
        let mut r = rpm.unwrap_or(1.0);
        label(ui, p, "Répétitions / mètre");
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
        let legacy0 = tool.face_legacy(env, world, sel[0].idx, sel[0].face, &sel[0].te);
        let water = sel.iter().all(|x| is_hide_water(&x.te, legacy0.as_ref()));
        if let Some(v) = check(ui, water, false, "Occulter eau", true) {
            if v {
                tool.ui.confirm = Some(Confirm::HideWater);
            } else {
                tool.edit_faces(world, |f, _, _| f.texture = DEFAULT_TEXTURE);
            }
        }
    });
    ui.horizontal(|ui| {
        if let Some(v) = check(ui, s.sync_materials, false, "Synchroniser les matériaux", true) {
            s.sync_materials = v;
        }
        let mapping = [("Défaut", 0u8), ("Planaire", TEX_GEN_PLANAR)];
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
            .add_enabled(
                planar,
                egui::Checkbox::new(&mut align, RichText::new("Aligner les faces planaires").size(12.0)),
            )
            .changed()
        {
            tool.ui.planar_align = align;
        }
        let objects = sel.iter().map(|x| x.idx).collect::<std::collections::BTreeSet<_>>().len();
        if ui
            .add_enabled(
                planar && tool.ui.planar_align && objects > 1,
                egui::Button::new(RichText::new("Aligner").size(12.0)),
            )
            .on_hover_text("Aligne les couches de texture présentes")
            .clicked()
        {
            tool.align_planar_faces(world, sel.iter().map(|x| (x.idx, x.face)).collect());
        }
    });
}

/// « Coller » (FSPanelFace::onPasteFaces): the copied faces in order onto
/// the selected ones; a single copied face goes on all of them.
fn paste_faces(tool: &mut BuildTool, world: &mut World, sel: &[Sel]) {
    let Some(clip) = tool.ui.face_clipboard.clone() else { return };
    if clip.is_empty() {
        return;
    }
    let pick = |i: usize| if clip.len() == 1 { &clip[0] } else { &clip[i.min(clip.len() - 1)] };
    let order: Vec<(usize, u8)> = sel.iter().map(|x| (x.idx, x.face)).collect();
    tool.edit_faces(world, |f, idx, face| {
        let i = order.iter().position(|o| *o == (idx, face)).unwrap_or(0);
        let c = pick(i).face;
        // the material id is the simulator's: the legacy material is put below
        let mat = f.material_id;
        *f = c;
        f.material_id = mat;
    });
    let mut puts = Vec::new();
    let mut gltf = Vec::new();
    for (i, x) in sel.iter().enumerate() {
        let c = pick(i);
        puts.push((x.idx, x.face, c.legacy.clone()));
        gltf.push(GltfUpdate {
            object: x.idx,
            face: x.face,
            asset: Some(c.gltf.as_ref().map(|g| g.0).unwrap_or_default()),
            over: c.gltf.as_ref().map(|g| g.1.clone()),
        });
    }
    tool.legacy_put(world, puts);
    tool.gltf_update(world, gltf);
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
        "Médias multiples".to_owned()
    } else {
        url.flatten().unwrap_or("").to_owned()
    };
    egui::Frame::new().fill(p.field).corner_radius(2).inner_margin(6).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.label(
            RichText::new(if text.is_empty() {
                "L'URL du média choisi, s'il y en a un, va ici"
            } else {
                &text
            })
            .size(12.0)
            .color(if text.is_empty() { p.muted_dim } else { p.ink }),
        );
    });
    let has_media = entries.iter().any(|e| e.is_some());
    ui.horizontal(|ui| {
        if ui.button(RichText::new("Choisir...").size(12.0)).clicked() {
            let first = entries.iter().flatten().next().cloned();
            tool.ui.media_settings = Some(super::MediaSettingsState::new(first));
        }
        if ui
            .add_enabled(has_media, egui::Button::new(RichText::new("Supprimer").size(12.0)))
            .clicked()
        {
            tool.ui.confirm = Some(Confirm::DeleteMedia);
        }
        if ui
            .add_enabled(has_media, egui::Button::new(RichText::new("Aligner").size(12.0)))
            .on_hover_text("Ajuster la texture du média à la face")
            .clicked()
        {
            media_autofix(tool, world, env, sel);
        }
    });
}

/// « Aligner » of the Media tab (FSPanelFace::onClickAutoFix): scale the
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

    /// « Aligner les faces planaires »: the planar-mapped faces take the
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
