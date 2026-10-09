//! Independent animated-object skeletons, following LLVOVolume::isAnimatedObject,
//! LLControlAvatar::updateAnimations and LLVOAvatar::updateAttachmentOverrides
//! (indra/newview/llvovolume.cpp, llcontrolavatar.cpp, llvoavatar.cpp, LGPL 2.1).

use crate::world::objects::ObjectStore;
use crate::world::{PlayingAnimation, World};
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// The linkset root owns the control skeleton even when its parent is an avatar.
/// A flag on a child cannot turn a regular attachment into an animated object.
pub fn owner(objects: &ObjectStore, mut idx: usize) -> Option<(Uuid, usize)> {
    for _ in 0..16 {
        let o = objects.get(idx)?;
        if o.is_avatar() {
            return Some((o.full_id, idx));
        }
        let parent = if o.parent_id == 0 { None } else { Some(objects.parent_of(o)?) };
        if parent.is_none_or(|p| objects.get(p).is_some_and(|p| p.is_avatar())) {
            if o.extra.extended_mesh_flags.unwrap_or(0) & aurora_prim::extra::EXTENDED_MESH_ANIMATED != 0 {
                return Some((o.full_id, idx));
            }
            return parent.and_then(|p| objects.get(p).map(|o| (o.full_id, p)));
        }
        idx = parent?;
    }
    None
}

/// Includes an animesh's root mesh, excludes other skeletons and their children.
pub fn members(objects: &ObjectStore, root: usize) -> Vec<usize> {
    let mut result = Vec::new();
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        if result.len() >= 20_000 {
            break;
        }
        let Some(o) = objects.get(i) else { continue };
        if i != root && owner(objects, i).is_none_or(|(_, p)| p != root) {
            continue;
        }
        result.push(i);
        stack.extend_from_slice(objects.children_of(&o.key));
    }
    result
}

#[derive(Default)]
pub struct Signals {
    playing: HashMap<Uuid, PlayingAnimation>,
    sources: HashMap<Uuid, HashMap<Uuid, Instant>>,
}

impl Signals {
    /// Merge per-object signals by maximum sequence, as LLControlAvatar does.
    /// Keep the controller's clocks when the same animation changes source.
    pub fn update(&mut self, world: &World, root: usize, now: Instant) -> Vec<PlayingAnimation> {
        let mut sequences = HashMap::<Uuid, i32>::new();
        let mut sources = HashMap::<Uuid, HashMap<Uuid, Instant>>::new();
        for i in members(&world.objects, root) {
            let Some(o) = world.objects.get(i) else { continue };
            for a in world.animations_of(&o.full_id) {
                sources.entry(a.id).or_default().insert(o.full_id, a.continuous_start);
                sequences
                    .entry(a.id)
                    .and_modify(|s| *s = (*s).max(a.sequence))
                    .or_insert(a.sequence);
            }
        }
        let next: HashMap<_, _> = sequences
            .into_iter()
            .map(|(id, seq)| {
                let continuing = sources.get(&id).is_some_and(|current| {
                    current
                        .iter()
                        .any(|(obj, start)| self.sources.get(&id).and_then(|previous| previous.get(obj)) == Some(start))
                });
                (
                    id,
                    PlayingAnimation::from_signal(id, seq, now, self.playing.get(&id).filter(|_| continuing)),
                )
            })
            .collect();
        self.playing = next;
        self.sources = sources;
        let mut signals: Vec<_> = self.playing.values().copied().collect();
        signals.sort_unstable_by_key(|a| a.id);
        signals
    }
}

/// Bind-space box of vertices influenced by one mesh joint. Transforming all
/// boxes encloses linear blend skinning, including motion beyond the prim scale.
#[derive(Clone, Copy)]
pub struct JointBounds {
    pub joint: u8,
    pub min: Vec3,
    pub max: Vec3,
}

pub fn posed_bounds(bounds: &[JointBounds], mut matrix: impl FnMut(u8) -> Option<Mat4>) -> Option<(Vec3, f32)> {
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut any = false;
    for b in bounds {
        let Some(m) = matrix(b.joint) else { continue };
        for x in [b.min.x, b.max.x] {
            for y in [b.min.y, b.max.y] {
                for z in [b.min.z, b.max.z] {
                    let p = m.transform_point3(Vec3::new(x, y, z));
                    if p.is_finite() {
                        min = min.min(p);
                        max = max.max(p);
                        any = true;
                    }
                }
            }
        }
    }
    any.then(|| ((min + max) * 0.5, ((max - min) * 0.5).length().max(0.05)))
}

