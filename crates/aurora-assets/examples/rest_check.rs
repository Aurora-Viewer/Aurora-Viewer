//! Diagnostic: at the default pose, joint_world * inverse_bind should be close
//! to identity for ordinary rigged meshes. Compares world matrices with and
//! without the joints' own scale. Usage: rest_check <cache dir> <skeleton.xml>
use std::collections::HashMap;

fn dev(m: glam::Mat4) -> f32 {
    (m - glam::Mat4::IDENTITY)
        .to_cols_array()
        .iter()
        .map(|v| v.abs())
        .fold(0.0, f32::max)
}

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("dir");
    let sk = aurora_assets::Skeleton::parse(&std::fs::read(a.next().expect("skel")).unwrap()).unwrap();
    let mut with_s: HashMap<String, glam::Mat4> = HashMap::new();
    let mut no_s: HashMap<String, glam::Mat4> = HashMap::new();
    for j in &sk.joints {
        with_s.insert(j.name.clone(), j.world);
        let (_, r, t) = j.world.to_scale_rotation_translation();
        no_s.insert(j.name.clone(), glam::Mat4::from_rotation_translation(r, t));
    }
    let (mut good_s, mut good_n, mut total) = (0, 0, 0);
    let mut cv_dev = (0.0f32, 0.0f32, 0usize);
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
            total += 1;
            let mut ds = Vec::new();
            let mut dn = Vec::new();
            for (n, ib) in s.joint_names.iter().zip(&s.inverse_bind) {
                let (Some(ws), Some(wn)) = (with_s.get(n), no_s.get(n)) else {
                    continue;
                };
                let a = dev(*ws * *ib);
                let b = dev(*wn * *ib);
                ds.push(a);
                dn.push(b);
                if n.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                    cv_dev.0 += a;
                    cv_dev.1 += b;
                    cv_dev.2 += 1;
                }
            }
            if ds.is_empty() {
                continue;
            }
            ds.sort_by(f32::total_cmp);
            dn.sort_by(f32::total_cmp);
            if ds[ds.len() / 2] < 0.05 {
                good_s += 1;
            }
            if dn[dn.len() / 2] < 0.05 {
                good_n += 1;
            }
        }
    }
    println!("rigged {total}: median joint near identity with joint scale {good_s}, without {good_n}");
    if cv_dev.2 > 0 {
        println!(
            "collision volumes: mean deviation with scale {:.3}, without {:.3} ({} joints)",
            cv_dev.0 / cv_dev.2 as f32,
            cv_dev.1 / cv_dev.2 as f32,
            cv_dev.2
        );
    }
}
