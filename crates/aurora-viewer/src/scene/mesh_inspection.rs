//! Local build inspection, following LLPanelObject::activateMeshFields and
//! LLViewerWindow::renderSelections. Mesh work stays on the existing workers;
//! the UI only reads the cached geometry's actual index counts.
use super::*;
use crate::world::objects::ObjKey;

/// A bounded force-high request, independent of the inspector's chosen LOD.
pub(super) struct LodReset {
    id: Uuid,
    frame: u32,
    since: Instant,
}

pub(super) fn effective_lod(
    overrides: &HashMap<ObjKey, u8>,
    resets: &HashMap<ObjKey, LodReset>,
    object: &Object,
    hud: bool,
    automatic: u8,
) -> u8 {
    if object.visual_volume().is_mesh() && resets.get(&object.key).is_some_and(|reset| reset.id == object.full_id) {
        3
    } else {
        display_lod(overrides, object, hud, automatic)
    }
}

pub struct MeshInfo {
    /// Lowest through highest. Missing/failed levels differ from a true zero.
    pub triangles: [Option<usize>; 4],
    pub faces: Option<usize>,
    pub radius: Option<f32>,
    pub rigged: bool,
}

pub(super) fn display_lod(overrides: &HashMap<ObjKey, u8>, object: &Object, hud: bool, automatic: u8) -> u8 {
    if hud {
        return 3;
    }
    if object.visual_volume().is_mesh() {
        return overrides.get(&object.key).copied().unwrap_or(automatic);
    }
    automatic
}

impl Scene {
    /// reset_mesh_lod (indra/newview/llviewermenu.cpp): force each worn mesh
    /// through its highest LOD briefly, then resume its ordinary selection.
    pub fn reset_mesh_lods(&mut self, world: &mut World, roots: &[Uuid], now: Instant) {
        let indices: Vec<_> = roots
            .iter()
            .filter_map(|id| world.objects.index_of_uuid(id))
            .flat_map(|root| crate::build::family(world, root))
            .collect();
        for index in indices {
            let Some(object) = world.objects.get(index).filter(|object| object.visual_volume().is_mesh()) else {
                continue;
            };
            let key = object.key;
            self.mesh_lod_resets.insert(
                key,
                LodReset {
                    id: object.full_id,
                    frame: self.frame,
                    since: now,
                },
            );
            if let Some(object) = world.objects.get_mut_untracked(index) {
                object.render.needs_records = true;
            }
            self.sets.dirty.insert(index);
        }
    }

    pub(super) fn finish_mesh_lod_resets(&mut self, world: &mut World, now: Instant) {
        let finished: Vec<_> = self
            .mesh_lod_resets
            .iter()
            .filter_map(|(key, reset)| {
                let index = world.objects.index_of(key);
                let valid = index
                    .and_then(|index| world.objects.get(index))
                    .is_some_and(|object| object.full_id == reset.id && object.visual_volume().is_mesh());
                let high = index
                    .and_then(|index| self.gpu.get(index))
                    .is_some_and(|gpu| matches!(gpu.geom, Some(GeomKey::Mesh { lod: 3, .. })));
                let expired = now.saturating_duration_since(reset.since) >= std::time::Duration::from_secs(5);
                (!valid || self.frame.wrapping_sub(reset.frame) > 0 && (high || expired)).then_some(*key)
            })
            .collect();
        for key in finished {
            self.mesh_lod_resets.remove(&key);
            if let Some(index) = world.objects.index_of(&key) {
                if let Some(object) = world.objects.get_mut_untracked(index) {
                    object.render.needs_records = true;
                }
                self.sets.dirty.insert(index);
            }
        }
    }

