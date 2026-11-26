//! What has a skeleton of its own in the 3D view: an animesh, worn or not, and
//! the avatar's own body (its eyes above all: a mesh head often leaves them to
//! the avatar).

use super::Group;
use super::build::{Driver, Hold, hold_from};
use crate::scene::Scene;
use crate::scene::anim::SkeletonBase;
use crate::scene::avatar::{AvatarLibrary, IMG_DEFAULT_AVATAR, IMG_INVISIBLE};
use crate::world::World;
use aurora_render::preview::Vertex;
use glam::{Mat4, Quat};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use uuid::Uuid;

/// The skeleton of an animesh, as it is taken.
pub(super) struct OwnInput {
    pub root: Uuid,
    pub group: usize,
    pub hold: Hold,
    pub scale: f32,
    pub base: Option<Arc<SkeletonBase>>,
    /// As it was last drawn.
    pub joints: Option<Vec<Mat4>>,
}

impl OwnInput {
    /// None for a linkset without one.
    pub fn of(world: &World, scene: &Scene, group: &Group, at: usize, worn: bool) -> Option<OwnInput> {
        let index = world.objects.index_of_uuid(&group.root)?;
        let object = world.objects.get(index).filter(|o| !o.is_avatar())?;
        Scene::skeleton_owner(world, index).filter(|(_, owner)| *owner == index)?;
        // LLControlAvatar::matchVolumeTransform: on the ground it also follows
        // its root mesh's bind rotation
        let turned = (!worn)
            .then(|| {
                let sculpt = object.visual_volume().sculpt?;
                let skin = scene.meshes.view(&sculpt.texture)?.skin.clone()?;
                crate::scene::anchors::unscaled_bind_rotation(skin.bind_shape)
            })
            .flatten()
            .unwrap_or(Quat::IDENTITY);
        Some(OwnInput {
            root: group.root,
            group: at,
            hold: hold_from(world, index, worn, Mat4::from_quat(turned))?,
            scale: world.animesh_fixups.get(&group.root).map_or(1.0, |f| f.scale),
            base: scene.skeleton_base(&group.root),
            joints: super::live_joints(world, &group.root),
        })
    }

    /// (not its pose: it moves without another model)
    pub fn hash(&self, hasher: &mut impl Hasher) {
        self.base.as_ref().map(|b| Arc::as_ptr(b) as usize).hash(hasher);
        self.joints.is_some().hash(hasher);
        self.scale.to_bits().hash(hasher);
    }
}

/// The skeleton of an animesh in the model.
pub(super) struct Own {
    pub root: Uuid,
    pub group: usize,
    /// What holds its object.
    pub place: Driver,
    /// From its joints to its object.
    pub after: Mat4,
    /// Its joints when they are not known now.
    pub still: Vec<Mat4>,
}

/// The parts of an avatar's own body that show, with their bake (None: not
/// known, drawn plain), as `Scene::sync_avatar` hides them.
pub fn body_parts(world: &World, scene: &Scene, avatar: usize) -> Vec<(usize, Option<Uuid>)> {
    let Some(object) = world.objects.get(avatar).filter(|o| o.is_avatar()) else {
        return Vec::new();
    };
    let own = object.full_id == world.agent_id;
    // the bakes its worn meshes show in their place
    let worn = scene.bom_mask(world, avatar);
    world
        .avatar_lib
        .parts
        .iter()
        .enumerate()
        .filter(|(_, part)| world.rlva_body_part_visible(own, part.name))
        .filter(|(_, part)| worn & (1u64 << part.bake_te.min(63)) == 0)
        .filter_map(|(i, part)| {
            let bake = world.bake_texture(avatar, part.bake_te).filter(|id| !id.is_nil());
            let shown = match bake {
                Some(IMG_INVISIBLE) => false,
                Some(IMG_DEFAULT_AVATAR) | None => part.name != "skirt",
                Some(_) => true,
            };
            shown.then(|| (i, bake.filter(|id| *id != IMG_DEFAULT_AVATAR)))
        })
        .collect()
}

/// A face of `body_faces`.
type Geometry = (Arc<[Vertex]>, Arc<[u32]>);

/// The geometry of some parts of the avatar's own body, in the shape `values`.
pub(super) fn body_faces(lib: &AvatarLibrary, parts: &[usize], values: &[u8]) -> Vec<Geometry> {
    let mut weights = lib.shape_params.effective_weights(values);
    // (the hands as they are most of the time, as Scene::sync_avatar)
    weights.insert(crate::scene::default_motions::RELAXED_HANDS, 1.0);
    parts
        .iter()
        .filter_map(|i| lib.parts.get(*i))
        .map(|part| {
            let morphs: Vec<f32> = part.morphs.iter().map(|m| weights.get(&m.param).copied().unwrap_or(0.0)).collect();
            let (geometry, _, _) = crate::scene::morph::morphed(part, &morphs);
            let vertices = geometry
                .vertices
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let skin = part.skin.get(i).copied().unwrap_or_default();
                    Vertex {
                        position: v.pos,
                        normal: [v.normal[0], v.normal[1], v.normal[2]].map(|n| f32::from(n) / 127.0),
                        uv: v.uv,
                        joints: skin.joints.map(u32::from),
                        weights: skin.weights.map(|w| f32::from(w) / f32::from(u16::MAX)),
                    }
                })
                .collect();
            (vertices, part.indices.iter().map(|i| u32::from(*i)).collect())
        })
        .collect()
}
