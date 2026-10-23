//! LLTextureCtrl TEXTURE_CHANGE / SELECT / CANCEL and LLSelectMgr's saved
//! textures (Firestorm lltexturectrl.cpp / llselectmgr.cpp, LGPL 2.1).
//! A live picker owns its original faces and can restore mixed selections.

use super::{Env, PickTarget, texture::Sel};
use crate::build::{BuildTool, edits::Extra, materials::GltfUpdate};
use crate::scene::legacy_mat::LegacyMaterial;
use crate::theme::Palette;
use crate::world::{
    World,
    objects::{LocalAppearance, ObjKey},
};
use aurora_assets::material::PbrOverride;
use aurora_prim::te::TextureFace;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

enum Original {
    Diffuse(Uuid),
    Legacy(Option<LegacyMaterial>),
    Material { asset: Uuid, over: PbrOverride },
    Map(Uuid),
    Sculpt(Uuid),
    Light(Option<aurora_prim::extra::LightImageParams>),
}

struct Saved {
    object: Uuid,
    face: u8,
    original: Original,
    local: Option<LocalAppearance>,
}

pub(super) struct Session {
    generation: u64,
    target: PickTarget,
    selection: Vec<ObjKey>,
    faces: HashMap<ObjKey, Vec<u8>>,
    linked: bool,
    saved: Vec<Saved>,
    last_preview: Option<Uuid>,
    remote_changed: bool,
}

impl Session {
    fn capture(tool: &mut BuildTool, world: &World, env: &Env, target: PickTarget) -> Self {
        let mut saved = Vec::new();
        for s in super::texture::selected(tool, world) {
            let Some(object) = world.objects.get(s.idx) else { continue };
            let original = match target {
                PickTarget::Diffuse => Original::Diffuse(s.te.texture),
                PickTarget::Normal | PickTarget::Specular => Original::Legacy(tool.face_legacy(env, world, s.idx, s.face, &s.te)),
                PickTarget::PbrMaterial => Original::Material {
                    asset: BuildTool::gltf_asset(world, s.idx, s.face).unwrap_or_default(),
                    over: BuildTool::gltf_override(world, s.idx, s.face),
                },
                PickTarget::PbrMap(i) => {
                    if BuildTool::gltf_asset(world, s.idx, s.face).is_none() {
                        continue;
                    }
                    Original::Map(BuildTool::gltf_override(world, s.idx, s.face).textures[i])
                }
                PickTarget::Sculpt => {
                    let Some(sculpt) = object.extra.sculpt else { continue };
                    Original::Sculpt(sculpt.texture)
                }
                PickTarget::LightTexture => Original::Light(object.extra.light_image),
            };
            saved.push(Saved {
                object: object.full_id,
                face: s.face,
                original,
                local: object.local_appearance(s.face as usize),
            });
            // Object parameters, unlike texture entries, affect one prim.
            if matches!(target, PickTarget::Sculpt | PickTarget::LightTexture) {
                break;
            }
        }
        Self {
            generation: tool.ui.picker.generation,
            target,
            selection: tool.selection.clone(),
            faces: tool.faces.clone(),
            linked: tool.selection_linked,
            saved,
            last_preview: None,
            remote_changed: false,
        }
    }

    fn resolve(&self, tool: &BuildTool, world: &World) -> Vec<Sel> {
        self.saved
            .iter()
            .filter_map(|saved| {
                let idx = world.objects.index_of_uuid(&saved.object)?;
                if !tool.can_modify(world, idx) {
                    return None;
                }
                let te = *world.objects.get(idx)?.te.as_ref()?.faces.get(saved.face as usize)?;
                Some(Sel { idx, face: saved.face, te })
            })
            .collect()
    }
}

impl BuildTool {
    fn set_local_picker_appearance(&mut self, world: &mut World, sel: &[Sel], appearance: Option<LocalAppearance>) {
        for s in sel {
            let Some(object) = world.objects.get_mut(s.idx) else { continue };
            if object.local_appearance(s.face as usize) == appearance {
                continue;
            }
            // Split a whole-object binding before editing one face, keeping
            // every other face unchanged and recording the equivalent links.
            if let Some(all) = object.render.local_appearance.remove(&None) {
                self.local_appearance_changes.push((object.full_id, None, None));
                for face in 0..object.te.as_ref().map_or(0, |te| te.faces.len()).min(256) {
                    let value = *object.render.local_appearance.entry(Some(face as u8)).or_insert(all);
                    self.local_appearance_changes.push((object.full_id, Some(face as u8), Some(value)));
                }
            }
            match appearance {
                Some(value) => {
                    object.render.local_appearance.insert(Some(s.face), value);
                }
                None => {
                    object.render.local_appearance.remove(&Some(s.face));
                }
            }
            object.material_dirty = true;
            object.render.needs_records = true;
            self.local_appearance_changes.push((object.full_id, Some(s.face), appearance));
        }
    }

    pub(super) fn set_diffuse(&mut self, world: &mut World, sel: &[Sel], mut edit: impl FnMut(&Sel, &mut TextureFace)) {
        let mut changed = HashSet::new();
        for s in sel {
            if !self.can_modify(world, s.idx) {
                continue;
            }
            let Some(object) = world.objects.get_mut(s.idx) else { continue };
            let Some(mut te) = object.te.as_deref().cloned() else { continue };
            let Some(face) = te.faces.get_mut(s.face as usize) else { continue };
            let before = *face;
            edit(s, face);
            if *face != before {
                object.te = Some(Arc::new(te));
                object.material_dirty = true;
                object.render.needs_records = true;
                changed.insert(s.idx);
            }
        }
        for idx in changed {
            self.send_te(world, idx);
        }
    }

