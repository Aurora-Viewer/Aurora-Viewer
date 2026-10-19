//! Linden trees and grass in the scene (LLVOTree, LLVOGrass): which
//! geometry they get, how it is placed and textured, and the grass that
//! follows the terrain under it.
//!
//! A tree's mesh (`aurora_prim::tree`) only depends on its species and
//! detail level, so every tree of a species shares it; the object places it
//! with a uniform scale. A grass tuft (`aurora_prim::grass`) stands on the
//! terrain under each blade: its mesh is its own, built here at once (a few
//! hundred vertices), and written again in place when terrain patches
//! arrive for its region (LLVOGrass::idleUpdate rebuilds when its surface
//! patch changes).
//!
//! Both are drawn with their species texture whatever their texture entry
//! says, alpha-tested at 0.5 (LLDrawPoolTree::beginDeferredPass and the
//! grass pass, setMinimumAlpha(0.5)), cast shadows, and are picked through
//! their cut-out like any masked face (the Legacy Viewer picks trees with a
//! tetrahedron around them, LLVOTree::lineSegmentIntersect).

use std::hash::{Hash, Hasher};

use aurora_prim::params::{LL_PCODE_LEGACY_GRASS, LL_PCODE_LEGACY_TREE};
use aurora_prim::te::TextureFace;
use aurora_prim::{grass, tree};
use aurora_render::Renderer;
use glam::{Mat4, Quat, Vec3};

use super::{GeomKey, GeomState, Scene, jobs};
use crate::world::World;
use crate::world::objects::Object;
use aurora_net::RegionHandle;

/// The key of a grass tuft's geometry: everything its blades depend on but
/// the ground (species, X and Y size, position).
pub(super) fn grass_hash(o: &Object) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    o.state.hash(&mut h);
    let scale = o.visual_scale();
    let pos = o.position;
    for v in [scale.x, scale.y, pos.x, pos.y, pos.z] {
        v.to_bits().hash(&mut h);
    }
    h.finish()
}

/// Model matrix of a plant's geometry at the object's place, None for
/// other geometries. A tree is turned a quarter around its axis and planted
/// 0.1 m into the ground (LLVOTree::updateMesh, "plants tree
/// underground"); a grass tuft ignores the object's rotation and size (its
/// mesh holds them).
pub(super) fn plant_model(key: GeomKey, o: &Object, pos: Vec3, rot: Quat) -> Option<Mat4> {
    match key {
        GeomKey::Tree { .. } => Some(Mat4::from_scale_rotation_translation(
            Vec3::splat(tree::tree_model_scale(o.visual_scale()).max(0.001)),
            rot * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            pos - Vec3::Z * 0.1,
        )),
        GeomKey::Grass { .. } => Some(Mat4::from_translation(pos)),
        _ => None,
    }
}

/// The model matrix object `o` draws `key` with: a plant's, else its scale,
/// rotation and position.
pub(super) fn geometry_model(scale: Vec3, o: &Object, key: Option<GeomKey>, pos: Vec3, rot: Quat) -> Mat4 {
    key.and_then(|k| plant_model(k, o, pos, rot))
        .unwrap_or_else(|| Mat4::from_scale_rotation_translation(scale, rot, pos))
}

/// The size a tree's detail level is judged on: half the side of the box
/// the Legacy Viewer gives it (LLVOTree::updateSpatialExtents). None for
/// what is not a tree.
pub(super) fn tree_lod_radius(o: &Object) -> Option<f32> {
    (o.pcode == LL_PCODE_LEGACY_TREE)
        .then(|| tree::tree_extent(tree::tree_species(o.tree_species)) * tree::tree_model_scale(o.visual_scale()))
}

/// The face a plant is drawn with: its species texture, white, no
/// material (LLVOTree's mTreeImagep, LLVOGrass::updateSpecies). None for
/// other objects.
pub(super) fn plant_face(o: &Object) -> Option<TextureFace> {
    let texture = match o.pcode {
        LL_PCODE_LEGACY_TREE => tree::tree_species(o.tree_species).texture,
        LL_PCODE_LEGACY_GRASS => grass::grass_species(o.state).texture,
        _ => return None,
    };
    Some(TextureFace {
        texture,
        ..Default::default()
    })
}

