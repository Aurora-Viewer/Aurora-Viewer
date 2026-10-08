//! Diagnostic: try the mesh decoder on a directory of cached SL assets
//! (e.g. a Firestorm cache). Usage: mesh_scan <dir> [max]
use std::collections::HashMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("dir");
    let max: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);
    let mut stats: HashMap<String, usize> = HashMap::new();
    let mut seen = 0;
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Ok(raw) = std::fs::read(&p) else { continue };
            // Firestorm mesh cache: 12-byte header before the asset
            let data: Vec<u8> = if raw.len() > 12 && raw[0..4] == [1, 0, 0, 0] && raw[12] == b'{' {
                raw[12..].to_vec()
            } else {
                raw
            };
            if data.first() != Some(&b'{') {
                continue;
            }
            seen += 1;
            let key = match aurora_assets::parse_mesh_header(&data) {
                Ok(h) => {
                    let mut ok = 0;
                    let mut err = String::new();
                    for r in h.lods.iter().flatten() {
                        match aurora_assets::mesh::section_bytes(&data, *r).map(aurora_assets::decode_mesh_lod) {
                            Some(Ok(_)) => ok += 1,
                            Some(Err(e)) => err = e.to_string(),
                            None => err = "section out of range".into(),
                        }
                    }
                    if err.is_empty() {
                        format!("ok ({ok} lods)")
                    } else {
                        format!("lod error: {err}")
                    }
                }
                Err(e) => format!("header error: {e}"),
            };
            if seen <= 3 || !key.starts_with("ok") {
                println!("{} -> {key}", p.display());
            }
            *stats.entry(key.split(':').next().unwrap_or("").to_owned()).or_default() += 1;
            if seen >= max {
                println!("{stats:?}");
                return;
            }
        }
    }
    println!("{stats:?}");
}
