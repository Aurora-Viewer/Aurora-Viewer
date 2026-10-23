//! Explicit transfer of the worn shape, authorized against its inventory item.
//! Modify checks follow LLPanelEditWearable; body deformation follows
//! LLPolyMorphTarget / LLPolySkeletalDistortion (indra/llappearance, LGPL 2.1).
use crate::{scene::avatar::AvatarLibrary, world::World};
use aurora_assets::wearable::Wearable;
use aurora_net::inventory::InvItem;
use glam::{Mat4, Vec3};
use serde_json::{Value, json};
use std::{
    sync::{Arc, mpsc},
    time::Instant,
};
use uuid::Uuid;

pub const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    Create,
    Apply,
}

impl Action {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Apply => "apply",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub item: Uuid,
    pub asset: Uuid,
}

impl Key {
    pub fn of(item: &InvItem) -> Self {
        Self {
            item: item.id,
            asset: item.asset_id,
        }
    }
    pub fn valid(self, world: &World) -> bool {
        editable(world).is_ok_and(|item| Self::of(&item) == self)
    }
}

/// Resolve the actual worn item, never a link's permissive masks or another
/// avatar's public appearance values. Unknown ownership/rights fail closed.
pub fn editable(world: &World) -> Result<InvItem, String> {
    let item = crate::shape_editor::worn_shape(world)
        .and_then(|id| world.inventory.items.get(&id))
        .filter(|item| !item.asset_id.is_nil())
        .ok_or_else(|| aurora_i18n::tr!("blender-shape-missing").to_owned())?;
    if world.agent_id.is_nil() || item.owner != world.agent_id || item.group_owned || item.owner_mask & aurora_net::build::perm::MODIFY == 0
    {
        return Err(aurora_i18n::tr!("blender-shape-no-modify").into());
    }
    Ok(item.clone())
}

pub struct Pending {
    pub key: Key,
    pub id: Uuid,
    pub result: mpsc::Receiver<Result<Arc<Value>, String>>,
}

pub struct Outgoing {
    pub id: Uuid,
    pub data: Arc<Value>,
}

#[derive(Default)]
pub struct State {
    pub(crate) demo_sent: bool,
    pub requested: Option<Action>,
    pub action: Action,
    pub blocked: Option<String>,
    pub apply_blocked: Option<String>,
    pub message: String,
    pub error: bool,
    pub loading: Option<Key>,
    pub pending: Option<Pending>,
    pub active: Option<(Key, Uuid)>,
    pub started: Option<Instant>,
}

impl State {
    pub fn busy(&self) -> bool {
        self.started.is_some()
    }
    pub fn fail(&mut self, message: String) {
        self.loading = None;
        self.pending = None;
        self.active = None;
        self.started = None;
        self.message = message;
        self.error = true;
    }
}

