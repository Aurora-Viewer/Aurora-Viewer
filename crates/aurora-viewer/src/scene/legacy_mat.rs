//! Legacy (pre-PBR) materials: normal and specular maps, alpha mode and
//! mask cutoff of a texture entry (`TextureFace::material_id`), fetched
//! from the region's `RenderMaterials` capability like LLMaterialMgr:
//! POST `{"Zipped": zlib(binary LLSD [material id binaries])}`, answer
//! `{"Zipped": zlib(binary LLSD [{"ID": binary, "Material": map}])}`.
//!
//! Field names and scaling ported from Firestorm's llmaterial.cpp and
//! llmaterialmgr.cpp (Copyright (C) Linden Research, Inc., GNU originally LGPL 2.1).

use aurora_llsd::Llsd;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// LLMaterial::DIFFUSE_ALPHA_MODE_*.
pub mod alpha_mode {
    pub const NONE: u8 = 0;
    pub const BLEND: u8 = 1;
    pub const MASK: u8 = 2;
    pub const EMISSIVE: u8 = 3;
}

/// Fixed-point scale of offsets, repeats and rotations.
const MATERIALS_MULTIPLIER: f32 = 10000.0;
/// LLMaterialMgr: at most 50 ids per request.
const MAX_PER_REQUEST: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyMaterial {
    pub normal_map: Uuid,
    /// scale s, t, offset s, t; rotation (radians)
    pub normal_st: [f32; 4],
    pub normal_rot: f32,
    pub specular_map: Uuid,
    pub specular_st: [f32; 4],
    pub specular_rot: f32,
    /// Specular light color (sRGB 0..255) and exponent (0..255, glossiness).
    pub specular_color: [u8; 4],
    pub specular_exp: u8,
    pub env_intensity: u8,
    pub diffuse_alpha_mode: u8,
    /// Alpha mask cutoff (0..255).
    pub alpha_cutoff: u8,
}

impl LegacyMaterial {
    pub fn from_llsd(m: &Llsd) -> LegacyMaterial {
        let f = |k: &str| m.get(k).as_i32() as f32 / MATERIALS_MULTIPLIER;
        let byte = |k: &str| m.get(k).as_i32().clamp(0, 255) as u8;
        let color = m.get("SpecColor").as_array();
        let c = |i: usize| color.get(i).map(|v| v.as_i32().clamp(0, 255) as u8).unwrap_or(255);
        LegacyMaterial {
            normal_map: m.get("NormMap").as_uuid(),
            normal_st: [f("NormRepeatX"), f("NormRepeatY"), f("NormOffsetX"), f("NormOffsetY")],
            normal_rot: f("NormRotation"),
            specular_map: m.get("SpecMap").as_uuid(),
            specular_st: [f("SpecRepeatX"), f("SpecRepeatY"), f("SpecOffsetX"), f("SpecOffsetY")],
            specular_rot: f("SpecRotation"),
            specular_color: [c(0), c(1), c(2), c(3)],
            specular_exp: byte("SpecExp"),
            env_intensity: byte("EnvIntensity"),
            diffuse_alpha_mode: byte("DiffuseAlphaMode"),
            alpha_cutoff: byte("AlphaMaskCutoff"),
        }
    }
}

/// Zip an LLSD value like LL's `zip_llsd` (binary LLSD, zlib).
pub fn zip_llsd(v: &Llsd) -> Vec<u8> {
    let raw = aurora_llsd::to_binary(v);
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    let _ = enc.write_all(&raw);
    enc.finish().unwrap_or_default()
}

/// LL's `unzip_llsd`: zlib, optional "<? LLSD/Binary ?>" header, binary LLSD.
pub fn unzip_llsd(data: &[u8]) -> Option<Llsd> {
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(64 * 1024 * 1024)
        .read_to_end(&mut raw)
        .ok()?;
    let mut body: &[u8] = &raw;
    if body.starts_with(b"<?") {
        let end = body.iter().position(|&b| b == b'\n')?;
        body = &body[end + 1..];
    }
    aurora_llsd::from_binary(body).ok().map(|(v, _)| v)
}

