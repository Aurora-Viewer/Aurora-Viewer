//! LLTextureCtrl TEXTURE_CHANGE / SELECT / CANCEL and LLSelectMgr's saved
//! textures (Firestorm lltexturectrl.cpp / llselectmgr.cpp, LGPL 2.1).
//! A live picker owns its original faces and can restore mixed selections.

use super::{Env, PickTarget, texture::Sel};
use crate::build::{BuildTool, edits::Extra, materials::GltfUpdate};
use crate::scene::legacy_mat::LegacyMaterial;
use crate::theme::Palette;
use crate::world::{World, objects::ObjKey};
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
    Light(Option<Uuid>),
}

struct Saved {
    object: Uuid,
    face: u8,
    original: Original,
}

pub(super) struct Session {
    generation: u64,
    target: PickTarget,
    selection: Vec<ObjKey>,
    faces: HashMap<ObjKey, Vec<u8>>,
    linked: bool,
    saved: Vec<Saved>,
    last_preview: Option<Uuid>,
}

impl Session {
    fn capture(tool: &BuildTool, world: &World, env: &Env, target: PickTarget) -> Self {
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
                PickTarget::LightTexture => Original::Light(object.extra.light_image.map(|light| light.texture)),
            };
            saved.push(Saved {
                object: object.full_id,
                face: s.face,
                original,
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
            self.apply_texture(world, env, session.target, id, &sel);
            session.last_preview = Some(id);
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
                    let mut material = current.clone().unwrap_or_default();
                    let old = original.clone().unwrap_or_default();
                    if session.target == PickTarget::Normal {
                        material.normal_map = old.normal_map;
                    } else {
                        material.specular_map = old.specular_map;
                    }
                    // Remove a preview-created material, keeping unrelated edits.
                    let material = if original.is_none() && material == LegacyMaterial::default() {
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
                Original::Light(texture) => {
                    let current = world.objects.get(idx).and_then(|o| o.extra.light_image);
                    let restored = texture.map(|texture| aurora_prim::extra::LightImageParams {
                        texture,
                        ..current.unwrap_or(crate::build::edits::LIGHT_IMAGE_DEFAULT)
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
}