impl super::Scene {
    pub fn log_demo_animesh(&self, world: &World) {
        let Some(&head) = self.avatar_lib.rig.names.get("mHead") else {
            return;
        };
        for local in [9200, 9202, 9203] {
            let Some((i, _)) = world.objects.iter().find(|(_, o)| o.key.local_id == local) else {
                continue;
            };
            let Some(g) = self.gpu.get(i) else { continue };
            let pose = g
                .skeleton_owner
                .and_then(|id| self.palette_slots.get(&id))
                .and_then(|slot| self.palettes.get(*slot as usize * super::anim::PALETTE_JOINTS + head))
                .map(|p| Mat4::from_cols_array_2d(p).to_scale_rotation_translation().1);
            let casters = g
                .faces
                .iter()
                .filter(|f| self.lists.shadow_casters.iter().any(|c| c.cmd.record == f.record))
                .count();
            log::info!(
                "demo animesh {local}: owner {:?}, head {:?}, shadow faces {casters}, center {:.3?}, radius {:.3}",
                g.skeleton_owner,
                pose,
                g.center,
                g.radius
            );
        }
    }
    /// Synthetic quadruped with a custom rest skeleton and smooth alpha fur.
    pub fn install_demo_animesh(&mut self, renderer: &mut aurora_render::Renderer) {
        use super::jobs::{FaceGeom, JobResult};
        use aurora_render::SkinVertex;
        let rig = self.avatar_lib.rig.clone();
        let mut local_pos = rig.local_pos.clone();
        for (name, p) in [
            ("mTorso", Vec3::new(0.15, 0.0, 0.1)),
            ("mChest", Vec3::new(0.2, 0.0, 0.0)),
            ("mNeck", Vec3::new(0.3, 0.0, 0.0)),
            ("mHead", Vec3::new(0.2, 0.0, 0.0)),
            ("mTail1", Vec3::new(-0.4, 0.0, 0.1)),
        ] {
            if let Some(&j) = rig.names.get(name) {
                local_pos[j] = p;
            }
        }
        let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &local_pos, &rig.scale);
        let mut names = vec![String::new(); rig.len()];
        // Prefer canonical m-prefixed names over skeleton aliases.
        for (name, &j) in &rig.names {
            if name.starts_with('m') {
                names[j] = name.clone();
            }
        }
        let skin = aurora_assets::SkinInfo {
            joint_names: names,
            inverse_bind: rest.iter().map(|m| m.inverse()).collect(),
            bind_shape: Mat4::IDENTITY,
            alt_inverse_bind: local_pos.iter().map(|p| Mat4::from_translation(*p)).collect(),
            pelvis_offset: 0.0,
            lock_scale_if_joint_position: true,
        };
        self.meshes.insert_demo_skin(crate::demo::ANIMESH_MESH, skin);
        let mut face = FaceGeom {
            vertices: Vec::new(),
            indices: Vec::new(),
            skin: Some(Vec::new()),
        };
        let cube = aurora_prim::generate_volume(&aurora_prim::VolumeParams::default(), 1.0);
        for (name, size) in [
            ("mChest", Vec3::new(0.7, 0.28, 0.28)),
            ("mHead", Vec3::splat(0.23)),
            ("mTail1", Vec3::new(0.6, 0.07, 0.07)),
        ] {
            let Some(&j) = rig.names.get(name) else { continue };
            let transform = Mat4::from_scale_rotation_translation(size, glam::Quat::IDENTITY, rest[j].w_axis.truncate());
            for f in &cube.faces {
                let base = face.vertices.len() as u16;
                for ((p, n), uv) in f.positions.iter().zip(&f.normals).zip(&f.uvs) {
                    let p = transform.transform_point3(Vec3::from_array(*p));
                    face.vertices.push(aurora_render::Vertex::new(p.to_array(), *n, *uv));
                    if let Some(s) = &mut face.skin {
                        s.push(SkinVertex {
                            joints: [j as u8, 0, 0, 0],
                            weights: [255, 0, 0, 0],
                        });
                    }
                }
                face.indices.extend(f.indices.iter().map(|i| base + *i));
            }
        }
        for lod in 0..4 {
            self.upload_geom(
                renderer,
                super::GeomKey::Mesh {
                    id: crate::demo::ANIMESH_MESH,
                    lod,
                },
                vec![Some(FaceGeom {
                    vertices: face.vertices.clone(),
                    indices: face.indices.clone(),
                    skin: face.skin.clone(),
                })],
                Vec3::splat(-2.0),
                Vec3::splat(2.0),
            );
        }
        self.anims.insert(
            crate::demo::ANIMESH_ANIM,
            std::sync::Arc::new(super::anim::BoundAnim::bind(crate::demo::animesh_animation(), &rig)),
        );
        self.textures
            .acquire(renderer, crate::demo::ANIMESH_TEXTURE, super::textures::TexSource::Asset);
        let mut rgba = Vec::new();
        for y in 0..32 {
            for x in 0..32 {
                let alpha = if x < 4 || y < 4 || x > 27 || y > 27 {
                    0
                } else if x < 6 || y < 6 || x > 25 || y > 25 {
                    100
                } else {
                    255
                };
                rgba.extend([255, 255, 255, alpha]);
            }
        }
        self.textures.on_job(JobResult::Texture {
            id: crate::demo::ANIMESH_TEXTURE,
            discard: 0,
            mips: vec![(32, 32, rgba)],
            alpha: super::jobs::AlphaKind::Blend,
            alpha_channel: true,
            sculpt: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use aurora_net::NetEvent;
    use std::sync::Arc;
    use std::time::Duration;

    fn fixture() -> World {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        for e in crate::demo::events().into_iter().chain(crate::demo::animesh_events()) {
            w.apply(e);
        }
        w
    }

    fn index(w: &World, local: u32) -> usize {
        w.objects
            .iter()
            .find(|(_, o)| o.key.local_id == local)
            .map(|(i, _)| i)
            .expect("fixture object")
    }

    #[test]
    fn ground_and_worn_linksets_have_independent_skeletons() {
        let w = fixture();
        for (mesh, root) in [(9200, 9200), (9202, 9201), (9203, 9203), (9100, 9001)] {
            let i = index(&w, root);
            assert_eq!(
                owner(&w.objects, index(&w, mesh)),
                Some((w.objects.get(i).expect("root").full_id, i))
            );
        }
        let human = members(&w.objects, index(&w, 9001));
        assert!(human.contains(&index(&w, 9100)));
        assert!(!human.contains(&index(&w, 9203)));
        assert!(members(&w.objects, index(&w, 9200)).contains(&index(&w, 9200)));
        assert_eq!(members(&w.objects, index(&w, 9201)).len(), 2);
    }

    #[test]
    fn child_flags_do_not_override_root_and_late_parent_is_deferred() {
        let mut w = fixture();
        let child = index(&w, 9202);
        w.objects.get_mut(child).expect("child").extra.extended_mesh_flags = Some(1);
        let root = index(&w, 9201);
        w.objects.get_mut(root).expect("root").extra.extended_mesh_flags = None;
        assert_eq!(owner(&w.objects, child), None);
        w.objects.get_mut(child).expect("child").parent_id = 99999;
        assert_eq!(owner(&w.objects, child), None);
        w.objects.get_mut(child).expect("child").parent_id = 9202;
        assert_eq!(owner(&w.objects, child), None); // Malformed cycle.
    }

    #[test]
    fn linkset_animation_signals_merge_and_preserve_phase_across_sources() {
        let mut w = fixture();
        let root = index(&w, 9201);
        let child = index(&w, 9202);
        let root_id = w.objects.get(root).expect("root").full_id;
        let child_id = w.objects.get(child).expect("child").full_id;
        let id = crate::demo::ANIMESH_ANIM;
        let t = Instant::now();
        let mut signals = Signals::default();
        let first = signals.update(&w, root, t);
        assert_eq!(first.len(), 1); // Only the child has a signal.
        w.apply(NetEvent::AvatarAnimations {
            avatar: root_id,
            anims: vec![(id, 9)],
        });
        let second = signals.update(&w, root, t + Duration::from_secs(1));
        assert_eq!(second[0].sequence, 9);
        assert_eq!(second[0].continuous_start, first[0].continuous_start);
        w.apply(NetEvent::AvatarAnimations {
            avatar: root_id,
            anims: vec![],
        });
        let third = signals.update(&w, root, t + Duration::from_secs(2));
        assert_eq!(third[0].sequence, 1);
        assert_eq!(third[0].continuous_start, first[0].continuous_start);
        w.apply(NetEvent::AvatarAnimations {
            avatar: child_id,
            anims: vec![],
        });
        assert!(signals.update(&w, root, t + Duration::from_secs(3)).is_empty());
        w.apply(NetEvent::AvatarAnimations {
            avatar: child_id,
            anims: vec![(id, 10)],
        });
        let restarted = signals.update(&w, root, t + Duration::from_secs(4));
        assert!(restarted[0].continuous_start > first[0].continuous_start);
        w.apply(NetEvent::AvatarAnimations {
            avatar: child_id,
            anims: vec![],
        });
        w.apply(NetEvent::AvatarAnimations {
            avatar: child_id,
            anims: vec![(id, 11)],
        });
        let between_frames = signals.update(&w, root, t + Duration::from_secs(5));
        assert!(between_frames[0].continuous_start > restarted[0].continuous_start);
    }

    #[test]
    fn posed_bounds_enclose_skinning_outside_the_prim_scale() {
        let b = [
            JointBounds {
                joint: 0,
                min: Vec3::splat(-1.0),
                max: Vec3::ONE,
            },
            JointBounds {
                joint: 1,
                min: Vec3::splat(-1.0),
                max: Vec3::ONE,
            },
        ];
        let transforms = [Mat4::from_translation(Vec3::X * 10.0), Mat4::from_translation(Vec3::X * -7.0)];
        let (center, radius) = posed_bounds(&b, |j| Some(transforms[j as usize])).expect("finite bounds");
        for x in [-8.0, 11.0] {
            assert!((Vec3::new(x, 1.0, 1.0) - center).length() <= radius + 1e-5);
        }
        assert!(radius > 9.0);
        assert_eq!(posed_bounds(&b, |_| None), None);
    }

    #[test]
    fn root_skin_overrides_load_without_touching_the_wearers_skeleton() {
        let mut w = fixture();
        let lib = w.avatar_lib.clone();
        let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/animesh-tests")
            .join(Uuid::new_v4().to_string());
        let mut scene = super::super::Scene::new(cache, lib.clone());
        let now = Instant::now();
        let human = index(&w, 9001);
        let human_id = w.objects.get(human).expect("avatar").full_id;
        let (before, _) = scene.skeleton_of(&w, human_id, human, now);
        let chest = lib.rig.names["mChest"];
        let offset = Vec3::new(0.7, 0.0, 0.03);
        scene.meshes.insert_demo_skin(
            crate::demo::ANIMESH_MESH,
            aurora_assets::SkinInfo {
                joint_names: vec!["mChest".into()],
                inverse_bind: vec![Mat4::IDENTITY],
                alt_inverse_bind: vec![Mat4::from_translation(offset)],
                ..Default::default()
            },
        );
        let (after, _) = scene.skeleton_of(&w, human_id, human, now);
        assert!(Arc::ptr_eq(&before, &after));
        for local in [9200, 9201, 9203] {
            let root = index(&w, local);
            let id = w.objects.get(root).expect("root").full_id;
            let (base, dz) = scene.skeleton_of(&w, id, root, now);
            assert_eq!(base.local_pos[chest], offset);
            assert_eq!(dz, 0.0);
            assert!(!Arc::ptr_eq(&base, &after));
        }
        let worn = index(&w, 9203);
        w.objects.get_mut(worn).expect("worn").extra.extended_mesh_flags = None;
        let (normal_attachment, _) = scene.skeleton_of(&w, human_id, human, now);
        assert_eq!(normal_attachment.local_pos[chest], offset);
        w.objects.get_mut(worn).expect("worn").extra.extended_mesh_flags = Some(1);
        let (restored, _) = scene.skeleton_of(&w, human_id, human, now);
        assert_eq!(restored.local_pos[chest], before.local_pos[chest]);
    }

    #[test]
    fn control_skeleton_has_no_procedural_head_or_eye_motion() {
        let lib = AvatarLibrary::load();
        let rig = &lib.rig;
        let mut c = super::super::anim::Controller::for_control_avatar();
        let mut palette = vec![Mat4::IDENTITY.to_cols_array_2d(); rig.len()];
        let now = Instant::now();
        c.evaluate(rig, now, None, &mut palette);
        c.evaluate(rig, now + Duration::from_secs(5), None, &mut palette);
        for (m, inv) in palette.iter().zip(&rig.default_world_inv) {
            assert!((Mat4::from_cols_array_2d(m) * *inv).abs_diff_eq(Mat4::IDENTITY, 1e-5));
        }
    }
}