    /// Retain at most one mesh's four levels. The normal async streaming,
    /// decoding, upload and cache eviction paths are shared with rendering.
    pub fn inspect_mesh(&mut self, world: &World, primary: Option<ObjKey>) {
        let object = primary
            .and_then(|key| world.objects.index_of(&key))
            .and_then(|idx| world.objects.get(idx));
        let id = object
            .filter(|o| o.visual_volume().is_mesh())
            .and_then(|o| o.visual_volume().sculpt.map(|s| s.texture));
        if id == self.inspected_mesh {
            return;
        }
        if let Some(old) = self.inspected_mesh.take() {
            for lod in 0..4 {
                self.geom_unref(GeomKey::Mesh { id: old, lod });
            }
        }
        self.inspected_mesh = id;
        if let (Some(id), Some(object)) = (id, object) {
            for lod in 0..4 {
                let key = GeomKey::Mesh { id, lod };
                self.request_geom(key, object, MeshNeed { px: 512.0, own: false });
                self.geom_ref(key);
            }
        }
    }

    /// Mark only changed objects dirty, so forcing/restoring a LOD works even
    /// with a stationary camera and the event-based LOD watcher.
    pub fn set_mesh_lod_override(&mut self, world: &mut World, objects: &[ObjKey], level: Option<u8>) {
        let next: HashMap<_, _> = level
            .filter(|&l| l < 4)
            .map(|level| {
                objects
                    .iter()
                    .copied()
                    .filter(|key| {
                        world
                            .objects
                            .index_of(key)
                            .and_then(|i| world.objects.get(i))
                            .is_some_and(|o| o.visual_volume().is_mesh())
                    })
                    .map(|key| (key, level))
                    .collect()
            })
            .unwrap_or_default();
        if next == self.mesh_lod_overrides {
            return;
        }
        for key in self.mesh_lod_overrides.keys().chain(next.keys()) {
            if self.mesh_lod_overrides.get(key) != next.get(key)
                && let Some(idx) = world.objects.index_of(key)
            {
                // SyncSets discards dirty entries without a render change.
                // This local flag also makes restoring an empty LOD immediate;
                // no simulator update or camera movement is needed.
                if let Some(object) = world.objects.get_mut_untracked(idx) {
                    object.render.needs_records = true;
                }
                self.sets.dirty.insert(idx);
            }
        }
        self.mesh_lod_overrides = next;
    }

    pub fn mesh_info(&self, world: &World, idx: usize) -> Option<MeshInfo> {
        let object = world.objects.get(idx)?;
        if !object.visual_volume().is_mesh() {
            return None;
        }
        let id = object.visual_volume().sculpt?.texture;
        let view = self.meshes.view(&id);
        let triangles = std::array::from_fn(|lod| {
            if view.as_ref().is_some_and(|v| v.header.lods[lod].is_none()) {
                return None;
            }
            self.geom_ready(&GeomKey::Mesh { id, lod: lod as u8 })
                .map(|g| g.faces.iter().flatten().map(|f| (f.index_count / 3) as usize).sum())
        });
        let rigged = view.as_ref().is_some_and(|v| v.skin.is_some());
        let radius = if rigged {
            Self::skeleton_owner(world, idx)
                .and_then(|(_, owner)| self.gpu.get(owner))
                .map(|g| g.radius)
        } else {
            Some(object.visual_scale().length() * 0.5)
        }
        .filter(|r| r.is_finite() && *r > 0.0);
        Some(MeshInfo {
            triangles,
            faces: self.geom_ready(&GeomKey::Mesh { id, lod: 3 }).map(|g| g.faces.len()),
            radius,
            rigged,
        })
    }
}

