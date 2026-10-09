//! Materials and media of the selected faces: GLTF materials and their
//! overrides (ModifyMaterialParams, LLGLTFMaterialList::queueApply /
//! queueModify), Blinn-Phong normal / specular maps (RenderMaterials PUT,
//! LLMaterialMgr::put / remove) and media on a prim (ObjectMedia UPDATE,
//! LLVOVolume::sendMediaDataUpdate). Originally LGPL 2.1 sources.

use super::BuildTool;
use super::edits::CapPurpose;
use crate::media::entry::{MediaEntry, ObjectMediaData};
use crate::scene::legacy_mat::{self, LegacyMaterial};
use crate::world::World;
use aurora_assets::material::PbrOverride;
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::RegionHandle;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// LLGLTFMaterialList MAX_TASK_UPDATES.
const MAX_TASK_UPDATES: usize = 255;
/// MF_HAS_MEDIA bit of the TE media byte.
pub const MF_HAS_MEDIA: u8 = 0x01;

/// One ModifyMaterialParams entry.
#[derive(Debug, Clone)]
pub struct GltfUpdate {
    pub object: usize,
    pub face: u8,
    /// Apply this material asset (null: remove the material).
    pub asset: Option<Uuid>,
    /// Override, `None` = clear the overrides ("").
    pub over: Option<PbrOverride>,
}

impl BuildTool {
    /// POST ModifyMaterialParams and echo the change locally (render
    /// material ids of the ExtraParams, overrides).
    pub fn gltf_update(&mut self, world: &mut World, updates: Vec<GltfUpdate>) {
        let mut by_region: HashMap<RegionHandle, Vec<Llsd>> = HashMap::new();
        for u in updates {
            let Some(o) = world.objects.get_mut(u.object) else { continue };
            let json = u.over.as_ref().map(|o| o.to_gltf_json()).unwrap_or_default();
            let mut e = llsd_map! {
                "object_id" => o.full_id,
                "side" => u.face as i32,
                "gltf_json" => json,
            };
            if let Some(a) = u.asset {
                e.insert("asset_id", a);
                // LLViewerObject::setRenderMaterialID: the local params first
                o.extra.render_materials.retain(|(f, _)| *f != u.face);
                if !a.is_nil() {
                    o.extra.render_materials.push((u.face, a));
                }
            }
            o.material_dirty = true;
            o.render.needs_records = true;
            let key = o.key;
            let mut list: Vec<(u8, PbrOverride)> = world.gltf_overrides.get(&key).map(|v| v.to_vec()).unwrap_or_default();
            list.retain(|(f, _)| *f != u.face);
            if let Some(ov) = u.over {
                list.push((u.face, ov));
            }
            world.gltf_overrides.insert(key, Arc::from(list));
            by_region.entry(key.region).or_default().push(e);
        }
        for (handle, list) in by_region {
            for chunk in list.chunks(MAX_TASK_UPDATES) {
                self.cap(
                    handle,
                    "ModifyMaterialParams",
                    false,
                    Llsd::Array(chunk.to_vec()),
                    CapPurpose::GltfMaterials,
                );
            }
        }
    }

    /// The override of one face (default: none).
    pub fn gltf_override(world: &World, idx: usize, face: u8) -> PbrOverride {
        world
            .objects
            .get(idx)
            .and_then(|o| world.gltf_overrides.get(&o.key))
            .and_then(|l| l.iter().find(|(f, _)| *f == face).map(|(_, o)| o.clone()))
            .unwrap_or_default()
    }

    /// The GLTF material asset of one face (None: Blinn-Phong).
    pub fn gltf_asset(world: &World, idx: usize, face: u8) -> Option<Uuid> {
        world
            .objects
            .get(idx)?
            .extra
            .render_materials
            .iter()
            .find(|(f, _)| *f == face || *f == u8::MAX)
            .map(|(_, id)| *id)
            .filter(|id| !id.is_nil())
    }

    /// RenderMaterials PUT of the Blinn-Phong maps of some faces
    /// (`None`: no material). The simulator answers with new material ids
    /// in the texture entries.
    pub fn legacy_put(&mut self, world: &World, faces: Vec<(usize, u8, Option<LegacyMaterial>)>) {
        let mut by_region: HashMap<RegionHandle, Vec<(u8, u32, Option<LegacyMaterial>)>> = HashMap::new();
        for (idx, face, m) in faces {
            let Some(o) = world.objects.get(idx) else { continue };
            by_region.entry(o.key.region).or_default().push((face, o.key.local_id, m));
        }
        for (handle, list) in by_region {
            // getMaxMaterialsPerTransaction default
            for chunk in list.chunks(50) {
                self.cap(
                    handle,
                    "RenderMaterials",
                    true,
                    legacy_mat::put_body(chunk),
                    CapPurpose::LegacyMaterials,
                );
            }
        }
    }

    /// ObjectMedia GET for an object of the selection.
    pub fn media_get(&mut self, world: &World, idx: usize) {
        let Some(o) = world.objects.get(idx) else { return };
        let body = llsd_map! { "verb" => "GET", "object_id" => o.full_id };
        let (handle, id) = (o.key.region, o.full_id);
        self.cap(handle, "ObjectMedia", false, body, CapPurpose::MediaGet(id));
    }

    /// ObjectMedia UPDATE with every face's entry (LLVOVolume::sendMediaDataUpdate).
    pub fn media_update(&mut self, world: &World, idx: usize, faces: &[Option<MediaEntry>]) {
        let Some(o) = world.objects.get(idx) else { return };
        let data: Vec<Llsd> = faces
            .iter()
            .map(|f| match f {
                Some(e) => e.to_llsd(),
                None => Llsd::Undef,
            })
            .collect();
        let body = llsd_map! {
            "verb" => "UPDATE",
            "object_id" => o.full_id,
            "object_media_data" => Llsd::Array(data),
        };
        let handle = o.key.region;
        self.cap(handle, "ObjectMedia", false, body, CapPurpose::MediaUpdate);
    }

    /// Answers of the requests of this module.
    pub(super) fn on_media_reply(&mut self, _world: &mut World, purpose: CapPurpose, result: Result<Llsd, String>) {
        match (purpose, result) {
            (CapPurpose::MediaGet(id), Ok(v)) => {
                if let Ok(d) = crate::media::entry::parse_get_response(&v, id) {
                    self.ui.media_data.insert(id, d);
                }
            }
            (CapPurpose::GltfMaterials, Ok(v)) if v.has("success") && !v.get("success").as_bool() => {
                self.status = format!("Matériau refusé : {}", v.get("message").to_string_value());
            }
            (CapPurpose::MediaUpdate | CapPurpose::GltfMaterials | CapPurpose::LegacyMaterials, Err(e)) => {
                self.status = format!("Échec de l'envoi : {e}");
            }
            _ => {}
        }
    }

    /// Media data of an object: ours (just sent / fetched for the panel)
    /// or the media manager's.
    pub fn object_media<'a>(&'a self, media: &'a crate::media::MediaManager, id: &Uuid) -> Option<&'a ObjectMediaData> {
        self.ui.media_data.get(id).or_else(|| media.object_media(id))
    }
}
