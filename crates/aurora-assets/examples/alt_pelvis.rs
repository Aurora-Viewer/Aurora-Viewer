//! Diagnostic: joint positions carried by rigged meshes (alt inverse bind
//! translations) for a few joints. Usage: alt_pelvis <cache dir>
use std::collections::HashMap;
fn main() {
    let dir = std::env::args().nth(1).expect("dir");
    let mut seen: HashMap<String, Vec<glam::Vec3>> = HashMap::new();
    for sub in std::fs::read_dir(&dir).unwrap().flatten() {
        let Ok(rd) = std::fs::read_dir(sub.path()) else { continue };
        for e in rd.flatten() {
            let Ok(raw) = std::fs::read(e.path()) else { continue };
            let data = if raw.len() > 12 && raw[0..4] == [1, 0, 0, 0] && raw[12] == b'{' {
                &raw[12..]
            } else {
                continue;
            };
            let Ok(h) = aurora_assets::parse_mesh_header(data) else { continue };
            let Some(r) = h.skin else { continue };
            let Some(sec) = aurora_assets::mesh::section_bytes(data, r) else {
                continue;
            };
            let Ok(s) = aurora_assets::decode_skin(sec) else { continue };
            if s.alt_inverse_bind.len() != s.joint_names.len() {
                continue;
            }
            for (n, m) in s.joint_names.iter().zip(&s.alt_inverse_bind) {
                if ["mPelvis", "mTorso", "mHipLeft", "mHead"].contains(&n.as_str()) {
                    seen.entry(n.clone()).or_default().push(m.w_axis.truncate());
                }
            }
        }
    }
    for (n, v) in seen {
        let mut z: Vec<f32> = v.iter().map(|p| p.z).collect();
        z.sort_by(f32::total_cmp);
        println!(
            "{n}: {} meshes, z min {:.3} median {:.3} max {:.3}, first {:?}",
            v.len(),
            z[0],
            z[z.len() / 2],
            z[z.len() - 1],
            v[0]
        );
    }
}
