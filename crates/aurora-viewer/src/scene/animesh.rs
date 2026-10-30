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
///
/// Same members as keeping the descendants whose `owner` is `root`, without
/// walking up from each one: below a member, a child belongs to another
/// skeleton only when it is an avatar, or an animated linkset root worn by
/// the avatar `root`; `owner` gives up beyond 16 levels.
pub fn members(objects: &ObjectStore, root: usize) -> Vec<usize> {
    let mut result = Vec::new();
    let Some(r) = objects.get(root) else {
        return result;
    };
    // not a skeleton owner (a child prim, an unflagged attachment): its
    // descendants belong to another skeleton
    if owner(objects, root).is_none_or(|(_, p)| p != root) {
        result.push(root);
        return result;
    }
    let mut stack = vec![(root, r, 0u32)];
    while let Some((i, o, depth)) = stack.pop() {
        if result.len() >= 20_000 {
            break;
        }
        result.push(i);
        if depth >= 15 {
            continue;
        }
        for &c in objects.children_of(&o.key) {
            let Some(child) = objects.get(c) else { continue };
            let animated = child.extra.extended_mesh_flags.unwrap_or(0) & aurora_prim::extra::EXTENDED_MESH_ANIMATED != 0;
            if child.is_avatar() || (o.is_avatar() && animated) {
                continue;
            }
            stack.push((c, child, depth + 1));
        }
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
    pub fn update(&mut self, world: &World, root: usize) -> Vec<PlayingAnimation> {
        let mut sequences = HashMap::<Uuid, PlayingAnimation>::new();
        let mut sources = HashMap::<Uuid, HashMap<Uuid, Instant>>::new();
        for i in members(&world.objects, root) {
            let Some(o) = world.objects.get(i) else { continue };
            for a in world.animations_of(&o.full_id) {
                sources.entry(a.id).or_default().insert(o.full_id, a.continuous_start);
                sequences
                    .entry(a.id)
                    .and_modify(|s| {
                        let continuous_start = s.continuous_start.min(a.continuous_start);
                        if a.sequence > s.sequence || (a.sequence == s.sequence && a.sequence_start < s.sequence_start) {
                            *s = *a;
                        }
                        s.continuous_start = continuous_start;
                    })
                    .or_insert(*a);
            }
        }
        let next: HashMap<_, _> = sequences
            .into_iter()
            .map(|(id, received)| {
                let continuing = sources.get(&id).is_some_and(|current| {
                    current
                        .iter()
                        .any(|(obj, start)| self.sources.get(&id).and_then(|previous| previous.get(obj)) == Some(start))
                });
                let previous = self.playing.get(&id).filter(|_| continuing);
                let mut merged = PlayingAnimation::from_signal(id, received.sequence, received.sequence_start, previous);
                if previous.is_none() {
                    merged.continuous_start = received.continuous_start;
                }
                (id, merged)
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

/// AURORA_DEMO_HAIR_CARDS: one face of `n` strand cards hanging around the
/// back of the head (`head`, the rest position of mHead, joint `joint`), at
/// several distances from it, in a shuffled order rather than back to front,
/// both sides of each card, as the strand cards of a mesh hair (#935).
/// `layered`: listed from the head outwards as a hair is made, the inner
/// cards on the dark half of the texture, the outer ones on the light half,
/// then drawn in the Legacy Viewer's triangle order like a mesh asset;
/// `outer_first`: listed from the outside in.
fn demo_hair_cards(head: Vec3, joint: usize, n: u32, layered: bool, outer_first: bool) -> super::jobs::FaceGeom {
    let mut face = super::jobs::FaceGeom {
        vertices: Vec::new(),
        indices: Vec::new(),
        skin: Some(Vec::new()),
    };
    let mut seed = 0x2545_f491_u32;
    let mut rand = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    let mut cards: Vec<(f32, f32, f32, f32, f32)> = (0..n)
        .map(|_| {
            // around -x, the back of the head the default camera looks at
            let angle = std::f32::consts::PI + (rand() - 0.5) * 3.4;
            let radius = 0.11 + 0.09 * rand();
            (angle, radius, rand(), rand(), rand())
        })
        .collect();
    if layered {
        cards.sort_by(|a, b| if outer_first { b.1.total_cmp(&a.1) } else { a.1.total_cmp(&b.1) });
    }
    for (angle, radius, r_width, r_length, r_u) in cards {
        let (out, side) = (Vec3::new(angle.cos(), angle.sin(), 0.0), Vec3::new(-angle.sin(), angle.cos(), 0.0));
        let center = head + Vec3::new(0.0, 0.0, 0.04) + out * radius;
        let (half_width, top, bottom) = (0.05 + 0.03 * r_width, 0.12, -0.32 - 0.12 * r_length);
        let u0 = if layered {
            (r_u * 2.0).floor() * 0.25 + if radius < 0.155 { 0.0 } else { 0.5 }
        } else {
            (r_u * 4.0).floor() * 0.25
        };
        for facing in [1.0f32, -1.0] {
            let base = face.vertices.len() as u16;
            for (dz, v) in [(top, 0.0), (bottom, 1.0)] {
                for (dx, u) in [(-half_width, u0), (half_width, u0 + 0.25)] {
                    let p = center + side * dx + Vec3::Z * dz;
                    face.vertices
                        .push(aurora_render::Vertex::new(p.to_array(), (out * facing).to_array(), [u, v]));
                    if let Some(s) = &mut face.skin {
                        s.push(aurora_render::SkinVertex {
                            joints: [joint as u8, 0, 0, 0],
                            weights: [255, 0, 0, 0],
                        });
                    }
                }
            }
            let quad = if facing > 0.0 { [0, 2, 3, 0, 3, 1] } else { [0, 3, 2, 0, 1, 3] };
            face.indices.extend(quad.map(|k| base + k));
        }
    }
    if layered && super::jobs::legacy_triangle_order() {
        aurora_assets::legacy_triangle_order(&mut face.indices, face.vertices.len());
    }
    face
}

impl super::Scene {
    pub fn log_demo_animesh(&self, world: &World) {
        let Some(&head) = self.avatar_lib.rig.names.get("mHead") else {
            return;
        };
        for local in [9200, 9201, 9202, 9203, 9205, 9206] {
            let Some((i, _)) = world.objects.iter().find(|(_, o)| o.key.local_id == local) else {
                continue;
            };
            let Some(g) = self.gpu.get(i) else { continue };
            let pose = g
                .skeleton_owner
                .and_then(|id| self.palette_slots.get(&id))
                .and_then(|slot| self.palettes.get(*slot as usize * super::anim::PALETTE_JOINTS + head))
                .map(|p| Mat4::from_cols_array_2d(p).to_scale_rotation_translation().1);
            // faces that cast (the GPU lists keep the casters on the GPU):
            // blended ones too (LLPipeline::renderShadow), unless
            // AURORA_BLEND_SHADOWS=0 leaves out those that are not rigged
            let blend_casts = aurora_render::shadow_cull::blend_casts();
            let casters = g
                .faces
                .iter()
                .filter(|f| f.pass != super::Pass::Hidden && (f.pass != super::Pass::Blend || g.rigged || g.is_avatar || blend_casts))
                .count();
            log::info!(
                "demo animesh {local}: owner {:?}, head {:?}, shadow faces {casters}, center {:.3?}, radius {:.3}",
                g.skeleton_owner,
                pose,
                g.center,
                g.radius
            );
            if std::env::var("AURORA_DEMO_ANIMESH").is_ok_and(|v| matches!(v.as_str(), "alpha" | "alpha-cycle")) {
                let red = g
                    .faces
                    .iter()
                    .filter(|f| self.lists.debug_red.iter().any(|c| c.record == f.record))
                    .count();
                log::info!(
                    "demo animesh alpha {local}: records {}, red {red}, rigged {}, overlay {:?}",
                    g.faces.len(),
                    g.rigged,
                    self.debug_alpha
                );
            }
        }
        // (AURORA_DEMO_ANIMESH=churn: stops growing once meshes are freed)
        let (used, len) = self.skin_binds.counts();
        log::info!("demo animesh: skin bindings {used} used of {len}");
    }
    /// Skin and geometry of the synthetic quadruped as mesh asset `id` (the
    /// demo's own, or the next one of AURORA_DEMO_ANIMESH=churn).
    pub fn install_demo_animesh_mesh(&mut self, renderer: &mut aurora_render::Renderer, id: Uuid) {
        self.install_demo_rigged_mesh(renderer, id, None);
    }

    /// The same skin with, for `dense`, that many triangles in spheres around
    /// the chest, a few centimeters apart (AURORA_DEMO_CROWD=<n>,<worn>: the
    /// triangle count of a worn mesh body, for the picking tests).
    pub fn install_demo_rigged_mesh(&mut self, renderer: &mut aurora_render::Renderer, id: Uuid, dense: Option<u32>) {
        use super::jobs::FaceGeom;
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
        self.meshes.insert_demo_skin(id, skin);
        if let Some(triangles) = dense {
            let Some(&j) = rig.names.get("mChest") else { return };
            let center = rest[j].w_axis.truncate();
            // (a face holds 100 × 100 quads: within 16-bit indices)
            const N: u32 = 100;
            let count = triangles.div_ceil(2 * N * N).max(1);
            let faces: Vec<_> = (0..count)
                .map(|f| {
                    let radius = 0.3 - 0.02 * f as f32 / count as f32;
                    let mut face = FaceGeom {
                        vertices: Vec::new(),
                        indices: Vec::new(),
                        skin: Some(Vec::new()),
                    };
                    for y in 0..=N {
                        let (sin_lat, cos_lat) = (y as f32 / N as f32 * std::f32::consts::PI).sin_cos();
                        for x in 0..=N {
                            let (sin_lon, cos_lon) = (x as f32 / N as f32 * std::f32::consts::TAU).sin_cos();
                            let n = Vec3::new(sin_lat * cos_lon, sin_lat * sin_lon, cos_lat);
                            face.vertices.push(aurora_render::Vertex::new(
                                (center + n * radius).to_array(),
                                n.to_array(),
                                [x as f32 / N as f32, y as f32 / N as f32],
                            ));
                            if let Some(s) = &mut face.skin {
                                s.push(SkinVertex {
                                    joints: [j as u8, 0, 0, 0],
                                    weights: [255, 0, 0, 0],
                                });
                            }
                        }
                    }
                    for y in 0..N {
                        for x in 0..N {
                            let a = (y * (N + 1) + x) as u16;
                            let b = a + N as u16 + 1;
                            face.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
                        }
                    }
                    Some(face)
                })
                .collect();
            for lod in 0..4 {
                let faces = faces
                    .iter()
                    .map(|f| {
                        f.as_ref().map(|f| FaceGeom {
                            vertices: f.vertices.clone(),
                            indices: f.indices.clone(),
                            skin: f.skin.clone(),
                        })
                    })
                    .collect();
                self.upload_geom(
                    renderer,
                    super::GeomKey::Mesh { id, lod },
                    super::jobs::PreparedGeom::new(faces, center - Vec3::splat(0.3), center + Vec3::splat(0.3), true, None),
                );
            }
            return;
        }
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
        // AURORA_DEMO_ANIMESH_TRIS=<n>: about n more triangles, as faces of
        // their own (a fine tube around the chest each), so that what wears
        // the mesh weighs what an avatar of the grid does
        let mut dense = Vec::new();
        if let Some(&j) = rig.names.get("mChest") {
            const SIDE: usize = 150;
            let center = rest[j].w_axis.truncate();
            for k in 0..crate::demo::animesh_dense_triangles().div_ceil(2 * SIDE * SIDE).min(7) {
                let mut tube = FaceGeom {
                    vertices: Vec::with_capacity((SIDE + 1) * (SIDE + 1)),
                    indices: Vec::with_capacity(6 * SIDE * SIDE),
                    skin: Some(vec![
                        SkinVertex {
                            joints: [j as u8, 0, 0, 0],
                            weights: [255, 0, 0, 0],
                        };
                        (SIDE + 1) * (SIDE + 1)
                    ]),
                };
                let radius = 0.2 + 0.015 * k as f32;
                for ring in 0..=SIDE {
                    for seg in 0..=SIDE {
                        let (u, v) = (seg as f32 / SIDE as f32, ring as f32 / SIDE as f32);
                        let (sin, cos) = (u * std::f32::consts::TAU).sin_cos();
                        let p = center + Vec3::new(cos * radius, sin * radius, (v - 0.5) * 0.5);
                        tube.vertices
                            .push(aurora_render::Vertex::new(p.to_array(), [cos, sin, 0.0], [u, v]));
                    }
                }
                for ring in 0..SIDE {
                    for seg in 0..SIDE {
                        let a = (ring * (SIDE + 1) + seg) as u16;
                        let b = a + SIDE as u16 + 1;
                        tube.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
                    }
                }
                dense.push(tube);
            }
        }
        for lod in 0..4 {
            let mut faces = vec![Some(FaceGeom {
                vertices: face.vertices.clone(),
                indices: face.indices.clone(),
                skin: face.skin.clone(),
            })];
            faces.extend(dense.iter().map(|t| {
                Some(FaceGeom {
                    vertices: t.vertices.clone(),
                    indices: t.indices.clone(),
                    skin: t.skin.clone(),
                })
            }));
            self.upload_geom(
                renderer,
                super::GeomKey::Mesh { id, lod },
                super::jobs::PreparedGeom::new(faces, Vec3::splat(-2.0), Vec3::splat(2.0), true, None),
            );
        }
    }

    /// AURORA_DEMO_HAIR: one rigged mesh per layer, a shell around the head
    /// (bound to mHead) and one around the torso (mChest), each its own
    /// face, slightly larger from one layer to the next. With
    /// AURORA_DEMO_LASHES, a small ball inside the head shell too.
    pub fn install_demo_hair(&mut self, renderer: &mut aurora_render::Renderer, demo: crate::demo::HairDemo) {
        use super::jobs::FaceGeom;
        use aurora_render::SkinVertex;
        let rig = self.avatar_lib.rig.clone();
        let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
        let mut names = vec![String::new(); rig.len()];
        for (name, &j) in &rig.names {
            if name.starts_with('m') {
                names[j] = name.clone();
            }
        }
        let seg = demo.segments;
        // a latitude / longitude sphere around a joint, the strands running down it
        let sphere = |name: &str, offset: Vec3, radii: Vec3| -> Option<FaceGeom> {
            let &j = rig.names.get(name)?;
            let center = rest[j].w_axis.truncate() + offset;
            let mut face = FaceGeom {
                vertices: Vec::new(),
                indices: Vec::new(),
                skin: Some(Vec::new()),
            };
            for row in 0..=seg {
                let lat = std::f32::consts::PI * row as f32 / seg as f32;
                for col in 0..=seg {
                    let lon = std::f32::consts::TAU * col as f32 / seg as f32;
                    let n = Vec3::new(lat.sin() * lon.cos(), lat.sin() * lon.sin(), lat.cos());
                    let p = center + n * radii;
                    let uv = [4.0 * col as f32 / seg as f32, row as f32 / seg as f32];
                    face.vertices.push(aurora_render::Vertex::new(p.to_array(), n.to_array(), uv));
                    if let Some(s) = &mut face.skin {
                        s.push(SkinVertex {
                            joints: [j as u8, 0, 0, 0],
                            weights: [255, 0, 0, 0],
                        });
                    }
                }
            }
            let at = |row: u32, col: u32| (row * (seg + 1) + col) as u16;
            for row in 0..seg {
                for col in 0..seg {
                    let (a, b, c, d) = (at(row, col), at(row + 1, col), at(row + 1, col + 1), at(row, col + 1));
                    face.indices.extend([a, b, c, a, c, d]);
                }
            }
            Some(face)
        };
        let mut meshes: Vec<(Uuid, Vec<Option<FaceGeom>>)> = (0..demo.layers)
            .map(|layer| {
                let grow = 0.012 * layer as f32;
                let faces = [
                    ("mHead", Vec3::new(0.0, 0.0, 0.07), Vec3::splat(0.14)),
                    ("mChest", Vec3::new(0.0, 0.0, -0.05), Vec3::new(0.2, 0.25, 0.34)),
                ]
                .into_iter()
                .filter_map(|(name, offset, radii)| sphere(name, offset, radii + grow).map(Some))
                .collect();
                (crate::demo::hair_mesh(layer), faces)
            })
            .collect();
        if demo.cards > 0
            && let Some(&j) = rig.names.get("mHead")
        {
            meshes.push((
                crate::demo::HAIR_CARDS_MESH,
                vec![Some(demo_hair_cards(
                    rest[j].w_axis.truncate(),
                    j,
                    demo.cards,
                    demo.layered,
                    demo.outer_first,
                ))],
            ));
        }
        if demo.lashes {
            // behind the face for the default camera (which looks at the
            // back of the head): nearer the camera than the centre of the
            // hair around it, as eyelashes are nearer than a fringe's centre
            meshes.push((
                crate::demo::LASHES_MESH,
                vec![sphere("mHead", Vec3::new(-0.105, 0.0, 0.07), Vec3::splat(0.03))],
            ));
        }
        if demo.sheer {
            // two torso shells 1.5 cm apart: the layer, then the dress over it
            for (id, grow) in [(crate::demo::SHEER_LAYER_MESH, 0.015), (crate::demo::SHEER_DRESS_MESH, 0.03)] {
                let radii = Vec3::new(0.2, 0.25, 0.34) + grow;
                meshes.push((id, vec![sphere("mChest", Vec3::new(0.0, 0.0, -0.05), radii)]));
            }
        }
        if demo.soft_mask {
            // pelvis shell: a Mask-classified soft fringe that must Blend when
            // rigged (LLFace::canRenderAsMask never auto-masks skinned faces)
            meshes.push((
                crate::demo::SOFT_MASK_MESH,
                vec![sphere("mPelvis", Vec3::new(0.0, 0.0, -0.02), Vec3::new(0.12, 0.1, 0.14))],
            ));
        }
        for (id, faces) in meshes {
            self.meshes.insert_demo_skin(
                id,
                aurora_assets::SkinInfo {
                    joint_names: names.clone(),
                    inverse_bind: rest.iter().map(|m| m.inverse()).collect(),
                    ..Default::default()
                },
            );
            for lod in 0..4 {
                let faces = faces
                    .iter()
                    .map(|f| {
                        f.as_ref().map(|f| FaceGeom {
                            vertices: f.vertices.clone(),
                            indices: f.indices.clone(),
                            skin: f.skin.clone(),
                        })
                    })
                    .collect();
                self.upload_geom(
                    renderer,
                    super::GeomKey::Mesh { id, lod },
                    super::jobs::PreparedGeom::new(faces, Vec3::splat(-2.0), Vec3::splat(2.0), true, None),
                );
            }
        }
    }

    /// AURORA_DEMO_PHYSICS: the balls of `crate::demo::physics::BALLS`, each
    /// a face skinned to its collision volume only, as the breasts, belly
    /// and butt of a mesh body.
    pub fn install_demo_physics_body(&mut self, renderer: &mut aurora_render::Renderer) {
        use super::jobs::FaceGeom;
        use aurora_render::SkinVertex;
        const SEG: u32 = 16;
        let rig = self.avatar_lib.rig.clone();
        let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
        let mut names = vec![String::new(); rig.len()];
        for (name, &j) in &rig.names {
            names[j] = name.clone();
        }
        let faces: Vec<Option<FaceGeom>> = crate::demo::physics::BALLS
            .iter()
            .filter_map(|&(name, offset, radii)| {
                let &j = rig.names.get(name)?;
                let center = rest[j].w_axis.truncate() + offset;
                let mut vertices = Vec::new();
                for row in 0..=SEG {
                    let lat = std::f32::consts::PI * row as f32 / SEG as f32;
                    for col in 0..=SEG {
                        let lon = std::f32::consts::TAU * col as f32 / SEG as f32;
                        let n = Vec3::new(lat.sin() * lon.cos(), lat.sin() * lon.sin(), lat.cos());
                        let uv = [col as f32 / SEG as f32, row as f32 / SEG as f32];
                        vertices.push(aurora_render::Vertex::new((center + n * radii).to_array(), n.to_array(), uv));
                    }
                }
                let at = |row: u32, col: u32| (row * (SEG + 1) + col) as u16;
                let mut indices = Vec::new();
                for row in 0..SEG {
                    for col in 0..SEG {
                        let (a, b, c, d) = (at(row, col), at(row + 1, col), at(row + 1, col + 1), at(row, col + 1));
                        indices.extend([a, b, c, a, c, d]);
                    }
                }
                let skin = vec![
                    SkinVertex {
                        joints: [j as u8, 0, 0, 0],
                        weights: [255, 0, 0, 0],
                    };
                    vertices.len()
                ];
                Some(Some(FaceGeom {
                    vertices,
                    indices,
                    skin: Some(skin),
                }))
            })
            .collect();
        self.meshes.insert_demo_skin(
            crate::demo::physics::BODY_MESH,
            aurora_assets::SkinInfo {
                joint_names: names,
                inverse_bind: rest.iter().map(|m| m.inverse()).collect(),
                ..Default::default()
            },
        );
        for lod in 0..4 {
            let faces = faces
                .iter()
                .map(|f| {
                    f.as_ref().map(|f| FaceGeom {
                        vertices: f.vertices.clone(),
                        indices: f.indices.clone(),
                        skin: f.skin.clone(),
                    })
                })
                .collect();
            self.upload_geom(
                renderer,
                super::GeomKey::Mesh {
                    id: crate::demo::physics::BODY_MESH,
                    lod,
                },
                super::jobs::PreparedGeom::new(faces, Vec3::splat(-2.0), Vec3::splat(2.0), true, None),
            );
        }
    }

    /// Synthetic quadruped with a custom rest skeleton and smooth alpha fur.
    /// AURORA_DEMO_ANIMESH=points: a bar rigged to the attachment point
    /// "Groin" alone (#1492): the point is a joint of the skeleton, which
    /// the animation turns (LLVOAvatar::getJoint finds attachment points,
    /// indra/newview/llvoavatar.cpp).
    fn install_demo_point_mesh(&mut self, renderer: &mut aurora_render::Renderer) {
        use aurora_render::SkinVertex;
        let rig = self.avatar_lib.rig.clone();
        let Some(&(joint, local)) = rig.skin_points.get(crate::demo::ANIMESH_POINT) else {
            return;
        };
        let point = rig.default_world_inv[joint].inverse() * local;
        self.meshes.insert_demo_skin(
            crate::demo::ANIMESH_POINT_MESH,
            aurora_assets::SkinInfo {
                joint_names: vec![crate::demo::ANIMESH_POINT.to_string()],
                inverse_bind: vec![point.inverse()],
                ..Default::default()
            },
        );
        // a bar 0.4 m long ahead of the point, in skeleton space
        let bar = point * Mat4::from_scale_rotation_translation(Vec3::new(0.4, 0.05, 0.05), glam::Quat::IDENTITY, Vec3::new(0.2, 0.0, 0.0));
        let cube = aurora_prim::generate_volume(&aurora_prim::VolumeParams::default(), 1.0);
        let (mut faces, _, _) = super::jobs::volume_to_faces(&cube);
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for face in faces.iter_mut().flatten() {
            for v in &mut face.vertices {
                let p = bar.transform_point3(Vec3::from(v.pos));
                (min, max) = (min.min(p), max.max(p));
                v.pos = p.to_array();
            }
            face.skin = Some(vec![
                SkinVertex {
                    joints: [0, 0, 0, 0],
                    weights: [255, 0, 0, 0],
                };
                face.vertices.len()
            ]);
        }
        for lod in 0..4 {
            let faces = faces
                .iter()
                .map(|f| {
                    f.as_ref().map(|f| super::jobs::FaceGeom {
                        vertices: f.vertices.clone(),
                        indices: f.indices.clone(),
                        skin: f.skin.clone(),
                    })
                })
                .collect();
            self.upload_geom(
                renderer,
                super::GeomKey::Mesh {
                    id: crate::demo::ANIMESH_POINT_MESH,
                    lod,
                },
                super::jobs::PreparedGeom::new(faces, min, max, true, None),
            );
        }
    }

    pub fn install_demo_animesh(&mut self, renderer: &mut aurora_render::Renderer) {
        use super::jobs::JobResult;
        self.install_demo_animesh_mesh(renderer, crate::demo::ANIMESH_MESH);
        if crate::demo::animesh_points() {
            self.install_demo_point_mesh(renderer);
        }
        if std::env::var("AURORA_DEMO_ANIMESH").is_ok_and(|v| matches!(v.as_str(), "alpha" | "alpha-cycle")) {
            // No skin metadata or skin vertex stream: sharing an animated
            // parent must not make these mesh children rigged.
            let cube = aurora_prim::generate_volume(&aurora_prim::VolumeParams::default(), 1.0);
            for lod in 0..4 {
                let (faces, min, max) = super::jobs::volume_to_faces(&cube);
                self.upload_geom(
                    renderer,
                    super::GeomKey::Mesh {
                        id: crate::demo::ANIMESH_UNRIGGED_MESH,
                        lod,
                    },
                    super::jobs::PreparedGeom::new(faces, min, max, true, None),
                );
            }
        }
        let rig = self.avatar_lib.rig.clone();
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
            pick_mask: super::textures::PickMask::from_rgba(32, 32, &rgba),
            mips: vec![(32, 32, rgba)],
            staged: None,
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

    /// `members` by its definition: the descendants whose `owner` is `root`.
    fn members_by_owner(objects: &ObjectStore, root: usize) -> Vec<usize> {
        let mut result = Vec::new();
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            let Some(o) = objects.get(i) else { continue };
            if i != root && owner(objects, i).is_none_or(|(_, p)| p != root) {
                continue;
            }
            result.push(i);
            stack.extend_from_slice(objects.children_of(&o.key));
        }
        result.sort_unstable();
        result
    }

    #[test]
    fn members_match_their_owner_definition() {
        let mut w = fixture();
        for e in crate::demo::crowd_events(3, true) {
            w.apply(e);
        }
        // an animated attachment with a child, and a prim linked below a child
        let worn = index(&w, 9203);
        let region = w.objects.get(worn).expect("worn").key.region;
        let mut u = crate::demo::action_avatar(false);
        u.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        u.local_id = 9250;
        u.full_id = Uuid::from_u128(9250);
        u.parent_id = 9203;
        w.objects.upsert(region, u.clone());
        u.local_id = 9251;
        u.full_id = Uuid::from_u128(9251);
        u.parent_id = 9202;
        w.objects.upsert(region, u);
        let mut checked = 0;
        for (idx, _) in w.objects.iter() {
            let mut fast = members(&w.objects, idx);
            fast.sort_unstable();
            assert_eq!(fast, members_by_owner(&w.objects, idx), "members of {idx}");
            checked += 1;
        }
        assert!(checked > 100);
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
        let mut signals = Signals::default();
        let first = signals.update(&w, root);
        assert_eq!(first.len(), 1); // Only the child has a signal.
        assert_eq!(first[0], w.animations_of(&child_id)[0]);
        w.apply(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar: root_id,
            anims: vec![(id, 9)],
        });
        let second = signals.update(&w, root);
        assert_eq!(second[0].sequence, 9);
        assert_eq!(second[0].sequence_start, w.animations_of(&root_id)[0].sequence_start);
        assert_eq!(second[0].continuous_start, first[0].continuous_start);
        w.apply(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar: root_id,
            anims: vec![],
        });
        let third = signals.update(&w, root);
        assert_eq!(third[0].sequence, 1);
        assert_eq!(third[0].continuous_start, first[0].continuous_start);
        w.apply(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar: child_id,
            anims: vec![],
        });
        assert!(signals.update(&w, root).is_empty());
        w.apply(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar: child_id,
            anims: vec![(id, 10)],
        });
        let restarted = signals.update(&w, root);
        assert!(restarted[0].continuous_start > first[0].continuous_start);
        w.apply(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar: child_id,
            anims: vec![],
        });
        w.apply(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar: child_id,
            anims: vec![(id, 11)],
        });
        let between_frames = signals.update(&w, root);
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
