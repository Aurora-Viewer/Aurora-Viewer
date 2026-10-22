use super::*;
use glam::Mat4;
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpStream},
    time::Duration,
};

fn triangle() -> serde_json::Value {
    let vertices: Vec<_> = [[0.0,0.0,0.0], [0.2,0.0,0.0], [0.0,0.2,0.0]].into_iter().map(|position| {
        serde_json::json!({"position":position,"normal":[0.0,0.0,1.0],"uv":[0.0,0.0],"joints":[0,0,0,0],"weights":[2.0,0.0,0.0,0.0]})
    }).collect();
    serde_json::json!({"protocol":1,"revision":1,"meshes":[{
        "name":"Test garment","joints":["mPelvis"],"inverse_bind":[Mat4::IDENTITY.to_cols_array()],
        "faces":[{"color":[0.3,0.2,0.8,1.0],"vertices":vertices,"indices":[0,1,2]}]
    }]})
}
fn supported() -> HashSet<String> {
    HashSet::from(["mPelvis".into()])
}
fn prepare(value: &serde_json::Value) -> Result<protocol::Prepared, String> {
    protocol::prepare(&serde_json::to_vec(value).expect("JSON"), &supported())
}

#[test]
fn blender_live_round_trip_keeps_skin_and_materials() {
    let prepared = prepare(&triangle()).expect("valid mesh");
    assert_eq!(prepared.revision, 1);
    assert_eq!(prepared.triangles, 1);
    assert_eq!(prepared.meshes[0].skin.joint_names, ["mPelvis"]);
    assert_eq!(prepared.meshes[0].colors, [[0.3, 0.2, 0.8, 1.0]]);
    let mesh = &prepared.meshes[0];
    let range = mesh.header.lods[3].expect("high LOD");
    let section = aurora_assets::mesh::section_bytes(&mesh.data, range).expect("section");
    let decoded = aurora_assets::decode_mesh_lod(section).expect("LOD");
    assert!(!decoded.is_empty());
}

#[test]
fn blender_live_embedded_pbr_keeps_factors_maps_transforms_and_content_identity() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut value = triangle();
    value["images"] = serde_json::json!([{"width":2,"height":1,"rgba":STANDARD.encode([255,0,0,255,0,255,0,128])}]);
    value["meshes"][0]["faces"][0]["material"] = serde_json::json!({
        "metallic":0.7,"roughness":0.2,"emissive":[0.1,0.2,0.3],"double_sided":true,
        "alpha_mode":"BLEND","textures":[0,null,null,0],
        "transforms":[[0.1,0.2,2.0,3.0,0.4],[0,0,1,1,0],[0,0,1,1,0],[0,0,1,1,0]]
    });
    let first = prepare(&value).expect("PBR snapshot");
    let second = prepare(&value).expect("repeated snapshot");
    assert_eq!(
        first.images[0].id, second.images[0].id,
        "unchanged pixels reuse the local texture slot"
    );
    assert!(first.images[0].has_alpha);
    let (_, material) = first.meshes[0].materials[0].as_ref().expect("PBR material");
    assert_eq!(material.base_color_factor, [0.3, 0.2, 0.8, 1.0]);
    assert_eq!(material.metallic_factor, 0.7);
    assert_eq!(material.roughness_factor, 0.2);
    assert_eq!(material.base_color_texture, Some(first.images[0].id));
    assert_eq!(material.emissive_texture, material.base_color_texture);
    assert_eq!(material.alpha_mode, aurora_assets::material::AlphaMode::Blend);
    assert_eq!(material.transforms[0].scale, [2.0, 3.0]);
    value["images"][0]["rgba"] = STANDARD.encode([0, 0, 255, 255, 255, 255, 0, 255]).into();
    assert_ne!(first.images[0].id, prepare(&value).expect("painted image").images[0].id);
}

#[test]
fn blender_live_rejects_invalid_materials_images_and_references() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut base = triangle();
    base["images"] = serde_json::json!([{"width":1,"height":1,"rgba":STANDARD.encode([255;4])}]);
    base["meshes"][0]["faces"][0]["material"] = serde_json::json!({
        "metallic":0.0,"roughness":0.5,"emissive":[0,0,0],"double_sided":true,"alpha_mode":"OPAQUE","textures":[0,null,null,null]
    });
    for case in 0..8 {
        let mut value = base.clone();
        match case {
            0 => value["images"][0]["width"] = 1025.into(),
            1 => value["images"][0]["rgba"] = "not base64!".into(),
            2 => value["images"][0]["height"] = 0.into(),
            3 => value["meshes"][0]["faces"][0]["material"]["textures"][0] = 1.into(),
            4 => value["meshes"][0]["faces"][0]["material"]["metallic"] = 2.into(),
            5 => value["meshes"][0]["faces"][0]["material"]["alpha_mode"] = "UNKNOWN".into(),
            6 => value["meshes"][0]["faces"][0]["material"]["emissive"][0] = (-1).into(),
            _ => value["images"] = serde_json::Value::Array(vec![value["images"][0].clone(); 17]),
        }
        assert!(prepare(&value).is_err(), "invalid appearance case {case}");
    }
}