/// The tree detail level of a scene level (3 the finest here, 0 there).
pub(super) fn tree_lod(lod: u8) -> usize {
    3 - lod.min(3) as usize
}

/// A grass tuft's geometry and the terrain it was placed on.
pub(super) struct GrassBuilt {
    key: GeomKey,
    region: RegionHandle,
    revision: u64,
}

impl Scene {
    /// The box a selected plant is outlined with (build/silhouette.rs):
    /// around what is drawn, not its object scale (a tree's is no size of
    /// it). Centre, rotation and half size; None until its geometry is in.
    pub(crate) fn plant_box(&self, world: &World, idx: usize, pos: Vec3, rot: Quat) -> Option<(Vec3, Quat, Vec3)> {
        let o = world.objects.get(idx)?;
        let key = self.gpu.get(idx)?.geom?;
        let model = plant_model(key, o, pos, rot)?;
        let geom = self.geom_ready(&key)?;
        let (scale, rotation, _) = model.to_scale_rotation_translation();
        let center = model.transform_point3((geom.min + geom.max) * 0.5);
        Some((center, rotation, (geom.max - geom.min) * 0.5 * scale))
    }

    /// Build grass geometry `key` for object `idx` at render position
    /// `pos`, unless it is already there.
    pub(super) fn request_grass(&mut self, renderer: &mut Renderer, world: &World, idx: usize, key: GeomKey, pos: Vec3) {
        let Some(o) = world.objects.get(idx) else {
            return;
        };
        let revision = if self.geoms.contains_key(&key) {
            if self.grass.get(&idx).is_some_and(|b| b.key == key) {
                return;
            }
            // built before for this object: placed on an unknown ground,
            // brought up to date by the next `sync_grass_ground`
            0
        } else {
            self.upload_geom(renderer, key, grass_geometry(world, o, pos));
            world.regions.get(&o.key.region).map_or(0, |r| r.terrain_revision)
        };
        self.grass.insert(
            idx,
            GrassBuilt {
                key,
                region: o.key.region,
                revision,
            },
        );
    }

    /// The grass whose region received terrain since it was placed stands
    /// on the new ground: its blades are written again in place (same
    /// vertex count), its bounds follow. `synced`: objects whose tables
    /// must be written again.
    pub(super) fn sync_grass_ground(&mut self, renderer: &mut Renderer, world: &World, now: std::time::Instant, synced: &mut Vec<usize>) {
        let geoms = &self.geoms;
        let gpu = &self.gpu;
        self.grass.retain(|&idx, b| {
            matches!(b.key, GeomKey::Grass { id, .. } if world.objects.get(idx).is_some_and(|o| o.full_id == id))
                && gpu.get(idx).is_some_and(|g| g.geom == Some(b.key))
                && geoms.contains_key(&b.key)
        });
        let stale: Vec<usize> = self
            .grass
            .iter()
            .filter(|(_, b)| world.regions.get(&b.region).is_some_and(|r| r.terrain_revision != b.revision))
            .map(|(&idx, _)| idx)
            .collect();
        for idx in stale {
            let (Some(o), Some((pos, _, _))) = (world.objects.get(idx), Self::object_transform(world, idx, now, 0)) else {
                continue;
            };
            let Some(b) = self.grass.get_mut(&idx) else {
                continue;
            };
            b.revision = world.regions.get(&b.region).map_or(0, |r| r.terrain_revision);
            let key = b.key;
            let prepared = grass_geometry(world, o, pos);
            let Some(super::GeomEntry {
                state: GeomState::Ready(geom),
                ..
            }) = self.geoms.get_mut(&key)
            else {
                continue;
            };
            let Some(geom) = std::sync::Arc::get_mut(geom) else {
                continue;
            };
            for (alloc, face) in geom.faces.iter().zip(prepared.faces) {
                if let (Some(alloc), Some(jobs::FaceUpload::Memory(face))) = (alloc, face) {
                    renderer.update_mesh_vertices(*alloc, &face.vertices);
                }
            }
            geom.min = prepared.min;
            geom.max = prepared.max;
            geom.pick_faces = prepared.pick_faces;
            let g = &mut self.gpu[idx];
            g.center = pos + (geom.min + geom.max) * 0.5;
            g.radius = ((geom.max - geom.min) * 0.5).length().max(0.05);
            self.pick.changed(idx);
            synced.push(idx);
        }
    }
}