/// Parse a `RenderMaterials` answer into (id, material) pairs.
pub fn parse_response(resp: &Llsd) -> Vec<(Uuid, LegacyMaterial)> {
    let Some(list) = unzip_llsd(resp.get("Zipped").as_binary()) else {
        return Vec::new();
    };
    list.as_array()
        .iter()
        .filter_map(|e| {
            let id = Uuid::from_slice(e.get("ID").as_binary()).ok()?;
            let m = e.get("Material");
            m.as_map()?;
            Some((id, LegacyMaterial::from_llsd(m)))
        })
        .collect()
}

pub fn request_body(ids: &[Uuid]) -> Llsd {
    let list = Llsd::Array(ids.iter().map(|id| Llsd::Binary(id.as_bytes().to_vec())).collect());
    let mut m = aurora_llsd::Map::new();
    m.insert("Zipped".into(), Llsd::Binary(zip_llsd(&list)));
    Llsd::Map(m)
}

enum State {
    Wanted {
        cap: String,
        since: Instant,
    },
    Fetching,
    Ready(Arc<LegacyMaterial>),
    /// Forgotten at `retry_at` (the next `get` asks again).
    Failed {
        retry_at: Instant,
    },
}

pub struct LegacyMaterials {
    entries: HashMap<Uuid, State>,
    tx: crossbeam_channel::Sender<(Vec<Uuid>, Result<Vec<(Uuid, LegacyMaterial)>, String>)>,
    rx: crossbeam_channel::Receiver<(Vec<Uuid>, Result<Vec<(Uuid, LegacyMaterial)>, String>)>,
    in_flight: usize,
    /// Request errors written to the log (the first ones).
    errors_logged: u32,
}

impl Default for LegacyMaterials {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        LegacyMaterials {
            entries: HashMap::new(),
            tx,
            rx,
            in_flight: 0,
            errors_logged: 0,
        }
    }
}

impl LegacyMaterials {
    /// (ready, waiting or fetching, unknown to the region) materials.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for s in self.entries.values() {
            match s {
                State::Ready(_) => c.0 += 1,
                State::Wanted { .. } | State::Fetching => c.1 += 1,
                State::Failed { .. } => c.2 += 1,
            }
        }
        c
    }

    /// A material known locally (offline demo).
    pub fn insert(&mut self, id: Uuid, m: LegacyMaterial) {
        self.entries.insert(id, State::Ready(Arc::new(m)));
    }

    /// The material if already known (no request).
    pub fn peek(&self, id: &Uuid) -> Option<Arc<LegacyMaterial>> {
        match self.entries.get(id) {
            Some(State::Ready(m)) => Some(m.clone()),
            _ => None,
        }
    }

    /// The material if known; otherwise queue it on the region's cap.
    pub fn get(&mut self, id: &Uuid, cap: Option<&str>) -> Option<Arc<LegacyMaterial>> {
        match self.entries.get(id) {
            Some(State::Ready(m)) => return Some(m.clone()),
            Some(_) => return None,
            None => {}
        }
        if let Some(cap) = cap {
            self.entries.insert(
                *id,
                State::Wanted {
                    cap: cap.to_owned(),
                    since: Instant::now(),
                },
            );
        }
        None
    }

    /// Send queued requests (batches per cap, a few at a time) and collect
    /// answers. Returns true when materials arrived.
    pub fn update(&mut self, rt: &tokio::runtime::Handle, http: &reqwest::Client) -> bool {
        let mut arrived = false;
        while let Ok((asked, result)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            match result {
                Ok(list) => {
                    for (id, m) in list {
                        self.entries.insert(id, State::Ready(Arc::new(m)));
                        arrived = true;
                    }
                    // asked but not in the answer: unknown to this region
                    for id in asked {
                        if matches!(self.entries.get(&id), Some(State::Fetching)) {
                            self.entries.insert(
                                id,
                                State::Failed {
                                    retry_at: Instant::now() + Duration::from_secs(600),
                                },
                            );
                        }
                    }
                }
                Err(e) => {
                    self.errors_logged += 1;
                    if self.errors_logged <= 10 {
                        log::warn!("RenderMaterials request failed: {e}");
                    }
                    for id in asked {
                        if let Some(State::Fetching) = self.entries.get(&id) {
                            self.entries.insert(
                                id,
                                State::Failed {
                                    retry_at: Instant::now() + Duration::from_secs(10),
                                },
                            );
                        }
                    }
                }
            }
        }
        // failed entries expire: the next `get` queues them again
        let now = Instant::now();
        self.entries
            .retain(|_, s| !matches!(s, State::Failed { retry_at } if now >= *retry_at));
        if self.in_flight >= 4 {
            return arrived;
        }
        // group the wanted ids by cap (waiting a little to fill batches)
        let mut by_cap: HashMap<String, Vec<Uuid>> = HashMap::new();
        for (id, s) in self.entries.iter() {
            if let State::Wanted { cap, since } = s
                && since.elapsed() > Duration::from_millis(50)
            {
                by_cap.entry(cap.clone()).or_default().push(*id);
            }
        }
        for (cap, ids) in by_cap {
            for chunk in ids.chunks(MAX_PER_REQUEST) {
                if self.in_flight >= 4 {
                    return arrived;
                }
                for id in chunk {
                    self.entries.insert(*id, State::Fetching);
                }
                self.in_flight += 1;
                let (tx, http, cap, chunk) = (self.tx.clone(), http.clone(), cap.clone(), chunk.to_vec());
                rt.spawn(async move {
                    let body = request_body(&chunk);
                    let r = match post(&http, &cap, &body).await {
                        Ok(resp) => Ok(parse_response(&resp)),
                        Err(e) => Err(e),
                    };
                    let _ = tx.send((chunk, r));
                });
            }
        }
        arrived
    }
}

