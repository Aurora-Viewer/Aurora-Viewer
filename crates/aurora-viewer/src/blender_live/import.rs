//! Request/freeze Blender LODs; never send a quote or an upload command.
//! Material ordering follows LLModelPreview::matchMaterialOrder
//! (indra/newview/llmodelpreview.cpp); missing surfaces become NoGeometry.
use super::{Live, protocol};
use aurora_assets::import_mesh::{Face, Scene};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct Request {
    pub id: Uuid,
    pub source: Uuid,
}

pub struct Bundle {
    pub high: protocol::PreparedSource,
    pub images: Vec<protocol::PreparedImage>,
    pub lods: [Arc<Scene>; 3],
}

#[derive(Default)]
pub struct State {
    active: Option<(Request, Instant)>,
    pub ready: Option<Bundle>,
    pub message: String,
    pub error: bool,
    pub demo_requested: bool,
}
impl State {
    pub fn busy(&self) -> bool {
        self.active.is_some() || self.ready.is_some()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Level {
    mesh: protocol::Mesh,
    slots: Vec<usize>,
    origin: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    id: Uuid,
    source: Uuid,
    #[serde(default)]
    levels: Vec<Level>,
    #[serde(default)]
    images: Vec<protocol::Image>,
    error: Option<String>,
}

fn aligned_scene(low: &Scene, high: &Scene, slots: &[usize]) -> Result<Arc<Scene>, String> {
    let mut faces: Vec<VecDeque<_>> = vec![VecDeque::new(); high.materials.len()];
    for model in &low.models {
        for face in &model.faces {
            let material = *slots.get(face.material).ok_or("Invalid LOD material mapping")?;
            let target = faces.get_mut(material).ok_or("Invalid High material slot")?;
            target.push_back((face.clone(), model.center, model.size));
        }
    }
    let mut scene = high.clone();
    for model in &mut scene.models {
        for face in &mut model.faces {
            let material = face.material;
            if let Some((mut lower, center, size)) = faces[material].pop_front() {
                for vertex in &mut lower.vertices {
                    vertex.position = ((Vec3::from_array(vertex.position) * size + center - model.center) / model.size).to_array();
                    vertex.normal = (Vec3::from_array(vertex.normal) / size * model.size).normalize_or_zero().to_array();
                }
                lower.material = material;
                *face = lower;
            } else {
                *face = Face {
                    material,
                    vertices: Vec::new(),
                    indices: Vec::new(),
                };
            }
        }
    }
    if faces.iter().any(|faces| !faces.is_empty()) {
        return Err("LOD material chunks exceed High".into());
    }
    Ok(Arc::new(scene))
}

pub fn decode(bytes: &[u8], expected: Request, supported: &HashSet<String>) -> Result<Bundle, String> {
    if bytes.is_empty() || bytes.len() > protocol::MAX_BYTES {
        return Err("Invalid Blender LOD payload size".into());
    }
    let reply: Reply = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if reply.id != expected.id || reply.source != expected.source {
        return Err("Stale Blender import response".into());
    }
    if let Some(error) = reply.error {
        return Err(error.chars().take(512).collect());
    }
    if reply.levels.len() != 4 {
        return Err("Expected four Blender LODs".into());
    }
    let corners: usize = reply
        .levels
        .iter()
        .flat_map(|level| &level.mesh.faces)
        .map(|face| face.vertices.len())
        .sum();
    if corners > protocol::MAX_CORNERS {
        return Err("Prepared LODs exceed 600,000 mesh corners".into());
    }
    let high = &reply.levels[3];
    for level in &reply.levels {
        if level.mesh.source_id != Some(expected.source)
            || level.mesh.name != high.mesh.name
            || level.mesh.joints != high.mesh.joints
            || level.mesh.inverse_bind.len() != high.mesh.inverse_bind.len()
            || level
                .mesh
                .inverse_bind
                .iter()
                .flatten()
                .zip(high.mesh.inverse_bind.iter().flatten())
                .any(|(a, b)| (a - b).abs() > 0.0001)
            || level.slots.len() != level.mesh.faces.len()
            || level.origin.len() > 256
            || level.slots.iter().any(|slot| *slot >= high.mesh.faces.len())
            || level.slots.iter().copied().collect::<HashSet<_>>().len() != level.slots.len()
        {
            return Err("LOD skeleton or material mapping disagrees with High".into());
        }
    }
    if high.slots != (0..high.mesh.faces.len()).collect::<Vec<_>>() {
        return Err("Invalid High material mapping".into());
    }
    let mut prepared = Vec::new();
    let mappings: Vec<_> = reply.levels.iter().map(|level| level.slots.clone()).collect();
    for level in reply.levels {
        // Reuse the strict geometry/weight/PBR decoder on the socket worker.
        // Imported scenes do not enter the live render/revision mailbox.
        let snapshot = protocol::Snapshot {
            protocol: 1,
            revision: 1,
            meshes: vec![level.mesh],
            images: reply.images.clone(),
        };
        prepared.push(protocol::prepare_snapshot(snapshot, supported, None, &Default::default())?);
    }
    let high = prepared.pop().ok_or("Missing High LOD")?;
    let source = high.sources.first().cloned().ok_or("Empty High LOD")?;
    let mut lods = Vec::new();
    for (index, low) in prepared.into_iter().enumerate() {
        let low = low.sources.first().ok_or("Empty lower LOD")?;
        lods.push(aligned_scene(&low.scene, &source.scene, &mappings[index])?);
    }
    Ok(Bundle {
        high: source,
        images: high.images,
        lods: lods.try_into().map_err(|_| "Missing lower LODs")?,
    })
}

impl Live {
    pub fn request_import(&mut self, source: Uuid) {
        if self.import.busy() {
            return;
        }
        let shared = self.server.as_ref().map(|server| &server.shared);
        let capable = shared.is_some_and(|shared| {
            let status = shared.status.lock();
            status.connected && status.import_capable
        });
        if !capable {
            self.import.message = aurora_i18n::tr!("blender-import-connect").into();
            self.import.error = true;
            return;
        }
        if let Some(shared) = shared {
            let request = Request {
                id: Uuid::new_v4(),
                source,
            };
            *shared.import_expected.lock() = Some(request);
            *shared.import_request.lock() = Some(request);
            self.import.active = Some((request, Instant::now()));
            self.import.error = false;
            self.import.message = aurora_i18n::tr!("blender-import-preparing").into();
        }
    }
    pub fn poll_import(&mut self) {
        let Some((request, start)) = self.import.active else { return };
        let shared = self.server.as_ref().map(|server| &server.shared);
        if let Some(shared) = shared
            && let Some((id, result)) = shared.import_result.lock().take()
            && id == request.id
        {
            self.import.active = None;
            self.import.error = result.is_err();
            match result {
                Ok(bundle) => {
                    self.import.ready = Some(bundle);
                    self.import.message = aurora_i18n::tr!("blender-import-ready").into();
                }
                Err(error) => self.import.message = aurora_i18n::tr!("blender-import-failed", error = error),
            }
        } else if start.elapsed() > Duration::from_secs(180) || shared.is_none_or(|s| !s.status.lock().connected) {
            if let Some(shared) = shared {
                shared.import_request.lock().take();
                shared.import_expected.lock().take();
            }
            self.import.active = None;
            self.import.error = true;
            self.import.message = aurora_i18n::tr!("blender-import-timeout").into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    #[test]
    fn blender_lods_request_requires_capability_times_out_and_preserves_live_state() {
        let shared = Arc::new(super::super::Shared::default());
        shared.status.lock().connected = true;
        let mut live = Live {
            server: Some(super::super::Server { shared: shared.clone() }),
            ..Default::default()
        };
        live.request_import(Uuid::from_u128(456));
        assert!(!live.import.busy());
        assert!(live.import.error);
        shared.status.lock().import_capable = true;
        live.request_import(Uuid::from_u128(456));
        let (request, _) = live.import.active.expect("pending");
        assert_eq!(shared.import_request.lock().as_ref().expect("request").source, request.source);
        live.import.active = Some((request, Instant::now() - Duration::from_secs(181)));
        live.poll_import();
        assert!(live.import.error && !live.import.busy());
        assert!(shared.import_expected.lock().is_none() && shared.import_request.lock().is_none());
        assert!(live.snapshot.is_none() && shared.latest.lock().is_none());
        live.request_import(request.source);
        let (request, _) = live.import.active.expect("next request");
        *shared.import_result.lock() = Some((request.id, Err("Missing LOD object".into())));
        live.poll_import();
        assert!(live.import.error && !live.import.busy());
        assert!(live.import.message.contains("Missing LOD object"));
    }

    fn fixture() -> (serde_json::Value, Request, HashSet<String>) {
        let request = Request {
            id: Uuid::from_u128(123),
            source: Uuid::from_u128(456),
        };
        let vertex =
            |position| serde_json::json!({"position":position,"normal":[0,0,1],"uv":[0.2,0.7],"joints":[0,0,0,0],"weights":[1,0,0,0]});
        let face = serde_json::json!({"color":[0.3,0.5,0.8,1],"vertices":[vertex([0.,0.,0.]),vertex([0.4,0.,0.]),vertex([0.,0.4,0.4])],"indices":[0,1,2]});
        let level = serde_json::json!({"mesh":{"source_id":request.source,"name":"Cloth","joints":["mPelvis"],"inverse_bind":[Mat4::IDENTITY.to_cols_array()],"faces":[face]},"slots":[0],"origin":"Automatic"});
        (
            serde_json::json!({"id":request.id,"source":request.source,"levels":vec![level;4],"images":[]}),
            request,
            HashSet::from(["mPelvis".into()]),
        )
    }

    #[test]
    fn blender_lods_static_sources_keep_all_levels_unskinned() {
        let (mut value, request, supported) = fixture();
        for level in value["levels"].as_array_mut().expect("levels") {
            level["mesh"]["joints"] = serde_json::json!([]);
            level["mesh"]["inverse_bind"] = serde_json::json!([]);
            for face in level["mesh"]["faces"].as_array_mut().expect("faces") {
                for vertex in face["vertices"].as_array_mut().expect("vertices") {
                    vertex["weights"] = serde_json::json!([0, 0, 0, 0]);
                }
            }
        }
        let bundle = decode(&serde_json::to_vec(&value).expect("JSON"), request, &supported).expect("static LODs");
        assert!(!bundle.high.rigged());
        for level in &bundle.lods {
            assert!(level.models.iter().all(|model| model.skin.is_none()));
        }
    }

    #[test]
    fn blender_lods_validate_all_levels_and_pad_missing_materials_without_losing_skin() {
        let (mut data, request, supported) = fixture();
        let mut second = data["levels"][3]["mesh"]["faces"][0].clone();
        second["color"] = serde_json::json!([0.8, 0.1, 0.1, 1]);
        data["levels"][3]["mesh"]["faces"].as_array_mut().expect("faces").push(second);
        data["levels"][3]["slots"] = serde_json::json!([0, 1]);
        for level in &mut data["levels"].as_array_mut().expect("levels")[..3] {
            level["slots"] = serde_json::json!([1]);
        }
        let result = decode(&serde_json::to_vec(&data).expect("wire"), request, &supported).expect("LOD bundle");
        assert_eq!(result.high.scene.models[0].faces.len(), 2);
        for level in result.lods {
            assert!(level.models[0].faces[0].vertices.is_empty());
            assert_eq!(level.models[0].faces[1].material, 1);
            assert_eq!(level.models[0].faces[1].vertices[0].weights, [1., 0., 0., 0.]);
            assert_eq!(level.models[0].skin.as_ref().expect("skin").names, ["mPelvis"]);
        }
        for case in 0..4 {
            let mut bad = data.clone();
            match case {
                0 => bad["source"] = serde_json::json!(Uuid::new_v4()),
                1 => bad["levels"][0]["mesh"]["faces"][0]["vertices"][0]["weights"] = serde_json::json!([0, 0, 0, 0]),
                2 => bad["levels"][0]["slots"] = serde_json::json!([3]),
                _ => bad["levels"][0]["mesh"]["inverse_bind"][0][12] = serde_json::json!(1),
            }
            assert!(decode(&serde_json::to_vec(&bad).expect("wire"), request, &supported).is_err());
        }
    }

    #[test]
    #[ignore = "requires AURORA_TEST_LODS_OUTPUT from the native Blender suite"]
    fn blender_lods_native_fixture_decodes_every_level_with_common_bind_palette() {
        let bytes = std::fs::read(std::env::var_os("AURORA_TEST_LODS_OUTPUT").expect("fixture path")).expect("fixture");
        let request = serde_json::from_slice::<Request>(&bytes).expect("request identity");
        let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml")).expect("skeleton");
        let names = skeleton.joints.iter().map(|joint| joint.name.clone()).collect();
        let bundle = decode(&bytes, request, &names).expect("native LODs");
        let counts: Vec<_> = bundle
            .lods
            .iter()
            .map(|scene| scene.models.iter().map(|model| model.triangles()).sum::<usize>())
            .collect();
        assert_eq!(counts, [126, 251, 502]);
        assert_eq!(bundle.high.scene.models.iter().map(|model| model.triangles()).sum::<usize>(), 1004);
    }
}