    fn preview_texture(&mut self, world: &mut World, env: &mut Env, id: Uuid) {
        let Some(mut session) = self.ui.texture_edit.take() else { return };
        // OK after a preview confirms it without sending it a second time.
        if session.last_preview != Some(id) {
            let sel = session.resolve(self, world);
            if let Some((_, material)) = world.local_assets.get(&id) {
                let appearance = match (session.target, *material) {
                    (PickTarget::Diffuse, false) => Some(LocalAppearance::Texture(id)),
                    (PickTarget::PbrMaterial, true) => Some(LocalAppearance::Material(id)),
                    _ => None,
                };
                if let Some(appearance) = appearance {
                    self.set_local_picker_appearance(world, &sel, Some(appearance));
                    session.last_preview = Some(id);
                }
            } else {
                if matches!(session.target, PickTarget::Diffuse | PickTarget::PbrMaterial) {
                    self.set_local_picker_appearance(world, &sel, None);
                }
                if session.target == PickTarget::LightTexture
                    && let Some(s) = sel.first()
                    && let Some(saved) = session.saved.first_mut()
                    && let Original::Light(Some(original)) = &mut saved.original
                {
                    // Keep the projection settings across None -> another
                    // texture in the same dialog, as LLVOVolume retains its
                    // parameter block when PARAMS_LIGHT_IMAGE is disabled.
                    if let Some(current) = world.objects.get(s.idx).and_then(|o| o.extra.light_image) {
                        original.params = current.params;
                    }
                    self.set_extra(
                        world,
                        s.idx,
                        Extra::LightImage((!id.is_nil()).then_some(aurora_prim::extra::LightImageParams { texture: id, ..*original })),
                    );
                } else {
                    self.apply_texture(world, env, session.target, id, &sel);
                }
                session.last_preview = Some(id);
                session.remote_changed = true;
            }
        }
        self.ui.texture_edit = Some(session);
    }

    pub(super) fn cancel_texture_picker(&mut self, world: &mut World, env: &mut Env) {
        self.ui.picker.open = false;
        self.ui.pick_target = None;
        self.restore_texture_preview(world, env);
    }

