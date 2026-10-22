use super::protocol;
use crate::scene::anim::Rig;
use glam::Vec3;

pub fn fixture(rig: &Rig) -> Result<protocol::Prepared, String> {
    let joint = rig.names.get("mChest").copied().ok_or("No chest joint")?;
    let inverse = rig.default_world_inv[joint];
    let center = inverse.inverse().w_axis.truncate();
    let vertices: Vec<_> = (0..8).map(|i| {
        let offset = Vec3::new(if i & 1 == 0 { -0.16 } else { 0.16 }, if i & 2 == 0 { -0.26 } else { 0.26 }, if i & 4 == 0 { -0.19 } else { 0.19 });
        serde_json::json!({ "position": (center + offset).to_array(), "normal": offset.normalize().to_array(), "uv": [0.0, 0.0], "joints": [0,0,0,0], "weights": [1.0,0.0,0.0,0.0] })
    }).collect();
    let bytes = serde_json::to_vec(&serde_json::json!({ "protocol": 1, "revision": 1, "meshes": [{
        "name": "Live chest shell", "joints": ["mChest"], "inverse_bind": [inverse.to_cols_array()],
        "faces": [{ "color": [0.40, 0.20, 0.90, 1.0], "vertices": vertices,
            "indices": [0,2,1,1,2,3,4,5,6,5,7,6,0,1,4,1,5,4,2,6,3,3,6,7,0,4,2,2,4,6,1,3,5,3,7,5] }]
    }] }))
    .map_err(|e| e.to_string())?;
    protocol::prepare(&bytes, &rig.names.keys().cloned().collect())
}
