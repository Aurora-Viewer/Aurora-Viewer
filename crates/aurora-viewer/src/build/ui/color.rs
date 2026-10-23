//! LLColorSwatchCtrl / LLSelectMgr color transactions: preview selected
//! faces, accept, or restore each face's original color on Cancel.
//! Firestorm llpanelface.cpp / llselectmgr.cpp, originally LGPL 2.1.

use super::{
    Env,
    common::{linear_to_srgb, srgb_to_linear},
};
use crate::build::{BuildSettings, BuildTool, edits::Extra, materials::GltfUpdate};
use crate::scene::legacy_mat::LegacyMaterial;
use crate::theme::Palette;
use crate::ui::color_picker::{Event, Rgb};
use crate::world::{World, objects::ObjKey};
use aurora_assets::material::PbrOverride;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTarget {
    Diffuse,
    PbrBase,
    PbrEmissive,
    Specular,
    Light(Uuid),
}

enum Original {
    Diffuse(Rgb),
    Pbr { over: PbrOverride, rgb: Rgb, alpha: f32 },
    Specular(Option<LegacyMaterial>),
    Light(Rgb),
}

struct Saved {
    object: Uuid,
    face: u8,
    original: Original,
}

pub(super) struct Session {
    target: ColorTarget,
    saved: Vec<Saved>,
    selection: Vec<ObjKey>,
    faces: HashMap<ObjKey, Vec<u8>>,
    applied: bool,
}

impl Original {
    fn rgb(&self) -> Rgb {
        match self {
            Self::Diffuse(rgb) | Self::Pbr { rgb, .. } => *rgb,
            Self::Light(rgb) => rgb.map(linear_to_srgb),
            Self::Specular(mat) => mat
                .as_ref()
                .map(|m| {
                    [
                        m.specular_color[0] as f32 / 255.0,
                        m.specular_color[1] as f32 / 255.0,
                        m.specular_color[2] as f32 / 255.0,
                    ]
                })
                .unwrap_or([1.0; 3]),
        }
    }
}

impl BuildTool {
    pub(in crate::build) fn color_highlighted(&self, world: &World) -> Vec<(usize, bool)> {
        let mut prims = self.highlighted(world);
        if self.ui.color_picker.pipette
            && let Some(idx) = self.ui.color_picker.sampled.and_then(|id| world.objects.index_of_uuid(&id))
            && !prims.iter().any(|(i, _)| *i == idx)
        {
            prims.push((idx, true));
        }
        prims
    }

    pub(super) fn color_swatch_response(
        &mut self,
        world: &mut World,
        env: &mut Env,
        target: ColorTarget,
        rgb: Rgb,
        swatch: egui::Response,
    ) {
        if swatch.clicked() {
            self.toggle_color_picker(world, env, target, rgb);
        }
        if self.ui.color_picker.is_open() && self.ui.color_edit.as_ref().is_some_and(|session| session.target == target) {
            self.ui.color_picker.set_origin(swatch.rect);
        }
    }