    fn restore_texture_preview(&mut self, world: &mut World, env: &mut Env) {
        let Some(session) = self.ui.texture_edit.take() else { return };
        if session.last_preview.is_none() {
            return;
        }
        if matches!(session.target, PickTarget::Diffuse | PickTarget::PbrMaterial) {
            for s in session.resolve(self, world) {
                let object = world.objects.get(s.idx).map(|object| object.full_id);
                if let Some(saved) = session
                    .saved
                    .iter()
                    .find(|saved| Some(saved.object) == object && saved.face == s.face)
                {
                    self.set_local_picker_appearance(world, std::slice::from_ref(&s), saved.local);
                }
            }
        }
        if !session.remote_changed {
            return;
        }
        if session.target == PickTarget::Diffuse {
            let originals: HashMap<_, _> = session
                .saved
                .iter()
                .filter_map(|saved| {
                    if let Original::Diffuse(id) = saved.original {
                        world.objects.index_of_uuid(&saved.object).map(|idx| ((idx, saved.face), id))
                    } else {
                        None
                    }
                })
                .collect();
            let sel = session.resolve(self, world);
            self.set_diffuse(world, &sel, |s, face| {
                if let Some(id) = originals.get(&(s.idx, s.face)) {
                    face.texture = *id;
                }
            });
            return;
        }
        let mut gltf = Vec::new();
        let mut legacy = Vec::new();
        let mut textures = HashSet::new();
        for saved in &session.saved {
            let Some(idx) = world.objects.index_of_uuid(&saved.object) else {
                continue;
            };
            if !self.can_modify(world, idx) {
                continue;
            }
            let Some(te) = world
                .objects
                .get(idx)
                .and_then(|o| o.te.as_ref())
                .and_then(|te| te.faces.get(saved.face as usize))
                .copied()
            else {
                continue;
            };
            match &saved.original {
                Original::Diffuse(_) => {}
                Original::Legacy(original) => {
                    let current = self.face_legacy(env, world, idx, saved.face, &te);
                    let default = super::texture::default_legacy(env, &te);
                    let mut material = current.clone().unwrap_or_else(|| default.clone());
                    let old = original.clone().unwrap_or_else(|| default.clone());
                    if session.target == PickTarget::Normal {
                        material.normal_map = old.normal_map;
                    } else {
                        material.specular_map = old.specular_map;
                    }
                    // Remove a preview-created material, keeping unrelated edits.
                    let material = if original.is_none() && material == default {
                        None
                    } else {
                        Some(material)
                    };
                    if current != material {
                        self.ui
                            .legacy_pending
                            .insert((saved.object, saved.face), (te.material_id, material.clone()));
                        legacy.push((idx, saved.face, material));
                    }
                }
                Original::Material { asset, over } => {
                    gltf.push(GltfUpdate {
                        object: idx,
                        face: saved.face,
                        asset: Some(*asset),
                        over: Some(over.clone()),
                    });
                    textures.insert(idx);
                }
                Original::Map(texture) => {
                    let PickTarget::PbrMap(channel) = session.target else { continue };
                    if BuildTool::gltf_asset(world, idx, saved.face).is_some() {
                        let mut over = BuildTool::gltf_override(world, idx, saved.face);
                        if over.textures[channel] != *texture {
                            over.textures[channel] = *texture;
                            gltf.push(GltfUpdate {
                                object: idx,
                                face: saved.face,
                                asset: None,
                                over: Some(over),
                            });
                        }
                    }
                }
                Original::Sculpt(texture) => {
                    if let Some(mut sculpt) = world.objects.get(idx).and_then(|o| o.extra.sculpt)
                        && sculpt.texture != *texture
                    {
                        sculpt.texture = *texture;
                        self.set_extra(world, idx, Extra::Sculpt(Some(sculpt)));
                    }
                }
                Original::Light(original) => {
                    let current = world.objects.get(idx).and_then(|o| o.extra.light_image);
                    let restored = original.map(|original| aurora_prim::extra::LightImageParams {
                        texture: original.texture,
                        // Clearing removes the parameter block; Cancel must
                        // recover its settings as well as its texture.
                        ..current.unwrap_or(original)
                    });
                    if current != restored {
                        self.set_extra(world, idx, Extra::LightImage(restored));
                    }
                }
            }
        }
        if !gltf.is_empty() {
            self.gltf_update(world, gltf);
        }
        for idx in textures {
            self.send_te(world, idx);
        }
        if !legacy.is_empty() {
            self.legacy_put(world, legacy);
        }
    }
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &mut Env) {
    let reopened = tool
        .ui
        .texture_edit
        .as_ref()
        .is_some_and(|s| s.generation != tool.ui.picker.generation);
    let changed = tool
        .ui
        .texture_edit
        .as_ref()
        .is_some_and(|s| s.selection != tool.selection || s.faces != tool.faces || s.linked != tool.selection_linked);
    if reopened {
        tool.restore_texture_preview(world, env);
    } else if changed || !tool.ui.picker.open {
        tool.cancel_texture_picker(world, env);
    }
    if tool.ui.picker.open
        && tool.ui.texture_edit.is_none()
        && let Some(target) = tool.ui.pick_target
    {
        tool.ui.texture_edit = Some(Session::capture(tool, world, env, target));
    }
    tool.ui.picker.allow_local = matches!(tool.ui.pick_target, Some(PickTarget::Diffuse | PickTarget::PbrMaterial));
    tool.ui.picker.prepare_material_preview(ctx, env.scene);
    let accepted = tool.ui.picker.show(ctx, p, world, env.images);
    if let Some(id) = accepted {
        tool.preview_texture(world, env, id);
        tool.ui.texture_edit = None;
        tool.ui.pick_target = None;
    } else if !tool.ui.picker.open {
        tool.cancel_texture_picker(world, env);
    } else if let Some(id) = tool.ui.picker.take_preview() {
        tool.preview_texture(world, env, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::{
        NetCommand,
        build::BuildCmd,
        inventory::{InvFolder, InvItem},
    };

    fn fixture(test: impl FnOnce(&mut World, &mut BuildTool, &mut Env, &[usize])) {
        let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(avatars.clone());
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
        for (n, &idx) in prims.iter().enumerate() {
            let o = world.objects.get_mut(idx).unwrap();
            o.owner_id = world.agent_id;
            o.update_flags |= crate::ui::context::flags::OBJECT_MODIFY;
            let mut te = o.te.as_deref().unwrap().clone();
            te.faces[0].texture = Uuid::from_u128(100 + n as u128);
            o.te = Some(Arc::new(te));
            tool.selection.push(o.key);
            tool.faces.insert(o.key, vec![0]);
        }
        let mut scene = crate::scene::Scene::new(std::env::temp_dir(), avatars);
        let images = HashMap::new();
        let mut media = crate::media::MediaManager::default();
        let mut env = Env {
            images: &images,
            media: &mut media,
            scene: &mut scene,
            zoom_fraction: 0.0,
            property_lines: &mut false,
            demo: true,
        };
        test(&mut world, &mut tool, &mut env, &prims);
    }

    fn open(tool: &mut BuildTool, world: &World, env: &Env, target: PickTarget) {
        tool.ui.picker.open_with_defaults("Texture", Uuid::from_u128(100), None, false);
        tool.ui.pick_target = Some(target);
        tool.ui.texture_edit = Some(Session::capture(tool, world, env, target));
    }

    fn texture(world: &World, idx: usize, face: usize) -> Uuid {
        world.objects.get(idx).unwrap().te.as_ref().unwrap().faces[face].texture
    }

    #[test]
    fn local_picker_choices_and_cancel_never_send_local_ids_to_the_simulator() {
        use crate::world::objects::LocalAppearance;
        fixture(|world, tool, env, prims| {
            let id = Uuid::from_u128(901);
            let original: Vec<_> = prims.iter().map(|idx| texture(world, *idx, 0)).collect();
            for (material, target, appearance) in [
                (false, PickTarget::Diffuse, LocalAppearance::Texture(id)),
                (true, PickTarget::PbrMaterial, LocalAppearance::Material(id)),
            ] {
                world.local_assets.insert(id, ("Local fixture".into(), material));
                open(tool, world, env, target);
                tool.out.clear();
                tool.preview_texture(world, env, id);
                assert!(tool.out.is_empty(), "a local preview must not enqueue network edits");
                for (&idx, &texture) in prims.iter().zip(&original) {
                    let object = world.objects.get(idx).unwrap();
                    assert_eq!(object.local_appearance(0), Some(appearance));
                    assert_eq!(object.te.as_ref().unwrap().faces[0].texture, texture);
                    assert!(object.extra.render_materials.is_empty());
                    assert_eq!(object.local_appearance(1), None);
                }
                tool.cancel_texture_picker(world, env);
                assert!(tool.out.is_empty(), "cancelling a local choice must stay local");
                for &idx in prims {
                    assert_eq!(world.objects.get(idx).unwrap().local_appearance(0), None);
                }
            }
        });
    }

    #[test]
    fn local_face_choices_keep_other_faces_and_cancel_restores_existing_local_bindings() {
        fixture(|world, tool, env, prims| {
            let old = LocalAppearance::Texture(Uuid::from_u128(902));
            let id = Uuid::from_u128(903);
            world.local_assets.insert(id, ("Local fixture".into(), false));
            for &idx in prims {
                world.objects.get_mut(idx).unwrap().render.local_appearance.insert(None, old);
            }
            open(tool, world, env, PickTarget::Diffuse);
            tool.out.clear();
            tool.preview_texture(world, env, id);
            for &idx in prims {
                let object = world.objects.get(idx).unwrap();
                assert_eq!(object.local_appearance(0), Some(LocalAppearance::Texture(id)));
                assert_eq!(object.local_appearance(1), Some(old));
            }
            tool.preview_texture(world, env, aurora_prim::te::BLANK_TEXTURE);
            for &idx in prims {
                let object = world.objects.get(idx).unwrap();
                assert_eq!(object.local_appearance(0), None);
                assert_eq!(object.local_appearance(1), Some(old));
                assert_eq!(texture(world, idx, 0), aurora_prim::te::BLANK_TEXTURE);
            }
            tool.cancel_texture_picker(world, env);
            let mut links = HashMap::new();
            for (object, face, appearance) in tool.local_appearance_changes.drain(..) {
                if let Some(appearance) = appearance {
                    links.insert((object, face), appearance);
                } else {
                    links.remove(&(object, face));
                }
            }
            for &idx in prims {
                let object = world.objects.get(idx).unwrap();
                assert_eq!(object.local_appearance(0), Some(old));
                assert_eq!(object.local_appearance(1), Some(old));
                assert_eq!(links[&(object.full_id, Some(0))], old);
                assert_eq!(links[&(object.full_id, Some(1))], old);
            }
        });
    }

    #[test]
    fn clicking_a_local_picker_row_previews_and_ok_keeps_its_client_only_binding() {
        fixture(|world, tool, env, prims| {
            let id = Uuid::from_u128(904);
            world.local_assets.insert(id, ("Local fixture".into(), false));
            open(tool, world, env, PickTarget::Diffuse);
            let ctx = egui::Context::default();
            let mut positions = HashMap::new();
            for i in 0..10 {
                positions = frame(&ctx, tool, world, env, i as f64 / 60.0, vec![]);
            }
            click(&ctx, tool, world, env, 0.5, positions[aurora_i18n::tr!("picker-local-tab")]);
            positions = frame(&ctx, tool, world, env, 0.75, vec![]);
            tool.out.clear();
            click(&ctx, tool, world, env, 1.0, positions["Local fixture"]);
            assert!(tool.ui.picker.open);
            for &idx in prims {
                assert_eq!(
                    world.objects.get(idx).unwrap().local_appearance(0),
                    Some(LocalAppearance::Texture(id))
                );
            }
            let changes = tool.local_appearance_changes.len();
            let positions = frame(&ctx, tool, world, env, 2.0, vec![]);
            click(&ctx, tool, world, env, 3.0, positions["OK"]);
            assert!(!tool.ui.picker.open);
            assert!(tool.out.is_empty());
            assert_eq!(tool.local_appearance_changes.len(), changes);
        });
    }

    #[test]
    fn previews_apply_to_all_original_faces_once_and_cancel_preserves_other_edits() {
        fixture(|world, tool, env, prims| {
            open(tool, world, env, PickTarget::Diffuse);
            let unselected = texture(world, prims[0], 1);
            let preview = Uuid::from_u128(900);
            tool.preview_texture(world, env, preview);
            for &idx in prims {
                assert_eq!(texture(world, idx, 0), preview);
            }
            assert_eq!(texture(world, prims[0], 1), unselected);
            assert_eq!(
                tool.out
                    .iter()
                    .filter(|c| matches!(c, NetCommand::Build(BuildCmd::SetTextures { .. })))
                    .count(),
                2
            );
            let sent = tool.out.len();
            tool.preview_texture(world, env, preview);
            assert_eq!(tool.out.len(), sent);
            tool.edit_faces(world, |face, _, _| {
                face.color[3] = 0.42;
                face.offset_s = 0.7;
            });
            // The dialog must never write to this later selection on Cancel.
            tool.selection.clear();
            tool.faces.clear();
            tool.cancel_texture_picker(world, env);
            for (n, &idx) in prims.iter().enumerate() {
                assert_eq!(texture(world, idx, 0), Uuid::from_u128(100 + n as u128));
                let te = world.objects.get(idx).unwrap().te.as_ref().unwrap();
                assert_eq!(te.faces[0].color[3], 0.42);
                assert_eq!(te.faces[0].offset_s, 0.7);
            }
        });
    }

    #[test]
    fn cancel_restores_pbr_assets_overrides_maps_and_legacy_without_losing_unrelated_map_edits() {
        fixture(|world, tool, env, prims| {
            for (n, &idx) in prims.iter().enumerate() {
                let o = world.objects.get_mut(idx).unwrap();
                o.extra.render_materials.push((0, Uuid::from_u128(300 + n as u128)));
                world.gltf_overrides.insert(
                    o.key,
                    Arc::from(vec![(
                        0,
                        PbrOverride {
                            textures: [Uuid::from_u128(400 + n as u128); 4],
                            roughness_factor: Some(0.3),
                            ..Default::default()
                        },
                    )]),
                );
            }
            let original: Vec<_> = prims
                .iter()
                .map(|&idx| (BuildTool::gltf_asset(world, idx, 0), BuildTool::gltf_override(world, idx, 0)))
                .collect();
            open(tool, world, env, PickTarget::PbrMaterial);
            tool.preview_texture(world, env, Uuid::from_u128(500));
            tool.cancel_texture_picker(world, env);
            for (&idx, (asset, over)) in prims.iter().zip(&original) {
                assert_eq!(BuildTool::gltf_asset(world, idx, 0), *asset);
                assert_eq!(BuildTool::gltf_override(world, idx, 0), *over);
            }
            open(tool, world, env, PickTarget::PbrMap(0));
            tool.preview_texture(world, env, Uuid::nil());
            for &idx in prims {
                let mut over = BuildTool::gltf_override(world, idx, 0);
                over.textures[3] = Uuid::from_u128(700);
                tool.gltf_update(
                    world,
                    vec![GltfUpdate {
                        object: idx,
                        face: 0,
                        asset: None,
                        over: Some(over),
                    }],
                );
            }
            tool.cancel_texture_picker(world, env);
            for (&idx, (_, original)) in prims.iter().zip(&original) {
                let over = BuildTool::gltf_override(world, idx, 0);
                assert_eq!(over.textures[0], original.textures[0]);
                assert_eq!(over.textures[3], Uuid::from_u128(700));
                assert_eq!(over.roughness_factor, original.roughness_factor);
            }
            for &idx in prims {
                let o = world.objects.get(idx).unwrap();
                tool.ui.legacy_pending.insert(
                    (o.full_id, 0),
                    (
                        o.te.as_ref().unwrap().faces[0].material_id,
                        Some(LegacyMaterial {
                            normal_map: Uuid::from_u128(800),
                            ..Default::default()
                        }),
                    ),
                );
            }
            open(tool, world, env, PickTarget::Normal);
            tool.preview_texture(world, env, Uuid::from_u128(801));
            for pending in tool.ui.legacy_pending.values_mut() {
                pending.1.as_mut().unwrap().specular_map = Uuid::from_u128(802);
            }
            tool.cancel_texture_picker(world, env);
            for &idx in prims {
                let o = world.objects.get(idx).unwrap();
                let material = tool.face_legacy(env, world, idx, 0, &o.te.as_ref().unwrap().faces[0]).unwrap();
                assert_eq!(material.normal_map, Uuid::from_u128(800));
                assert_eq!(material.specular_map, Uuid::from_u128(802));
            }
        });
    }

    #[test]
    fn cancel_restores_sculpt_and_light_texture_and_removes_a_preview_created_legacy_material() {
        fixture(|world, tool, env, prims| {
            tool.selection.truncate(1);
            let idx = prims[0];
            world.objects.get_mut(idx).unwrap().extra.sculpt = Some(crate::build::edits::SCULPT_DEFAULT);
            open(tool, world, env, PickTarget::Sculpt);
            tool.preview_texture(world, env, Uuid::from_u128(900));
            assert_eq!(world.objects.get(idx).unwrap().extra.sculpt.unwrap().texture, Uuid::from_u128(900));
            tool.cancel_texture_picker(world, env);
            assert_eq!(
                world.objects.get(idx).unwrap().extra.sculpt.unwrap(),
                crate::build::edits::SCULPT_DEFAULT
            );
            world.objects.get_mut(idx).unwrap().extra.light_image = None;
            open(tool, world, env, PickTarget::LightTexture);
            tool.preview_texture(world, env, Uuid::from_u128(901));
            assert!(world.objects.get(idx).unwrap().extra.light_image.is_some());
            tool.cancel_texture_picker(world, env);
            assert!(world.objects.get(idx).unwrap().extra.light_image.is_none());
            let original = aurora_prim::extra::LightImageParams {
                texture: Uuid::from_u128(902),
                ..crate::build::edits::LIGHT_IMAGE_DEFAULT
            };
            world.objects.get_mut(idx).unwrap().extra.light_image = Some(original);
            open(tool, world, env, PickTarget::LightTexture);
            tool.preview_texture(world, env, Uuid::from_u128(903));
            world.objects.get_mut(idx).unwrap().extra.light_image.as_mut().unwrap().params.y = 0.25;
            tool.cancel_texture_picker(world, env);
            let restored = world.objects.get(idx).unwrap().extra.light_image.unwrap();
            assert_eq!(restored.texture, original.texture);
            assert_eq!(restored.params.y, 0.25);
            let o = world.objects.get_mut(idx).unwrap();
            let mut te = o.te.as_deref().unwrap().clone();
            te.faces[0].material_id = Uuid::nil();
            o.te = Some(Arc::new(te));
            open(tool, world, env, PickTarget::Specular);
            tool.preview_texture(world, env, Uuid::from_u128(904));
            tool.cancel_texture_picker(world, env);
            let o = world.objects.get(idx).unwrap();
            assert!(tool.face_legacy(env, world, idx, 0, &o.te.as_ref().unwrap().faces[0]).is_none());
            open(tool, world, env, PickTarget::Specular);
            tool.preview_texture(world, env, Uuid::from_u128(905));
            for pending in tool.ui.legacy_pending.values_mut() {
                pending.1.as_mut().unwrap().normal_st[0] = 2.5;
            }
            tool.cancel_texture_picker(world, env);
            let o = world.objects.get(idx).unwrap();
            let material = tool.face_legacy(env, world, idx, 0, &o.te.as_ref().unwrap().faces[0]).unwrap();
            assert!(material.normal_map.is_nil() && material.specular_map.is_nil());
            assert_eq!(material.normal_st[0], 2.5);
        });
    }

    fn frame(
        ctx: &egui::Context,
        tool: &mut BuildTool,
        world: &mut World,
        env: &mut Env,
        time: f64,
        events: Vec<egui::Event>,
    ) -> HashMap<String, egui::Pos2> {
        let p = crate::theme::Theme::default().palette();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |_| show(ctx, &p, tool, world, env),
        );
        output.textures_delta.clear();
        output
            .shapes
            .into_iter()
            .filter_map(|s| {
                if let egui::Shape::Text(t) = s.shape {
                    Some((t.galley.text().to_owned(), t.pos + t.galley.size() * 0.5))
                } else {
                    None
                }
            })
            .collect()
    }

    fn click(ctx: &egui::Context, tool: &mut BuildTool, world: &mut World, env: &mut Env, time: f64, pos: egui::Pos2) {
        for (step, pressed) in [true, false].into_iter().enumerate() {
            frame(
                ctx,
                tool,
                world,
                env,
                time + step as f64 / 60.0,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn light_picker_none_removes_only_projection_and_cancel_restores_its_settings() {
        fixture(|world, tool, env, prims| {
            tool.selection.truncate(1);
            let idx = prims[0];
            let light = aurora_prim::extra::LightParams {
                color: [0.2, 0.4, 0.7, 0.6],
                radius: 12.0,
                falloff: 1.25,
                ..crate::build::edits::LIGHT_DEFAULT
            };
            let projector = aurora_prim::extra::LightImageParams {
                texture: Uuid::from_u128(910),
                params: glam::Vec3::new(0.8, 2.5, 0.3),
            };
            let object = world.objects.get_mut(idx).unwrap();
            object.extra.light = Some(light);
            object.extra.light_image = Some(projector);
            let key = object.key;
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            // Open through the actual Features swatch, so the test also
            // covers whether this caller exposes the None preset.
            let mut draw_features = |time, events| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| super::super::features::show(ui, &theme.palette(), tool, world, &mut Default::default(), env),
                );
                output.textures_delta.clear();
                output.shapes.into_iter().find_map(|shape| match shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("common-texture") => {
                        // Caption below the 40-point swatch, with a 14-point
                        // label and the theme's vertical item spacing.
                        Some(egui::pos2(text.pos.x + 20.0, text.pos.y - 24.0))
                    }
                    _ => None,
                })
            };
            let mut swatch = None;
            for i in 0..10 {
                swatch = draw_features(i as f64 / 60.0, vec![]);
            }
            let pos = swatch.expect("light texture swatch");
            for (i, pressed) in [true, false].into_iter().enumerate() {
                draw_features(
                    1.0 + i as f64 / 60.0,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(tool.ui.pick_target, Some(PickTarget::LightTexture));
            let mut positions = HashMap::new();
            for i in 0..10 {
                positions = frame(&ctx, tool, world, env, 2.0 + i as f64 / 60.0, vec![]);
            }
            tool.out.clear();
            click(&ctx, tool, world, env, 3.0, positions[aurora_i18n::tr!("common-none-2")]);
            assert!(tool.ui.picker.open);
            let object = world.objects.get(idx).unwrap();
            assert_eq!(object.extra.light, Some(light));
            assert!(object.extra.light_image.is_none());
            assert!(matches!(tool.out.as_slice(), [NetCommand::Build(BuildCmd::SetExtraParam {
                handle, local_id, param_type, in_use: false, ..
            })] if *handle == key.region && *local_id == key.local_id && *param_type == aurora_prim::extra::PARAMS_LIGHT_IMAGE));

            let positions = frame(&ctx, tool, world, env, 3.2, vec![]);
            click(&ctx, tool, world, env, 3.3, positions[aurora_i18n::tr!("common-blank")]);
            assert_eq!(world.objects.get(idx).unwrap().extra.light_image.unwrap().params, projector.params);
            let positions = frame(&ctx, tool, world, env, 3.5, vec![]);
            click(&ctx, tool, world, env, 3.6, positions[aurora_i18n::tr!("common-none-2")]);
            let positions = frame(&ctx, tool, world, env, 4.0, vec![]);
            click(&ctx, tool, world, env, 5.0, positions[aurora_i18n::tr!("common-cancel")]);
            assert_eq!(world.objects.get(idx).unwrap().extra.light_image, Some(projector));
            assert_eq!(world.objects.get(idx).unwrap().extra.light, Some(light));

            tool.ui.picker.open_with_defaults("Light", projector.texture, None, true);
            tool.ui.pick_target = Some(PickTarget::LightTexture);
            let positions = frame(&ctx, tool, world, env, 6.0, vec![]);
            tool.out.clear();
            click(&ctx, tool, world, env, 7.0, positions[aurora_i18n::tr!("common-none-2")]);
            let positions = frame(&ctx, tool, world, env, 8.0, vec![]);
            let sent = tool.out.len();
            click(&ctx, tool, world, env, 9.0, positions["OK"]);
            assert!(!tool.ui.picker.open);
            assert_eq!(tool.out.len(), sent, "OK confirms the removal without resending it");
            assert!(world.objects.get(idx).unwrap().extra.light_image.is_none());
            assert_eq!(world.objects.get(idx).unwrap().extra.light, Some(light));
        });
    }

    #[test]
    fn a_real_single_click_applies_before_ok_while_accept_and_cancel_keep_their_roles() {
        fixture(|world, tool, env, prims| {
            let root = Uuid::from_u128(10);
            let item = Uuid::from_u128(11);
            let asset = Uuid::from_u128(12);
            world.inventory.root = root;
            world.inventory.folders.insert(
                root,
                Folder {
                    info: InvFolder {
                        id: root,
                        name: "Textures".into(),
                        ..Default::default()
                    },
                    children: vec![],
                    items: vec![item],
                    state: FetchState::Fetched,
                    library: false,
                },
            );
            world.inventory.items.insert(
                item,
                InvItem {
                    id: item,
                    parent: root,
                    asset_id: asset,
                    inv_type: 0,
                    name: "A texture".into(),
                    owner_mask: crate::world::inventory::actions::COPY,
                    ..Default::default()
                },
            );
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            open(tool, world, env, PickTarget::Diffuse);
            let mut positions = HashMap::new();
            for i in 0..10 {
                positions = frame(&ctx, tool, world, env, i as f64 / 60.0, vec![]);
            }
            let row = positions["A texture"];
            click(&ctx, tool, world, env, 1.0, row);
            assert!(tool.ui.picker.open, "single click must keep the picker open");
            for &idx in prims {
                assert_eq!(texture(world, idx, 0), asset);
            }
            let positions = frame(&ctx, tool, world, env, 2.0, vec![]);
            let sent = tool.out.len();
            click(&ctx, tool, world, env, 3.0, positions["OK"]);
            assert!(!tool.ui.picker.open);
            assert!(tool.ui.texture_edit.is_none());
            assert_eq!(tool.out.len(), sent, "OK must not resend a preview");
            // Reopening and closing without a choice leaves the committed texture.
            open(tool, world, env, PickTarget::Diffuse);
            let positions = frame(&ctx, tool, world, env, 4.0, vec![]);
            click(&ctx, tool, world, env, 5.0, positions[aurora_i18n::tr!("common-cancel")]);
            for &idx in prims {
                assert_eq!(texture(world, idx, 0), asset);
            }
            // Cancel after a different preview restores those committed textures.
            open(tool, world, env, PickTarget::Diffuse);
            let positions = frame(&ctx, tool, world, env, 6.0, vec![]);
            click(&ctx, tool, world, env, 6.5, positions[aurora_i18n::tr!("common-blank")]);
            for &idx in prims {
                assert_eq!(texture(world, idx, 0), aurora_prim::te::BLANK_TEXTURE);
            }
            let positions = frame(&ctx, tool, world, env, 7.0, vec![]);
            click(&ctx, tool, world, env, 8.0, positions[aurora_i18n::tr!("common-cancel")]);
            for &idx in prims {
                assert_eq!(texture(world, idx, 0), asset);
            }
            // A selection change also cancels, restoring the original faces.
            open(tool, world, env, PickTarget::Diffuse);
            tool.preview_texture(world, env, Uuid::from_u128(14));
            tool.faces.clear();
            frame(&ctx, tool, world, env, 9.0, vec![]);
            assert!(!tool.ui.picker.open);
            for &idx in prims {
                assert_eq!(texture(world, idx, 0), asset);
            }
            // Closing with the window X follows the same cancellation path.
            open(tool, world, env, PickTarget::Diffuse);
            tool.preview_texture(world, env, Uuid::from_u128(15));
            tool.ui.picker.open = false;
            frame(&ctx, tool, world, env, 10.0, vec![]);
            for &idx in prims {
                assert_eq!(texture(world, idx, 0), asset);
            }
            // Like Firestorm, no-copy items require OK instead of a preview.
            world.inventory.items.get_mut(&item).unwrap().owner_mask = 0;
            world.inventory.items.get_mut(&item).unwrap().asset_id = Uuid::from_u128(16);
            world.inventory.generation += 1;
            open(tool, world, env, PickTarget::Diffuse);
            let positions = frame(&ctx, tool, world, env, 11.0, vec![]);
            click(&ctx, tool, world, env, 12.0, positions["A texture"]);
            for &idx in prims {
                assert_eq!(texture(world, idx, 0), asset);
            }
            let positions = frame(&ctx, tool, world, env, 13.0, vec![]);
            assert!(positions.contains_key(aurora_i18n::tr!("ui-texture-picker-confirm-no-copy")));
            click(&ctx, tool, world, env, 14.0, positions["OK"]);
            for &idx in prims {
                assert_eq!(texture(world, idx, 0), Uuid::from_u128(16));
            }
        });
    }
    #[test]
    fn texture_library_surface_drop_preserves_selection_and_other_face_properties() {
        fixture(|world, tool, env, prims| {
            let idx = prims[0];
            let object = world.objects.get_mut(idx).unwrap();
            let mut te = object.te.as_deref().unwrap().clone();
            te.faces[1].color = [0.2, 0.4, 0.6, 0.8];
            te.faces[1].scale_s = 2.7;
            te.faces[1].offset_t = 0.21;
            te.faces[1].glow = 0.1;
            let mut expected = te.clone();
            let asset = Uuid::from_u128(906);
            expected.faces[1].texture = asset;
            object.te = Some(Arc::new(te));
            let selection = tool.selection.clone();
            let faces = tool.faces.clone();
            let other = world.objects.get(prims[1]).unwrap().te.clone();
            tool.apply_inventory_surface(world, env, idx, &[1], asset, false, false);
            assert_eq!(world.objects.get(idx).unwrap().te.as_deref(), Some(&expected));
            assert_eq!(world.objects.get(prims[1]).unwrap().te, other);
            assert_eq!(tool.selection, selection);
            assert_eq!(tool.faces, faces);
            assert!(!tool.out.is_empty(), "the face edit must enqueue a simulator update");
            let count = tool.num_faces(world, idx);
            let all: Vec<_> = (0..count).map(|i| i as u8).collect();
            tool.apply_inventory_surface(world, env, idx, &all, asset, false, false);
            assert!(
                world
                    .objects
                    .get(idx)
                    .unwrap()
                    .te
                    .as_ref()
                    .unwrap()
                    .faces
                    .iter()
                    .take(count)
                    .all(|f| f.texture == asset)
            );
        });
    }

    #[test]
    fn texture_library_pbr_texture_drop_keeps_other_maps_and_alt_clears_the_material() {
        fixture(|world, tool, env, prims| {
            let idx = prims[0];
            let asset = Uuid::from_u128(907);
            let map = Uuid::from_u128(908);
            tool.apply_inventory_surface(world, env, idx, &[1], asset, true, false);
            let mut over = BuildTool::gltf_override(world, idx, 1);
            over.textures[1] = Uuid::from_u128(909);
            over.roughness_factor = Some(0.25);
            tool.gltf_update(
                world,
                vec![crate::build::materials::GltfUpdate {
                    object: idx,
                    face: 1,
                    asset: None,
                    over: Some(over.clone()),
                }],
            );
            tool.apply_inventory_surface(world, env, idx, &[1], map, false, false);
            assert_eq!(BuildTool::gltf_asset(world, idx, 1), Some(asset));
            over.textures[0] = map;
            assert_eq!(BuildTool::gltf_override(world, idx, 1), over);
            assert_ne!(texture(world, idx, 1), map);
            tool.apply_inventory_surface(world, env, idx, &[1], map, false, true);
            assert_eq!(BuildTool::gltf_asset(world, idx, 1), None);
            assert_eq!(BuildTool::gltf_override(world, idx, 1), Default::default());
            assert_eq!(texture(world, idx, 1), map);
            assert_eq!(BuildTool::gltf_asset(world, idx, 0), None);
        });
    }

    #[test]
    fn texture_library_drop_rejects_unmodifiable_objects_and_invalid_faces() {
        fixture(|world, tool, env, prims| {
            let idx = prims[0];
            let original = world.objects.get(idx).unwrap().te.clone();
            tool.out.clear();
            tool.apply_inventory_surface(
                world,
                env,
                idx,
                &[tool.num_faces(world, idx) as u8, u8::MAX],
                Uuid::from_u128(910),
                false,
                false,
            );
            assert_eq!(world.objects.get(idx).unwrap().te, original);
            assert!(tool.out.is_empty());
            world.objects.get_mut(idx).unwrap().owner_id = Uuid::from_u128(911);
            world.objects.get_mut(idx).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
            tool.apply_inventory_surface(world, env, idx, &[0], Uuid::from_u128(910), true, false);
            assert_eq!(world.objects.get(idx).unwrap().te, original);
            assert!(tool.out.is_empty());
        });
    }
    #[test]
    fn texture_library_click_targets_only_the_editable_selected_faces() {
        fixture(|world, tool, env, prims| {
            tool.open = true;
            tool.tool = crate::build::Tool::Edit;
            tool.edit_mode = crate::build::EditMode::Face;
            let first = world.objects.get(prims[0]).unwrap().key;
            tool.faces.insert(first, vec![1, 1, u8::MAX]);
            for material in [false, true] {
                let before: Vec<_> = prims.iter().map(|idx| world.objects.get(*idx).unwrap().te.clone()).collect();
                let selection = tool.selection.clone();
                let faces = tool.faces.clone();
                let targets = tool.inventory_surface_selection(world);
                assert_eq!(targets.len(), 2);
                assert!(targets.contains(&(prims[0], vec![1])));
                assert!(targets.contains(&(prims[1], vec![0])));
                let asset = Uuid::from_u128(if material { 920 } else { 921 });
                for (idx, faces) in targets {
                    tool.apply_inventory_surface(world, env, idx, &faces, asset, material, false);
                }
                for (n, &idx) in prims.iter().enumerate() {
                    let face = if n == 0 { 1 } else { 0 };
                    if material {
                        assert_eq!(BuildTool::gltf_asset(world, idx, face), Some(asset));
                    } else {
                        assert_eq!(texture(world, idx, face as usize), asset);
                    }
                    let other = if face == 0 { 1 } else { 0 };
                    assert_eq!(
                        world.objects.get(idx).unwrap().te.as_ref().unwrap().faces[other],
                        before[n].as_ref().unwrap().faces[other]
                    );
                }
                assert_eq!(tool.selection, selection);
                assert_eq!(tool.faces, faces);
            }
            world.objects.get_mut(prims[1]).unwrap().owner_id = Uuid::from_u128(922);
            world.objects.get_mut(prims[1]).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
            assert_eq!(tool.inventory_surface_selection(world), vec![(prims[0], vec![1])]);
            tool.open = false;
            assert!(tool.inventory_surface_selection(world).is_empty());
            tool.open = true;
            tool.tool = crate::build::Tool::Create;
            assert!(tool.inventory_surface_selection(world).is_empty());
        });
    }

    #[test]
    fn shared_objects_accept_inventory_and_library_surfaces_and_recheck_revoked_rights() {
        fixture(|world, tool, env, prims| {
            use crate::ui::context::flags::OBJECT_MODIFY;
            use aurora_net::build::{ObjectProps, perm};
            tool.open = true;
            tool.tool = crate::build::Tool::Edit;
            for &idx in prims {
                let object = world.objects.get_mut(idx).unwrap();
                object.owner_id = Uuid::from_u128(940);
                tool.props.insert(
                    object.full_id,
                    ObjectProps {
                        owner_id: object.owner_id,
                        owner_mask: perm::MODIFY,
                        ..Default::default()
                    },
                );
            }
            assert_eq!(tool.inventory_surface_selection(world).len(), 2);
            let idx = prims[0];
            tool.out.clear();
            tool.apply_inventory_surface(world, env, idx, &[0], Uuid::from_u128(941), false, false);
            assert_eq!(texture(world, idx, 0), Uuid::from_u128(941));
            tool.apply_inventory_surface(world, env, idx, &[0], Uuid::from_u128(942), true, false);
            assert_eq!(BuildTool::gltf_asset(world, idx, 0), Some(Uuid::from_u128(942)));
            assert!(
                tool.out
                    .iter()
                    .any(|cmd| matches!(cmd, aurora_net::NetCommand::Build(BuildCmd::SetTextures { .. })))
            );
            assert!(
                tool.out
                    .iter()
                    .any(|cmd| matches!(cmd, aurora_net::NetCommand::Build(BuildCmd::Cap { cap, .. }) if cap == "ModifyMaterialParams"))
            );
            world.objects.get_mut(idx).unwrap().update_flags &= !OBJECT_MODIFY;
            tool.out.clear();
            tool.apply_inventory_surface(world, env, idx, &[0], Uuid::from_u128(943), false, false);
            tool.apply_inventory_surface(world, env, idx, &[0], Uuid::from_u128(944), true, false);
            assert!(tool.out.is_empty());
            assert_eq!(texture(world, idx, 0), Uuid::from_u128(941));
            assert_eq!(BuildTool::gltf_asset(world, idx, 0), Some(Uuid::from_u128(942)));
            assert_eq!(tool.inventory_surface_selection(world), vec![(prims[1], vec![0])]);
        });
    }

    #[test]
    fn texture_library_commit_finishes_the_old_picker_before_applying() {
        fixture(|world, tool, env, prims| {
            let original = texture(world, prims[1], 0);
            open(tool, world, env, PickTarget::Diffuse);
            tool.preview_texture(world, env, Uuid::from_u128(923));
            tool.apply_inventory_surface(world, env, prims[0], &[0], Uuid::from_u128(924), false, false);
            assert!(!tool.ui.picker.open);
            assert!(tool.ui.texture_edit.is_none());
            tool.cancel_texture_picker(world, env);
            assert_eq!(texture(world, prims[0], 0), Uuid::from_u128(924));
            assert_eq!(texture(world, prims[1], 0), original);
        });
    }
}
