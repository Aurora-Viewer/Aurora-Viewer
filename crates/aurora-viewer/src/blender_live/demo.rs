use super::protocol;
use crate::scene::anim::Rig;
use glam::Vec3;

pub fn fixture(rig: &Rig, multiple: bool) -> Result<protocol::Prepared, String> {
    let joint = rig.names.get("mChest").copied().ok_or("No chest joint")?;
    let inverse = rig.default_world_inv[joint];
    let center = inverse.inverse().w_axis.truncate();
    let vertices: Vec<_> = (0..8).map(|i| {
        let offset = Vec3::new(if i & 1 == 0 { -0.16 } else { 0.16 }, if i & 2 == 0 { -0.26 } else { 0.26 }, if i & 4 == 0 { -0.19 } else { 0.19 });
        serde_json::json!({ "position": (center + offset).to_array(), "normal": offset.normalize().to_array(), "uv": [0.0, 0.0], "joints": [0,0,0,0], "weights": [1.0,0.0,0.0,0.0] })
    }).collect();
    let shell = serde_json::json!({
        "name": "Live chest shell", "joints": ["mChest"], "inverse_bind": [inverse.to_cols_array()],
        "faces": [{ "color": [0.40, 0.20, 0.90, 1.0], "vertices": vertices,
            "indices": [0,2,1,1,2,3,4,5,6,5,7,6,0,1,4,1,5,4,2,6,3,3,6,7,0,4,2,2,4,6,1,3,5,3,7,5] }]
    });
    let mut meshes = vec![shell.clone()];
    if multiple {
        for (name, scale, offset, color) in [
            (
                "Live belt",
                Vec3::new(1.1, 1.05, 0.15),
                Vec3::new(0.0, 0.0, -0.3),
                [0.1, 0.7, 0.65, 1.0],
            ),
            (
                "Live neck band",
                Vec3::new(0.7, 0.4, 0.1),
                Vec3::new(0.0, 0.0, 0.3),
                [0.9, 0.35, 0.2, 1.0],
            ),
        ] {
            let mut mesh = shell.clone();
            mesh["name"] = name.into();
            mesh["faces"][0]["color"] = serde_json::json!(color);
            for vertex in mesh["faces"][0]["vertices"].as_array_mut().ok_or("No demo vertices")? {
                let position: [f32; 3] = serde_json::from_value(vertex["position"].clone()).map_err(|e| e.to_string())?;
                vertex["position"] = serde_json::json!(((Vec3::from_array(position) - center) * scale + center + offset).to_array());
            }
            meshes.push(mesh);
        }
    }
    let bytes = serde_json::to_vec(&serde_json::json!({"protocol":1,"revision":1,"meshes":meshes})).map_err(|e| e.to_string())?;
    protocol::prepare(&bytes, &rig.names.keys().cloned().collect())
}

pub fn static_fixture(rig: &Rig, linked: bool) -> Result<protocol::Prepared, String> {
    let vertices: Vec<_> = (0..8)
        .map(|i| {
            let p = Vec3::new(
                if i & 1 == 0 { -0.25 } else { 0.25 },
                if i & 2 == 0 { -0.2 } else { 0.2 },
                if i & 4 == 0 { -0.15 } else { 0.15 },
            );
            serde_json::json!({"position":p.to_array(),"normal":p.normalize().to_array(),"uv":[0,0],"joints":[0,0,0,0],"weights":[0,0,0,0]})
        })
        .collect();
    let root = uuid::Uuid::from_u128(101);
    let mut meshes = vec![serde_json::json!({
        "name":"Live static object","source_id":root,"joints":[],"inverse_bind":[],
        "faces":[{"color":[0.1,0.8,0.65,1.0],"vertices":vertices,"indices":[0,2,1,1,2,3,4,5,6,5,7,6,0,1,4,1,5,4,2,6,3,3,6,7,0,4,2,2,4,6,1,3,5,3,7,5]}]
    })];
    if linked {
        let mut child = meshes[0].clone();
        child["source_id"] = serde_json::json!(uuid::Uuid::from_u128(102));
        child["name"] = "Live child object".into();
        child["faces"][0]["color"] = serde_json::json!([0.95, 0.4, 0.15, 1.0]);
        child["link"] = serde_json::json!({"parent":root,"offset":[0.0,0.65,0.0]});
        // Child first also exercises list-order independence in the worker.
        meshes.insert(0, child);
    }
    let bytes = serde_json::to_vec(&serde_json::json!({"protocol":1,"revision":1,"meshes":meshes})).map_err(|e| e.to_string())?;
    let mut snapshot = protocol::prepare(&bytes, &rig.names.keys().cloned().collect())?;
    let rigged = fixture(rig, false)?;
    snapshot.triangles += rigged.triangles;
    snapshot.meshes.extend(rigged.meshes);
    snapshot.sources.extend(rigged.sources);
    Ok(snapshot)
}