    pub(super) fn toggle_color_picker(&mut self, world: &mut World, env: &mut Env, target: ColorTarget, rgb: Rgb) {
        // Unlike LLColorSwatchCtrl::showPicker, a repeated click closes the
        // same transaction instead of resetting the original color snapshot.
        let same = self.ui.color_picker.is_open()
            && self
                .ui
                .color_edit
                .as_ref()
                .is_some_and(|session| session.target == target && session.selection == self.selection && session.faces == self.faces);
        self.cancel_color_picker(world);
        if same {
            return;
        }
        let mut saved = Vec::new();
        if let ColorTarget::Light(id) = target {
            if let Some(idx) = world.objects.index_of_uuid(&id)
                && self.can_modify(world, idx)
                && let Some(light) = world.objects.get(idx).and_then(|o| o.extra.light)
            {
                saved.push(Saved {
                    object: id,
                    face: 0,
                    original: Original::Light([light.color[0], light.color[1], light.color[2]]),
                });
            }
        } else {
            for (idx, face) in self.sel_te(world) {
                if !self.can_modify(world, idx) {
                    continue;
                }
                let Some(object) = world.objects.get(idx) else {
                    continue;
                };
                let Some(te) = object.te.as_ref().and_then(|te| te.faces.get(face as usize)) else {
                    continue;
                };
                let original = match target {
                    ColorTarget::Diffuse => Original::Diffuse([te.color[0], te.color[1], te.color[2]]),
                    ColorTarget::Specular => Original::Specular(self.face_legacy(env, world, idx, face, te)),
                    ColorTarget::PbrBase | ColorTarget::PbrEmissive => {
                        let Some(asset) = Self::gltf_asset(world, idx, face) else {
                            continue;
                        };
                        let over = Self::gltf_override(world, idx, face);
                        let mut material = env.scene.materials.get(&asset).map(|m| (*m).clone()).unwrap_or_default();
                        material.apply_override(&over);
                        let rgb = if target == ColorTarget::PbrBase {
                            [
                                material.base_color_factor[0],
                                material.base_color_factor[1],
                                material.base_color_factor[2],
                            ]
                        } else {
                            material.emissive_factor
                        };
                        Original::Pbr {
                            over,
                            rgb: rgb.map(linear_to_srgb),
                            alpha: material.base_color_factor[3],
                        }
                    }
                    ColorTarget::Light(_) => continue,
                };
                saved.push(Saved {
                    object: object.full_id,
                    face,
                    original,
                });
            }
        }
        if saved.is_empty() {
            return;
        }
        let mixed = saved.iter().any(|s| s.original.rgb() != saved[0].original.rgb());
        self.ui.color_picker.open_from(rgb, mixed, egui::Id::unique(super::INSPECTOR_ID));
        self.ui.color_edit = Some(Session {
            target,
            saved,
            selection: self.selection.clone(),
            faces: self.faces.clone(),
            applied: false,
        });
    }

    pub fn cancel_color_picker(&mut self, world: &mut World) {
        self.ui.color_picker.close();
        let Some(session) = self.ui.color_edit.take() else {
            return;
        };
        self.discard_color_edit();
        if session.applied {
            self.set_session_color(world, &session, None);
        }
    }

    fn color_event(&mut self, world: &mut World, event: Event) {
        if event == Event::Cancel {
            self.cancel_color_picker(world);
            return;
        }
        let Some(mut session) = self.ui.color_edit.take() else {
            return;
        };
        match event {
            Event::Preview(rgb) => {
                self.preview_color_edit(|tool| session.applied |= tool.set_session_color(world, &session, Some(rgb)));
                self.ui.color_edit = Some(session);
            }
            Event::Accept(rgb) => {
                self.preview_color_edit(|tool| {
                    tool.set_session_color(world, &session, Some(rgb));
                });
                self.commit_color_edit();
            }
            Event::Cancel => {}
        }
    }

