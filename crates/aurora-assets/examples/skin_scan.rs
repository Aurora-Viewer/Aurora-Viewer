//! Diagnostic: joint names used by cached rigged meshes that the avatar
//! skeleton does not define. Usage: skin_scan <cache dir> <avatar_skeleton.xml>
use std::collections::{BTreeMap, HashSet};

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("dir");
    let skel = a.next().expect("skeleton");
    let sk = aurora_assets::Skeleton::parse(&std::fs::read(skel).unwrap()).unwrap();
    let mut known: HashSet<String> = HashSet::new();
    for j in &sk.joints {
        known.insert(j.name.clone());
        known.extend(j.aliases.iter().cloned());
    }
    println!("skeleton: {} joints", sk.joints.len());
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    let mut rigged = 0;
    let mut alt = 0;
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
            rigged += 1;
            if !s.alt_inverse_bind.is_empty() {
                alt += 1;
            }
            for n in &s.joint_names {
                if !known.contains(n) {
                    *unknown.entry(n.clone()).or_default() += 1;
                }
            }
        }
    }
    println!("rigged meshes: {rigged} (with joint positions: {alt})");
    for (n, c) in unknown {
        println!("unknown joint {n}: {c}");
    }
}
