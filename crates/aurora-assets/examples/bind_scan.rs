//! Diagnostic: how far rigged meshes' bind poses (inverse bind matrices) are
//! from the default SL skeleton. Usage: bind_scan <cache dir> <avatar_skeleton.xml>
use std::collections::HashMap;

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("dir");
    let skel = a.next().expect("skeleton");
    let sk = aurora_assets::Skeleton::parse(&std::fs::read(skel).unwrap()).unwrap();
    let mut def: HashMap<String, glam::Vec3> = HashMap::new();
    for j in &sk.joints {
        def.insert(j.name.clone(), j.world_position());
    }
    let mut buckets = [0usize; 6]; // <1cm <5cm <10cm <30cm <1m >=1m (worst joint per mesh)
    let mut rot_meshes = 0;
    let mut shown = 0;
    let mut total = 0;
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
            let mut worst = 0.0f32;
            let mut worst_name = String::new();
            let mut rotated = false;
            for (n, ib) in s.joint_names.iter().zip(&s.inverse_bind) {
                let Some(d) = def.get(n) else { continue };
                let bind = ib.inverse();
                let p = bind.w_axis.truncate();
                let dist = (p - *d).length();
                if dist > worst {
                    worst = dist;
                    worst_name = n.clone();
                }
                let (_, r, _) = bind.to_scale_rotation_translation();
                if r.angle_between(glam::Quat::IDENTITY) > 0.05 {
                    rotated = true;
                }
            }
            if rotated {
                rot_meshes += 1;
            }
            let b = match worst {
                w if w < 0.01 => 0,
                w if w < 0.05 => 1,
                w if w < 0.1 => 2,
                w if w < 0.3 => 3,
                w if w < 1.0 => 4,
                _ => 5,
            };
            buckets[b] += 1;
            if b >= 3 && shown < 6 {
                shown += 1;
                println!(
                    "{} worst {worst_name} {worst:.3} m, bind shape t {:?}",
                    e.path().display(),
                    s.bind_shape.w_axis.truncate()
                );
            }
        }
    }
    println!("rigged {total}, worst joint offset buckets <1cm <5cm <10cm <30cm <1m >=1m: {buckets:?}, with rotated bind: {rot_meshes}");
}