/// Build only from the authorized shape asset. No skin textures, baked meshes,
/// attachments, other wearables or live animation data enter this payload.
pub fn build(library: &AvatarLibrary, bytes: &[u8], id: Uuid, name: &str) -> Result<Arc<Value>, String> {
    let wearable = Wearable::parse(bytes).map_err(|_| aurora_i18n::tr!("blender-shape-invalid").to_owned())?;
    if wearable.kind != 0 {
        return Err(aurora_i18n::tr!("blender-shape-invalid").into());
    }
    // Include the sex driver, which is not an ordinary editor slider. Only
    // parameters belonging to the shape wearable may enter the reference.
    let defaults = crate::world::inventory::wearable::create(0, "", Uuid::nil())?;
    let defaults = Wearable::parse(&defaults).map_err(str::to_owned)?;
    let params: std::collections::BTreeMap<_, _> = defaults
        .params
        .into_iter()
        .filter_map(|(id, default)| {
            let (min, max, _) = library.shape_params.range(id)?;
            Some((id, wearable.params.get(&id).copied().unwrap_or(default).clamp(min, max)))
        })
        .collect();
    let rig = &library.rig;
    let skeleton = library
        .skeleton
        .as_ref()
        .ok_or_else(|| aurora_i18n::tr!("blender-shape-invalid").to_owned())?;
    let weights = library.shape_params.weights_with(&[], &params);
    let shape = library.shape_params.shape_weights(weights.clone(), rig);
    let world = rig.world_with(&rig.local_rot, Vec3::ZERO, &shape.local_pos, &shape.scale);
    if world.iter().any(|matrix| !matrix.is_finite() || matrix.determinant().abs() < 1e-8) {
        return Err(aurora_i18n::tr!("blender-shape-invalid").into());
    }
    let skin: Vec<_> = world
        .iter()
        .zip(&rig.default_world_inv)
        .map(|(world, inverse)| *world * *inverse)
        .collect();
    let names: Vec<_> = skeleton.joints.iter().take(rig.len()).map(|joint| joint.name.clone()).collect();
    let mut parts = Vec::new();
    for part in library
        .parts
        .iter()
        .filter(|part| matches!(part.name, "head" | "upper" | "lower" | "eye_left" | "eye_right" | "eyelashes"))
    {
        let morphs: Vec<_> = part
            .morphs
            .iter()
            .map(|morph| weights.get(&morph.param).copied().unwrap_or(0.0))
            .collect();
        let (geometry, _, _) = crate::scene::morph::morphed(part, &morphs);
        let mut positions = Vec::with_capacity(geometry.vertices.len());
        let mut influences = Vec::with_capacity(geometry.vertices.len());
        for (vertex, influence) in geometry.vertices.iter().zip(&part.skin) {
            let point = Vec3::from_array(vertex.pos);
            let total = influence.weights.iter().map(|w| *w as f32).sum::<f32>().max(1.0);
            let mut position = Vec3::ZERO;
            let mut groups = Vec::new();
            for (&joint, &weight) in influence.joints.iter().zip(&influence.weights) {
                if weight == 0 {
                    continue;
                }
                let weight = weight as f32 / total;
                position += skin.get(joint as usize).copied().unwrap_or(Mat4::IDENTITY).transform_point3(point) * weight;
                groups.push((if (joint as usize) < rig.len() { joint as usize } else { rig.pelvis }, weight));
            }
            positions.push(position.to_array());
            influences.push(groups);
        }
        parts.push(json!({"name": part.name, "positions": positions,
            "uvs": geometry.vertices.iter().map(|v| v.uv).collect::<Vec<_>>(),
            "triangles": part.indices.as_chunks::<3>().0, "weights": influences}));
    }
    let joints: Vec<_> = skeleton
        .joints
        .iter()
        .zip(&world)
        .zip(&rig.default_world_inv)
        .map(|((joint, world), reference)| {
            let (_, rotation, head) = world.to_scale_rotation_translation();
            json!({"name": joint.name, "head": head.to_array(),
            "tail": world.transform_point3(joint.end).to_array(), "rotation": rotation.to_array(),
            "inverse_bind": world.inverse().to_cols_array(), "reference_bind": reference.to_cols_array()})
        })
        .collect();
    let data = json!({"id": id, "name": name.chars().take(128).collect::<String>(), "params": params,
        "joints": joints, "names": names, "parts": parts});
    if serde_json::to_vec(&data).map_err(|e| e.to_string())?.len() > MAX_BYTES {
        return Err(aurora_i18n::tr!("blender-shape-too-large").into());
    }
    Ok(Arc::new(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blender_live_shape_requires_the_resolved_worn_items_modify_right() {
        let (mut world, _) = crate::shape_editor::tests::fixture();
        let id = crate::shape_editor::worn_shape(&world).expect("shape");
        world.inventory.items.get_mut(&id).expect("shape").asset_id = Uuid::from_u128(1000);
        let key = Key::of(&editable(&world).expect("owned editable shape"));
        // Copy/transfer and creator status do not confer Modify rights.
        world.inventory.items.get_mut(&id).expect("shape").owner_mask = 0x88000;
        assert!(editable(&world).is_err());
        assert!(!key.valid(&world));
        let item = world.inventory.items.get_mut(&id).expect("shape");
        item.owner_mask = 0x4000;
        item.creator = Uuid::from_u128(999);
        assert!(key.valid(&world), "Modify is sufficient, independent of Copy or creator");
        world.inventory.items.get_mut(&id).expect("shape").owner = Uuid::nil();
        assert!(!key.valid(&world), "unknown owner is not authorized");
        let item = world.inventory.items.get_mut(&id).expect("shape");
        item.owner = world.agent_id;
        item.group_owned = true;
        assert!(!key.valid(&world));
        let item = world.inventory.items.get_mut(&id).expect("shape");
        item.group_owned = false;
        item.asset_id = Uuid::from_u128(1001);
        assert!(!key.valid(&world), "replaced shape asset invalidates pending work");
        world.inventory.items.remove(&id);
        assert!(
            editable(&world).is_err(),
            "a permissive COF link cannot authorize an unresolved target"
        );
    }

    #[test]
    fn blender_live_shape_payload_keeps_sex_morphs_and_shaped_bind_matrices() {
        let lib = AvatarLibrary::load();
        let data = crate::world::inventory::wearable::create(0, "Test Shape", Uuid::nil()).expect("asset");
        let mut wearable = Wearable::parse(&data).expect("shape");
        wearable.params.insert(80, 1.0);
        wearable.params.insert(33, 1.5);
        wearable.params.insert(105, 0.8);
        let data = wearable.encode("Test Shape").expect("asset");
        let payload = build(&lib, &data, Uuid::from_u128(1234), "Test Shape").expect("payload");
        assert_eq!(payload["params"]["80"], 1.0);
        assert_eq!(payload["joints"].as_array().expect("joints").len(), 159);
        assert_eq!(payload["parts"].as_array().expect("body").len(), 6);
        assert!(serde_json::to_vec(&payload).expect("JSON").len() < MAX_BYTES);
        let params = wearable.params;
        let shape = lib
            .shape_params
            .shape_weights(lib.shape_params.weights_with(&[], &params), &lib.rig);
        let world = lib.rig.world_with(&lib.rig.local_rot, Vec3::ZERO, &shape.local_pos, &shape.scale);
        for (joint, matrix) in payload["joints"].as_array().expect("joints").iter().zip(world) {
            let inverse: [f32; 16] = serde_json::from_value(joint["inverse_bind"].clone()).expect("inverse");
            assert!((matrix * Mat4::from_cols_array(&inverse)).abs_diff_eq(Mat4::IDENTITY, 1e-5));
        }
        // Optional fixture for native Blender interoperability tests; synthetic
        // data only, never an account's shape or inventory metadata.
        if let Some(path) = std::env::var_os("AURORA_TEST_SHAPE_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&payload).expect("JSON")).expect("fixture");
        }
        assert!(build(&lib, b"invalid asset", Uuid::nil(), "Test").is_err());
        let other = crate::world::inventory::wearable::create(1, "Skin", Uuid::nil()).expect("skin");
        assert!(build(&lib, &other, Uuid::nil(), "Skin").is_err());
    }
}