/// LLPanelObject::setLODDistValues: cap the base distance BEFORE the factor.
pub fn switch_distances(radius: f32, factor: f32) -> Option<[f32; 3]> {
    if !radius.is_finite() || radius <= 0.0 || !factor.is_finite() || factor <= 0.0 {
        return None;
    }
    Some([0.24, 0.06, 0.03].map(|threshold| (radius / threshold).min(512.0) * factor))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stationary_frame(scene: &mut Scene, world: &World) -> Vec<usize> {
        let state = sync_plan::FrameState {
            gpu: &scene.gpu,
            geoms: &scene.geoms,
            textures: &scene.textures,
            mat_gen: scene.materials.generation,
            mat_changes: &scene.mat_changes,
            still_huds: true,
        };
        scene
            .sets
            .frame_list(
                &world.objects,
                world.agent_id,
                1,
                world.objects.slots.len(),
                &state,
                sync_sets::LodChecks::Events {
                    anchored: &[],
                    dt: 0.0,
                    camera_moved: false,
                },
            )
            .items
            .into_iter()
            .map(|(idx, _)| idx)
            .collect()
    }

    fn fixture() -> (World, Scene, usize) {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for event in crate::demo::events()
            .into_iter()
            .chain(crate::demo::complex_face_selection_events())
        {
            world.apply(event);
        }
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(99621)).unwrap();
        let mut scene = Scene::new(Default::default(), lib);
        for (lod, triangles) in [0, 2, 7, 10].into_iter().enumerate() {
            scene.geom_insert(
                GeomKey::Mesh {
                    id: crate::demo::FACE_SELECTION_MESH,
                    lod: lod as u8,
                },
                GeomState::Ready(Arc::new(GpuGeom {
                    faces: vec![Some(MeshAlloc {
                        vertex_offset: 0,
                        vertex_count: 0,
                        index_offset: 0,
                        index_count: triangles * 3,
                    })],
                    min: Vec3::splat(-0.5),
                    max: Vec3::splat(0.5),
                    joint_bounds: Vec::new(),
                    pick_faces: Vec::new(),
                })),
            );
        }
        (world, scene, idx)
    }

    #[test]
    fn mesh_inspection_reads_real_index_counts_retains_once_and_releases_all_levels() {
        let (world, mut scene, idx) = fixture();
        let key = world.objects.get(idx).unwrap().key;
        scene.inspect_mesh(&world, Some(key));
        scene.inspect_mesh(&world, Some(key));
        let info = scene.mesh_info(&world, idx).unwrap();
        assert_eq!(info.triangles, [Some(0), Some(2), Some(7), Some(10)]);
        assert!((info.radius.unwrap() - 3f32.sqrt()).abs() < 0.0001);
        for lod in 0..4 {
            assert_eq!(
                scene.geoms[&GeomKey::Mesh {
                    id: crate::demo::FACE_SELECTION_MESH,
                    lod
                }]
                    .refs,
                1
            );
        }
        scene.geoms.remove(&GeomKey::Mesh {
            id: crate::demo::FACE_SELECTION_MESH,
            lod: 2,
        });
        assert_eq!(scene.mesh_info(&world, idx).unwrap().triangles[2], None);
        scene.inspect_mesh(&world, None);
        assert!(scene.geoms.values().all(|g| g.refs == 0));
    }

    #[test]
    fn forcing_each_lod_marks_selected_meshes_dirty_and_restores_automatic_without_changing_objects() {
        let (mut world, mut scene, idx) = fixture();
        let object = world.objects.get(idx).unwrap();
        let (key, scale) = (object.key, object.scale);
        for level in 0..4 {
            scene.set_mesh_lod_override(&mut world, &[key], Some(level));
            assert!(scene.sets.dirty.contains(idx));
            let object = world.objects.get(idx).unwrap();
            assert_eq!(display_lod(&scene.mesh_lod_overrides, object, false, 3), level);
            assert_eq!(display_lod(&scene.mesh_lod_overrides, object, true, 0), 3);
        }
        scene.set_mesh_lod_override(&mut world, &[], None);
        assert_eq!(display_lod(&scene.mesh_lod_overrides, world.objects.get(idx).unwrap(), false, 2), 2);
        assert_eq!(world.objects.get(idx).unwrap().scale, scale);
        scene.set_mesh_lod_override(&mut world, &[key], Some(9));
        assert!(scene.mesh_lod_overrides.is_empty());
    }

    #[test]
    fn stationary_mesh_is_scheduled_on_every_lod_change_and_restore_even_after_an_empty_lod() {
        let (mut world, mut scene, idx) = fixture();
        let key = world.objects.get(idx).unwrap().key;
        scene.gpu.resize_with(world.objects.slots.len(), ObjGpu::default);
        for object in world.objects.slots.iter_mut().flatten() {
            object.render.needs_records = false;
            object.shape_dirty = false;
            object.material_dirty = false;
        }
        assert!(stationary_frame(&mut scene, &world).is_empty());
        // No moving camera, object updates, materials, or periodic sweep can
        // rescue a dropped local invalidation here. Lowest has zero triangles.
        for level in [Some(0), Some(3), Some(1), Some(2), None] {
            scene.set_mesh_lod_override(&mut world, &[key], level);
            assert!(
                stationary_frame(&mut scene, &world).contains(&idx),
                "LOD {level:?} was not scheduled"
            );
            world.objects.get_mut_untracked(idx).unwrap().render.needs_records = false;
            scene.set_mesh_lod_override(&mut world, &[key], level);
            assert!(
                stationary_frame(&mut scene, &world).is_empty(),
                "unchanged choice was scheduled again"
            );
        }
    }

    #[test]
    fn mesh_reset_temporarily_forces_high_and_restores_manual_lod_even_when_stationary() {
        let (mut world, mut scene, idx) = fixture();
        let object = world.objects.get(idx).unwrap();
        let (key, id) = (object.key, object.full_id);
        scene.gpu.resize_with(world.objects.slots.len(), ObjGpu::default);
        scene.set_mesh_lod_override(&mut world, &[key], Some(0));
        let now = Instant::now();
        scene.reset_mesh_lods(&mut world, &[id], now);
        assert_eq!(
            effective_lod(
                &scene.mesh_lod_overrides,
                &scene.mesh_lod_resets,
                world.objects.get(idx).unwrap(),
                false,
                1
            ),
            3
        );
        scene.gpu[idx].geom = Some(GeomKey::Mesh {
            id: crate::demo::FACE_SELECTION_MESH,
            lod: 3,
        });
        scene.finish_mesh_lod_resets(&mut world, now);
        assert!(!scene.mesh_lod_resets.is_empty(), "at least one forced synchronization");
        scene.frame = scene.frame.wrapping_add(1);
        world.objects.get_mut_untracked(idx).unwrap().render.needs_records = false;
        scene.sets.dirty.remove(idx);
        scene.finish_mesh_lod_resets(&mut world, now);
        assert!(scene.mesh_lod_resets.is_empty());
        assert!(scene.sets.dirty.contains(idx));
        assert!(world.objects.get(idx).unwrap().render.needs_records);
        assert_eq!(
            effective_lod(
                &scene.mesh_lod_overrides,
                &scene.mesh_lod_resets,
                world.objects.get(idx).unwrap(),
                false,
                1
            ),
            0
        );
    }

    #[test]
    fn mesh_reset_is_bounded_on_failed_streaming_and_never_follows_a_reused_identity() {
        let (mut world, mut scene, idx) = fixture();
        let object = world.objects.get(idx).unwrap();
        let (key, id) = (object.key, object.full_id);
        let now = Instant::now();
        scene.reset_mesh_lods(&mut world, &[id], now);
        scene.frame = scene.frame.wrapping_add(1);
        scene.finish_mesh_lod_resets(&mut world, now + std::time::Duration::from_secs(6));
        assert!(scene.mesh_lod_resets.is_empty());
        scene.reset_mesh_lods(&mut world, &[id], now);
        world.objects.get_mut_untracked(idx).unwrap().full_id = Uuid::new_v4();
        assert_eq!(
            effective_lod(
                &scene.mesh_lod_overrides,
                &scene.mesh_lod_resets,
                world.objects.get(idx).unwrap(),
                false,
                1
            ),
            1
        );
        scene.finish_mesh_lod_resets(&mut world, now);
        assert!(!scene.mesh_lod_resets.contains_key(&key));
    }

    #[test]
    fn indicative_distances_match_reference_and_cap_before_factor() {
        let rounded = |values: [f32; 3]| values.map(|v| (v * 10.0).round() / 10.0);
        assert_eq!(rounded(switch_distances(1.517, 1.25).unwrap()), [7.9, 31.6, 63.2]);
        assert_eq!(rounded(switch_distances(1.517, 2.0).unwrap()), [12.6, 50.6, 101.1]);
        assert_eq!(rounded(switch_distances(100.0, 2.0).unwrap()), [833.3, 1024.0, 1024.0]);
        assert!(switch_distances(f32::NAN, 2.0).is_none());
        assert!(switch_distances(1.0, 0.0).is_none());
    }
}