#[test]
fn blender_live_rejects_bad_skin_geometry_and_protocol_without_replacing_good_state() {
    for change in 0..8 {
        let mut value = triangle();
        match change {
            0 => value["protocol"] = 2.into(),
            1 => value["meshes"][0]["joints"][0] = "unknown".into(),
            2 => value["meshes"][0]["faces"][0]["indices"][2] = 99.into(),
            3 => value["meshes"][0]["faces"][0]["vertices"][0]["weights"] = serde_json::json!([0, 0, 0, 0]),
            4 => value["meshes"][0]["faces"][0]["vertices"][0]["joints"][0] = 2.into(),
            5 => value["meshes"][0]["inverse_bind"][0] = serde_json::to_value([0.0; 16]).expect("matrix"),
            6 => value["meshes"][0]["faces"][0]["vertices"][0]["weights"] = serde_json::json!([2, -1, 0, 0]),
            _ => value["meshes"][0]["faces"][0]["color"][0] = (-1).into(),
        }
        assert!(prepare(&value).is_err(), "invalid case {change}");
    }
    assert!(protocol::prepare(b"{}", &supported()).is_err());
    let empty = prepare(&serde_json::json!({"protocol":1,"revision":2,"meshes":[]})).expect("clear snapshot");
    assert!(empty.meshes.is_empty());
}

#[test]
fn blender_live_splits_many_materials_without_discarding_triangles() {
    let mut value = triangle();
    let face = value["meshes"][0]["faces"][0].clone();
    value["meshes"][0]["faces"] = serde_json::Value::Array(vec![face; 9]);
    let prepared = prepare(&value).expect("nine surfaces");
    assert_eq!(prepared.meshes.len(), 2);
    assert_eq!(prepared.triangles, 9);
}

#[test]
fn blender_live_attachment_namespace_survives_rekey_without_colliding_with_simulator_objects() {
    use crate::world::objects::{ObjectStore, PREVIEW_REGION};
    let mut store = ObjectStore::default();
    let mut avatar = crate::demo::action_avatar(false);
    let avatar_id = avatar.full_id;
    let avatar_index = store.upsert(0, avatar.clone());
    let parent = store.get(avatar_index).expect("avatar").key;
    let mut preview = avatar.clone();
    preview.local_id = 1;
    preview.full_id = Uuid::new_v4();
    preview.pcode = aurora_prim::params::LL_PCODE_VOLUME;
    let preview_id = preview.full_id;
    let preview_index = store.insert_preview_attachment(parent, preview.clone());
    assert_eq!(store.get(preview_index).expect("preview").key.region, PREVIEW_REGION);
    assert_eq!(store.parent_of(store.get(preview_index).expect("preview")), Some(avatar_index));
    preview.full_id = Uuid::new_v4();
    preview.parent_id = 0;
    let server_index = store.upsert(0, preview);
    assert_ne!(server_index, preview_index);
    avatar.local_id = 900;
    store.upsert(256, avatar);
    let parent = store.get(avatar_index).expect("moved avatar");
    assert_eq!(parent.full_id, avatar_id);
    assert_eq!(store.get(preview_index).expect("preview").parent_key(), Some(parent.key));
    store.remove_idx(avatar_index);
    assert!(store.index_of_uuid(&preview_id).is_none());
    assert!(store.get(server_index).is_some());
}

#[test]
fn blender_live_transport_handshake_errors_reconnect_and_latest_slot() {
    let ctx = egui::Context::default();
    let probe = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("free port");
    let port = probe.local_addr().expect("address").port();
    let shared = transport::start_listener(probe, supported(), Default::default(), ctx, None).expect("start");
    let owner = Server { shared: shared.clone() };
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    stream.write_all(b"AURBLD1\n").expect("magic");
    let mut reader = BufReader::new(stream.try_clone().expect("reader"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("hello");
    assert!(line.contains("mPelvis"));
    assert!(line.contains("pbr-materials"));
    for revision in 1..=3 {
        let mut value = triangle();
        value["revision"] = revision.into();
        let bytes = serde_json::to_vec(&value).expect("snapshot");
        stream.write_all(&(bytes.len() as u32).to_le_bytes()).expect("length");
        stream.write_all(&bytes).expect("data");
        line.clear();
        reader.read_line(&mut line).expect("ack");
        assert!(line.contains("\"ok\":true"));
    }
    assert_eq!(shared.latest.lock().as_ref().expect("latest").revision, 3);
    stream.write_all(&2u32.to_le_bytes()).expect("bad length");
    stream.write_all(b"{}").expect("bad data");
    line.clear();
    reader.read_line(&mut line).expect("rejection");
    assert!(line.contains("\"ok\":false"));
    assert_eq!(shared.latest.lock().as_ref().expect("unchanged").revision, 3);
    drop(owner);
}