/// A grass tuft's blades on the ground under them, relative to `pos`.
fn grass_geometry(world: &World, o: &Object, pos: Vec3) -> jobs::PreparedGeom {
    let species = grass::grass_species(o.state);
    let scale = o.visual_scale();
    let bases = grass::grass_bases(species, scale);
    let ground: [f32; grass::GRASS_BLADES * 2] = std::array::from_fn(|i| {
        let [x, y] = bases[i];
        world.ground_height(pos + Vec3::new(x, y, 0.0)).map_or(0.0, |h| h - pos.z)
    });
    let mesh = grass::generate_grass(species, scale, &ground);
    let (faces, min, max) = jobs::volume_to_faces(&mesh);
    jobs::PreparedGeom::new(faces, min, max, true, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_is_turned_a_quarter_and_planted() {
        let mut o = crate::demo::action_avatar(false);
        o.pcode = LL_PCODE_LEGACY_TREE;
        o.tree_species = Some(1);
        o.scale = Vec3::new(4.0, 4.0, 4.0);
        let o = Object::from_update(1, o);
        let key = Scene::object_geom_key(&o, 3).expect("a tree has geometry");
        assert_eq!(key, GeomKey::Tree { species: 1, lod: 3 });
        let m = plant_model(key, &o, Vec3::new(10.0, 20.0, 30.0), Quat::IDENTITY).unwrap();
        // the tree's +X goes to the object's +Y, its base 0.1 m down
        let s = 4.0 * 3f32.sqrt() * 0.05;
        assert!((m.transform_vector3(Vec3::X) - Vec3::Y * s).length() < 1e-5);
        assert!((m.transform_point3(Vec3::ZERO) - Vec3::new(10.0, 20.0, 29.9)).length() < 1e-5);
        assert_eq!(plant_face(&o).map(|f| f.texture), Some(tree::TREE_SPECIES[1].texture));
        // an unknown species is drawn as the first
        let mut u = crate::demo::action_avatar(false);
        u.pcode = LL_PCODE_LEGACY_TREE;
        u.tree_species = Some(77);
        assert_eq!(
            Scene::object_geom_key(&Object::from_update(2, u), 0),
            Some(GeomKey::Tree { species: 0, lod: 0 })
        );
    }

    #[test]
    fn grass_keys_follow_its_species_size_and_place() {
        let mut g = crate::demo::action_avatar(false);
        g.pcode = LL_PCODE_LEGACY_GRASS;
        g.state = 4;
        g.scale = Vec3::new(3.0, 3.0, 1.0);
        let a = Object::from_update(1, g.clone());
        let key = Scene::object_geom_key(&a, 2);
        assert!(matches!(key, Some(GeomKey::Grass { id, .. }) if id == a.full_id));
        // the detail level does not change it, a move does
        assert_eq!(key, Scene::object_geom_key(&a, 0));
        g.position.x += 1.0;
        assert_ne!(key, Scene::object_geom_key(&Object::from_update(1, g.clone()), 2));
        g.state = 1;
        assert_eq!(
            plant_face(&Object::from_update(1, g)).map(|f| f.texture),
            Some(grass::GRASS_SPECIES[1].texture)
        );
    }

    #[test]
    fn new_trees_are_not_drawn() {
        let mut t = crate::demo::action_avatar(false);
        t.pcode = aurora_prim::params::LL_PCODE_TREE_NEW;
        assert_eq!(Scene::object_geom_key(&Object::from_update(1, t), 3), None);
    }

    #[test]
    fn tree_levels_run_the_other_way() {
        assert_eq!([tree_lod(3), tree_lod(2), tree_lod(1), tree_lod(0)], [0, 1, 2, 3]);
    }
}