    /// A dialog owns its original objects/faces, never a later selection.
    /// Only the RGB property is changed/restored: alpha, transforms and other
    /// edits made while the dialog was open keep their current values.
    fn set_session_color(&mut self, world: &mut World, session: &Session, rgb: Option<Rgb>) -> bool {
        let linear = rgb.map(|rgb| rgb.map(srgb_to_linear));
        let bytes = rgb.map(|rgb| rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
        let mut textures = HashSet::new();
        let mut gltf = Vec::new();
        let mut legacy = Vec::new();
        let mut gltf_changed = false;
        let mut legacy_changed = false;
        let mut changed = false;
        for saved in &session.saved {
            let Some(idx) = world.objects.index_of_uuid(&saved.object) else {
                continue;
            };
            if !self.can_modify(world, idx) {
                continue;
            }
            match &saved.original {
                Original::Diffuse(original) => {
                    let Some(object) = world.objects.get_mut(idx) else {
                        continue;
                    };
                    let Some(mut te) = object.te.as_deref().cloned() else {
                        continue;
                    };
                    let Some(face) = te.faces.get_mut(saved.face as usize) else {
                        continue;
                    };
                    let color = rgb.unwrap_or(*original);
                    if face.color[..3] != color {
                        face.color[..3].copy_from_slice(&color);
                        object.te = Some(Arc::new(te));
                        object.material_dirty = true;
                        object.render.needs_records = true;
                        textures.insert(idx);
                        changed = true;
                    }
                }
                Original::Pbr {
                    over: original,
                    alpha: original_alpha,
                    ..
                } => {
                    if Self::gltf_asset(world, idx, saved.face).is_none() {
                        continue;
                    }
                    let mut over = Self::gltf_override(world, idx, saved.face);
                    let before = over.clone();
                    if session.target == ColorTarget::PbrBase {
                        let alpha = over.base_color_factor.map(|c| c[3]).unwrap_or(*original_alpha);
                        over.base_color_factor = if let Some(linear) = linear {
                            Some([linear[0], linear[1], linear[2], alpha])
                        } else if let Some(c) = original.base_color_factor {
                            Some([c[0], c[1], c[2], alpha])
                        } else if alpha == *original_alpha {
                            None
                        } else {
                            let Original::Pbr { rgb, .. } = &saved.original else {
                                continue;
                            };
                            let c = rgb.map(srgb_to_linear);
                            Some([c[0], c[1], c[2], alpha])
                        };
                    } else {
                        over.emissive_factor = linear.or(original.emissive_factor);
                    }
                    gltf_changed |= over != before;
                    gltf.push(GltfUpdate {
                        object: idx,
                        face: saved.face,
                        asset: None,
                        over: Some(over),
                    });
                }
                Original::Specular(original) => {
                    let Some(te) = world
                        .objects
                        .get(idx)
                        .and_then(|o| o.te.as_ref())
                        .and_then(|t| t.faces.get(saved.face as usize))
                    else {
                        continue;
                    };
                    let current = self
                        .ui
                        .legacy_pending
                        .get(&(saved.object, saved.face))
                        .and_then(|(_, m)| m.clone())
                        .or_else(|| original.clone());
                    let mut material = current.clone().unwrap_or_default();
                    let color = bytes.unwrap_or_else(|| {
                        original
                            .as_ref()
                            .map(|m| [m.specular_color[0], m.specular_color[1], m.specular_color[2]])
                            .unwrap_or([255; 3])
                    });
                    material.specular_color[..3].copy_from_slice(&color);
                    let material = if original.is_none() && rgb.is_none() {
                        None
                    } else {
                        Some(material)
                    };
                    legacy_changed |= current != material;
                    self.ui
                        .legacy_pending
                        .insert((saved.object, saved.face), (te.material_id, material.clone()));
                    legacy.push((idx, saved.face, material));
                }
                Original::Light(original) => {
                    let Some(mut light) = world.objects.get(idx).and_then(|o| o.extra.light) else {
                        continue;
                    };
                    let color = linear.unwrap_or(*original);
                    if light.color[..3] != color {
                        light.color[..3].copy_from_slice(&color);
                        self.set_extra(world, idx, Extra::Light(Some(light)));
                        changed = true;
                    }
                }
            }
        }
        for idx in textures {
            self.send_te(world, idx);
        }
        // Always replace an entire region's material batch, including faces
        // whose RGB already equals the final value; no pending face is lost.
        if gltf_changed {
            self.gltf_update(world, gltf);
            changed = true;
        }
        if legacy_changed {
            self.legacy_put(world, legacy);
            changed = true;
        }
        changed
    }
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, settings: &mut BuildSettings) {
    let valid = tool.ui.color_edit.as_ref().is_none_or(|session| {
        session.selection == tool.selection
            && session.faces == tool.faces
            && session.saved.iter().all(|s| {
                world
                    .objects
                    .index_of_uuid(&s.object)
                    .is_some_and(|idx| tool.can_modify(world, idx))
            })
    });
    if !valid || tool.ui.picker.open {
        tool.cancel_color_picker(world);
    }
    if let Some(event) = tool.ui.color_picker.show(ctx, p, &mut settings.color_picker) {
        tool.color_event(world, event);
    }
}

pub(super) fn finish_gesture(ctx: &egui::Context, tool: &mut BuildTool) {
    let (focused, held) = ctx.input(|i| (i.focused, i.pointer.any_down()));
    // Losing focus ends a real gesture once; an unfocused window must not
    // flush every intermediate event of an offline scripted drag.
    if !held || !focused && tool.ui.color_focused {
        tool.commit_color_edit();
    }
    tool.ui.color_focused = focused;
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::{NetCommand, build::BuildCmd};

    fn fixture(target: ColorTarget) -> (World, BuildTool, Vec<usize>) {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        for event in crate::demo::events() {
            if matches!(event, aurora_net::NetEvent::ObjectUpdates { .. }) {
                world.apply(event);
            }
        }
        let prims: Vec<_> = world
            .objects
            .iter()
            .filter(|(_, o)| matches!(o.key.local_id, 118 | 119))
            .map(|(i, _)| i)
            .collect();
        let mut tool = BuildTool::default();
        let mut saved = Vec::new();
        for (n, &idx) in prims.iter().enumerate() {
            let o = world.objects.get_mut(idx).unwrap();
            o.owner_id = world.agent_id;
            let mut te = o.te.as_deref().unwrap().clone();
            let rgb = [0.1 + n as f32 * 0.2, 0.4, 0.7];
            te.faces[0].color = [rgb[0], rgb[1], rgb[2], 0.4];
            o.te = Some(Arc::new(te));
            tool.selection.push(o.key);
            tool.faces.insert(o.key, vec![0]);
            saved.push(Saved {
                object: o.full_id,
                face: 0,
                original: Original::Diffuse(rgb),
            });
        }
        tool.ui.color_edit = Some(Session {
            target,
            saved,
            selection: tool.selection.clone(),
            faces: tool.faces.clone(),
            applied: false,
        });
        tool.ui.color_picker.open([1.0; 3], true);
        (world, tool, prims)
    }

    #[test]
    fn clicking_the_same_swatch_closes_and_restores_but_another_target_opens_its_own_draft() {
        let (mut world, mut tool, prims) = fixture(ColorTarget::Diffuse);
        let originals: Vec<_> = prims
            .iter()
            .map(|&i| world.objects.get(i).unwrap().te.as_ref().unwrap().clone())
            .collect();
        let images = HashMap::new();
        let mut media = crate::media::MediaManager::default();
        let mut scene = crate::scene::Scene::new(std::env::temp_dir(), Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut env = Env {
            images: &images,
            media: &mut media,
            scene: &mut scene,
            zoom_fraction: 0.0,
            property_lines: &mut false,
            demo: true,
        };
        tool.color_event(&mut world, Event::Preview([1.0, 0.0, 0.0]));
        tool.toggle_color_picker(&mut world, &mut env, ColorTarget::Diffuse, [1.0, 0.0, 0.0]);
        assert!(!tool.ui.color_picker.is_open());
        assert!(tool.ui.color_edit.is_none());
        assert!(tool.color_preview.is_empty());
        for (&idx, original) in prims.iter().zip(&originals) {
            assert_eq!(
                world.objects.get(idx).unwrap().te.as_ref().unwrap().faces[0].color,
                original.faces[0].color
            );
        }
        tool.toggle_color_picker(&mut world, &mut env, ColorTarget::Diffuse, [0.1, 0.4, 0.7]);
        assert!(tool.ui.color_picker.is_open());
        tool.color_event(&mut world, Event::Preview([0.0, 1.0, 0.0]));
        tool.toggle_color_picker(&mut world, &mut env, ColorTarget::Specular, [1.0; 3]);
        assert!(tool.ui.color_picker.is_open());
        assert_eq!(tool.ui.color_edit.as_ref().unwrap().target, ColorTarget::Specular);
        for (&idx, original) in prims.iter().zip(&originals) {
            assert_eq!(
                world.objects.get(idx).unwrap().te.as_ref().unwrap().faces[0].color,
                original.faces[0].color
            );
        }
        tool.toggle_color_picker(&mut world, &mut env, ColorTarget::Specular, [1.0; 3]);
        assert!(!tool.ui.color_picker.is_open());
    }

    #[test]
    fn cancel_restores_each_mixed_face_and_preserves_alpha_and_other_faces() {
        let (mut world, mut tool, prims) = fixture(ColorTarget::Diffuse);
        let originals: Vec<_> = prims
            .iter()
            .map(|&i| world.objects.get(i).unwrap().te.as_ref().unwrap().clone())
            .collect();
        for n in 0..200 {
            tool.color_event(&mut world, Event::Preview([n as f32 / 200.0, 0.2, 0.3]));
            assert!(tool.out.is_empty());
        }
        assert_eq!(tool.color_preview.len(), 2);
        tool.commit_color_edit();
        assert_eq!(tool.out.len(), 2);
        tool.out.clear();
        // Another control changes transparency while the color dialog is open.
        tool.edit_faces(&mut world, |face, _, _| face.color[3] = 0.9);
        tool.out.clear();
        tool.color_event(&mut world, Event::Cancel);
        assert!(!tool.ui.color_picker.is_open());
        assert_eq!(tool.out.len(), 2);
        for (&idx, original) in prims.iter().zip(originals) {
            let te = world.objects.get(idx).unwrap().te.as_ref().unwrap();
            assert_eq!(te.face(0).color[..3], original.face(0).color[..3]);
            assert_eq!(te.face(0).color[3], 0.9);
            assert_eq!(te.face(1), original.face(1));
        }
    }

    #[test]
    fn accepting_a_prepared_color_sends_once_and_cancel_drops_unsent_previews() {
        let (mut world, mut tool, _) = fixture(ColorTarget::Diffuse);
        tool.color_event(&mut world, Event::Accept([0.2, 0.3, 0.4]));
        assert_eq!(tool.out.len(), 2);
        assert!(tool.ui.color_edit.is_none());
        let (mut world, mut tool, _) = fixture(ColorTarget::Diffuse);
        tool.color_event(&mut world, Event::Preview([1.0; 3]));
        tool.color_event(&mut world, Event::Cancel);
        assert!(tool.color_preview.is_empty());
        assert_eq!(tool.out.len(), 2);
        assert!(
            tool.out
                .iter()
                .all(|c| matches!(c, NetCommand::Build(BuildCmd::SetTextures { .. })))
        );
    }

    #[test]
    fn pbr_cancel_restores_unset_and_mixed_overrides_without_losing_other_edits() {
        for target in [ColorTarget::PbrBase, ColorTarget::PbrEmissive] {
            let (mut world, mut tool, prims) = fixture(target);
            for (n, &idx) in prims.iter().enumerate() {
                world.objects.get_mut(idx).unwrap().extra.render_materials = vec![(0, Uuid::from_u128(9000))];
                let original = PbrOverride {
                    base_color_factor: (n == 1).then_some([0.2, 0.3, 0.4, 0.6]),
                    emissive_factor: (n == 1).then_some([0.1, 0.2, 0.3]),
                    ..Default::default()
                };
                let key = world.objects.get(idx).unwrap().key;
                world.gltf_overrides.insert(key, Arc::from([(0, original.clone())]));
                tool.ui.color_edit.as_mut().unwrap().saved[n].original = Original::Pbr {
                    over: original,
                    rgb: [1.0; 3],
                    alpha: if n == 0 { 1.0 } else { 0.6 },
                };
            }
            tool.color_event(&mut world, Event::Preview([0.5, 0.25, 0.75]));
            assert!(tool.out.is_empty());
            assert_eq!(tool.color_preview.len(), 1);
            for &idx in &prims {
                let mut over = BuildTool::gltf_override(&world, idx, 0);
                over.metallic_factor = Some(0.123);
                if idx == prims[1] {
                    over.base_color_factor.as_mut().unwrap()[3] = 0.8;
                }
                let key = world.objects.get(idx).unwrap().key;
                world.gltf_overrides.insert(key, Arc::from([(0, over)]));
            }
            tool.color_event(&mut world, Event::Cancel);
            assert!(tool.color_preview.is_empty());
            assert_eq!(tool.out.len(), 1);
            let a = BuildTool::gltf_override(&world, prims[0], 0);
            let b = BuildTool::gltf_override(&world, prims[1], 0);
            assert_eq!(a.metallic_factor, Some(0.123));
            assert_eq!(b.metallic_factor, Some(0.123));
            assert_eq!(b.base_color_factor.unwrap()[3], 0.8);
            if target == ColorTarget::PbrBase {
                assert_eq!(a.base_color_factor, None);
                assert_eq!(b.base_color_factor, Some([0.2, 0.3, 0.4, 0.8]));
            } else {
                assert_eq!(a.emissive_factor, None);
                assert_eq!(b.emissive_factor, Some([0.1, 0.2, 0.3]));
            }
        }
    }

    #[test]
    fn escape_stops_pipette_then_cancels_dialog_before_closing_build_tools() {
        let (mut world, mut tool, _) = fixture(ColorTarget::Diffuse);
        let ctx = egui::Context::default();
        let mut settings = BuildSettings::default();
        tool.open = true;
        tool.ui.color_picker.pipette = true;
        let selection = tool.selection.clone();
        for step in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    focused: true,
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| {
                    super::super::shortcuts(ui.ctx(), &mut tool, &mut world, &mut settings);
                    show(
                        ui.ctx(),
                        &crate::theme::Theme::default().palette(),
                        &mut tool,
                        &mut world,
                        &mut settings,
                    );
                },
            );
            output.textures_delta.clear();
            if step < 2 {
                assert!(tool.open);
                assert_eq!(tool.selection, selection);
                assert!(!tool.ui.color_picker.pipette);
                assert_eq!(tool.ui.color_picker.is_open(), step == 0);
            } else {
                assert!(!tool.open);
            }
            // Distinct Escape presses, not a held key's automatic repeats.
            let mut output = ctx.run_ui(
                egui::RawInput {
                    focused: true,
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| {
                    super::super::shortcuts(ui.ctx(), &mut tool, &mut world, &mut settings);
                    show(
                        ui.ctx(),
                        &crate::theme::Theme::default().palette(),
                        &mut tool,
                        &mut world,
                        &mut settings,
                    );
                },
            );
            output.textures_delta.clear();
        }
    }

    #[test]
    fn unfocused_scripted_drags_coalesce_and_real_focus_loss_flushes_once() {
        let (mut world, mut tool, _) = fixture(ColorTarget::Diffuse);
        let ctx = egui::Context::default();
        let pointer = |pressed| egui::Event::PointerButton {
            pos: egui::pos2(40.0, 50.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for n in 0..100 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    focused: false,
                    events: if n == 0 { vec![pointer(true)] } else { vec![] },
                    ..Default::default()
                },
                |ui| {
                    tool.color_event(&mut world, Event::Preview([n as f32 / 100.0, 0.2, 0.3]));
                    finish_gesture(ui.ctx(), &mut tool);
                },
            );
            output.textures_delta.clear();
            assert!(tool.out.is_empty(), "unfocused script must not flush every preview");
        }
        let mut output = ctx.run_ui(
            egui::RawInput {
                focused: false,
                events: vec![pointer(false)],
                ..Default::default()
            },
            |ui| finish_gesture(ui.ctx(), &mut tool),
        );
        output.textures_delta.clear();
        assert_eq!(tool.out.len(), 2);
        tool.out.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                focused: true,
                events: vec![pointer(true)],
                ..Default::default()
            },
            |ui| {
                tool.color_event(&mut world, Event::Preview([0.4, 0.5, 0.6]));
                finish_gesture(ui.ctx(), &mut tool);
            },
        );
        output.textures_delta.clear();
        assert!(tool.out.is_empty());
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    focused: false,
                    ..Default::default()
                },
                |ui| finish_gesture(ui.ctx(), &mut tool),
            );
            output.textures_delta.clear();
        }
        assert_eq!(tool.out.len(), 2);
    }

    #[test]
    fn specular_cancel_preserves_glossiness_and_restores_each_faces_rgb() {
        let (mut world, mut tool, prims) = fixture(ColorTarget::Specular);
        for (n, &idx) in prims.iter().enumerate() {
            let original = LegacyMaterial {
                specular_map: Uuid::from_u128(9000),
                specular_color: [40 + n as u8 * 30, 60, 80, 255],
                ..Default::default()
            };
            let o = world.objects.get(idx).unwrap();
            tool.ui
                .legacy_pending
                .insert((o.full_id, 0), (o.te.as_ref().unwrap().face(0).material_id, Some(original.clone())));
            tool.ui.color_edit.as_mut().unwrap().saved[n].original = Original::Specular(Some(original));
        }
        tool.color_event(&mut world, Event::Preview([0.5, 0.25, 0.75]));
        assert!(tool.out.is_empty());
        assert_eq!(tool.color_preview.len(), 1);
        for (_, material) in tool.ui.legacy_pending.values_mut() {
            material.as_mut().unwrap().specular_exp = 123;
        }
        tool.color_event(&mut world, Event::Cancel);
        assert_eq!(tool.out.len(), 1);
        for (n, &idx) in prims.iter().enumerate() {
            let id = world.objects.get(idx).unwrap().full_id;
            let material = tool.ui.legacy_pending[&(id, 0)].1.as_ref().unwrap();
            assert_eq!(material.specular_color, [40 + n as u8 * 30, 60, 80, 255]);
            assert_eq!(material.specular_exp, 123);
        }
    }

    #[test]
    fn light_cancel_restores_linear_rgb_and_keeps_current_intensity_and_radius() {
        let (mut world, mut tool, prims) = fixture(ColorTarget::Diffuse);
        let idx = prims[0];
        let id = world.objects.get(idx).unwrap().full_id;
        let rgb = [0.1, 0.2, 0.3];
        world.objects.get_mut(idx).unwrap().extra.light = Some(aurora_prim::extra::LightParams {
            color: [rgb[0], rgb[1], rgb[2], 0.4],
            ..crate::build::edits::LIGHT_DEFAULT
        });
        let session = tool.ui.color_edit.as_mut().unwrap();
        session.target = ColorTarget::Light(id);
        session.saved = vec![Saved {
            object: id,
            face: 0,
            original: Original::Light(rgb),
        }];
        tool.color_event(&mut world, Event::Preview([0.5, 0.25, 0.75]));
        let light = world.objects.get_mut(idx).unwrap().extra.light.as_mut().unwrap();
        light.color[3] = 0.8;
        light.radius = 9.0;
        tool.color_event(&mut world, Event::Cancel);
        let light = world.objects.get(idx).unwrap().extra.light.unwrap();
        assert_eq!(light.color, [0.1, 0.2, 0.3, 0.8]);
        assert_eq!(light.radius, 9.0);
        assert_eq!(tool.out.len(), 1);
    }
}