async fn post(http: &reqwest::Client, url: &str, body: &Llsd) -> Result<Llsd, String> {
    let resp = http
        .post(url)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(aurora_llsd::to_xml(body))
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    aurora_llsd::from_xml(&bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_round_trip_and_parse() {
        let id = Uuid::from_u128(0x1234);
        let mut mat = aurora_llsd::Map::new();
        mat.insert("NormMap".into(), Llsd::Uuid(Uuid::from_u128(5)));
        mat.insert("NormRepeatX".into(), Llsd::Integer(20000));
        mat.insert("NormRotation".into(), Llsd::Integer(15708));
        mat.insert(
            "SpecColor".into(),
            Llsd::Array(vec![Llsd::Integer(255), Llsd::Integer(128), Llsd::Integer(0), Llsd::Integer(255)]),
        );
        mat.insert("SpecExp".into(), Llsd::Integer(51));
        mat.insert("DiffuseAlphaMode".into(), Llsd::Integer(2));
        mat.insert("AlphaMaskCutoff".into(), Llsd::Integer(128));
        let mut entry = aurora_llsd::Map::new();
        entry.insert("ID".into(), Llsd::Binary(id.as_bytes().to_vec()));
        entry.insert("Material".into(), Llsd::Map(mat));
        let mut resp = aurora_llsd::Map::new();
        resp.insert("Zipped".into(), Llsd::Binary(zip_llsd(&Llsd::Array(vec![Llsd::Map(entry)]))));
        let list = parse_response(&Llsd::Map(resp));
        assert_eq!(list.len(), 1);
        let (got, m) = &list[0];
        assert_eq!(*got, id);
        assert_eq!(m.normal_map, Uuid::from_u128(5));
        assert!((m.normal_st[0] - 2.0).abs() < 1e-6 && (m.normal_rot - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        assert_eq!((m.diffuse_alpha_mode, m.alpha_cutoff, m.specular_exp), (alpha_mode::MASK, 128, 51));
        assert_eq!(m.specular_color, [255, 128, 0, 255]);
        // request: a zipped array of 16-byte ids
        let body = request_body(&[id]);
        let ids = unzip_llsd(body.get("Zipped").as_binary()).expect("unzip");
        assert_eq!(ids.as_array()[0].as_binary(), id.as_bytes());
    }
}
