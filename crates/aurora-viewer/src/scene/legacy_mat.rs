//! Legacy (pre-PBR) materials: normal and specular maps, alpha mode and
//! mask cutoff of a texture entry (`TextureFace::material_id`), fetched
//! from the region's `RenderMaterials` capability like LLMaterialMgr:
//! a GET of the cap answers every material of the region, then POST
//! `{"Zipped": zlib(binary LLSD [material id binaries])}` asks for the ones
//! still missing; both answer
//! `{"Zipped": zlib(binary LLSD [{"ID": binary, "Material": map}])}`.
//! Requests are throttled per region like LLMaterialMgr::processGetQueue
//! (the SimulatorFeatures rate, 1 per second by default), else the
//! simulator answers 503.
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

/// LLAppCoreHttp AP_MATERIALS: 2 connections shared by all regions.
const MAX_IN_FLIGHT: usize = 2;
/// LLMaterialMgr MATERIALS_POST_TIMEOUT: an id the region did not answer is
/// asked again after 5 minutes.
const UNKNOWN_RETRY: Duration = Duration::from_secs(300);
/// After an HTTP error (503 when the simulator is busy) the region gets no
/// request for ERROR_BACKOFF and the ids are asked again after FAILED_RETRY.
/// Deviation: LLMaterialMgr keeps failed ids pending 5 minutes and a failed
/// GET-all blocks the region's batches for 20 minutes (MATERIALS_GET_TIMEOUT);
/// here a failed GET-all falls back to the batches.
const ERROR_BACKOFF: Duration = Duration::from_secs(10);
const FAILED_RETRY: Duration = Duration::from_secs(30);

/// RenderMaterials limits of a region, from its SimulatorFeatures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionLimits {
    /// Requests per second ("RenderMaterialsCapability",
    /// LLViewerRegion::resetMaterialsCapThrottle).
    pub rate: f32,
    /// Material ids per request ("MaxMaterialsPerTransaction",
    /// LLViewerRegion::getMaxMaterialsPerTransaction).
    pub max_per_request: usize,
}

impl Default for RegionLimits {
    /// LL's hard-coded defaults: 1 request per second, 50 ids.
    fn default() -> Self {
        RegionLimits {
            rate: 1.0,
            max_per_request: 50,
        }
    }
}

impl RegionLimits {
    /// Limits from the SimulatorFeatures values (`None` = not given). A zero
    /// rate falls back to 1 per second like resetMaterialsCapThrottle.
    pub fn from_features(rate: Option<f32>, max: Option<u32>) -> Self {
        let d = RegionLimits::default();
        RegionLimits {
            rate: rate.filter(|r| r.is_finite() && *r > 0.0).unwrap_or(d.rate),
            max_per_request: max.filter(|m| *m > 0).map_or(d.max_per_request, |m| m as usize),
        }
    }

    fn interval(&self) -> Duration {
        Duration::from_secs_f32(1.0 / self.rate)
    }
}

/// The region a material is asked from.
#[derive(Debug, Clone, Copy)]
pub struct RegionCap<'a> {
    /// `RenderMaterials` capability URL.
    pub url: &'a str,
    pub limits: RegionLimits,
}

enum State {
    /// Queued on its region.
    Wanted,
    Fetching,
    Ready(Arc<LegacyMaterial>),
    /// Forgotten at `retry_at` (the next `get` asks again).
    Failed {
        retry_at: Instant,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GetAll {
    NotAsked,
    Pending,
    Done,
}

/// Queue and throttle of one region (keyed by its cap URL), like
/// LLMaterialMgr::mGetQueue / mGetAllRequested and the region's
/// mMaterialsCapThrottleTimer.
struct RegionQueue {
    limits: RegionLimits,
    get_all: GetAll,
    /// No request to the region before this.
    next_request: Instant,
    queue: Vec<Uuid>,
}

/// One HTTP request: `ids: None` is the region's GET-all.
#[derive(Debug, Clone, PartialEq)]
struct Request {
    cap: String,
    ids: Option<Vec<Uuid>>,
}

type Answer = (Request, Result<Vec<(Uuid, LegacyMaterial)>, String>);

pub struct LegacyMaterials {
    entries: HashMap<Uuid, State>,
    regions: HashMap<String, RegionQueue>,
    tx: crossbeam_channel::Sender<Answer>,
    rx: crossbeam_channel::Receiver<Answer>,
    in_flight: usize,
    /// Request errors written to the log (the first ones).
    errors_logged: u32,
}

impl Default for LegacyMaterials {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        LegacyMaterials {
            entries: HashMap::new(),
            regions: HashMap::new(),
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
                State::Wanted | State::Fetching => c.1 += 1,
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
    pub fn get(&mut self, id: &Uuid, region: Option<RegionCap>) -> Option<Arc<LegacyMaterial>> {
        match self.entries.get(id) {
            Some(State::Ready(m)) => return Some(m.clone()),
            Some(_) => return None,
            None => {}
        }
        if let Some(region) = region {
            self.entries.insert(*id, State::Wanted);
            if !self.regions.contains_key(region.url) {
                self.regions.insert(
                    region.url.to_owned(),
                    RegionQueue {
                        limits: region.limits,
                        get_all: GetAll::NotAsked,
                        next_request: Instant::now(),
                        queue: Vec::new(),
                    },
                );
            }
            if let Some(q) = self.regions.get_mut(region.url) {
                // the SimulatorFeatures may arrive after the first materials
                q.limits = region.limits;
                q.queue.push(*id);
            }
        }
        None
    }

    /// Send the requests whose region throttle expired and collect answers.
    /// Returns true when materials arrived.
    pub fn update(&mut self, rt: &tokio::runtime::Handle, http: &reqwest::Client) -> bool {
        let mut arrived = false;
        while let Ok(answer) = self.rx.try_recv() {
            arrived |= self.receive(answer, Instant::now());
        }
        for req in self.plan(Instant::now()) {
            let (tx, http) = (self.tx.clone(), http.clone());
            rt.spawn(async move {
                let result = fetch(&http, &req).await;
                let _ = tx.send((req, result));
            });
        }
        arrived
    }

    /// Apply one answer. Returns true when materials arrived.
    fn receive(&mut self, (req, result): Answer, now: Instant) -> bool {
        self.in_flight = self.in_flight.saturating_sub(1);
        let mut arrived = false;
        let get_all = req.ids.is_none();
        let asked = req.ids.unwrap_or_default();
        let q = self.regions.get_mut(&req.cap);
        match result {
            Ok(list) => {
                if get_all {
                    log::info!("RenderMaterials: {} materials from the region (GET all)", list.len());
                }
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
                                retry_at: now + UNKNOWN_RETRY,
                            },
                        );
                    }
                }
                if let Some(q) = q
                    && get_all
                {
                    // onGetAllResponse: batches only after the GET-all,
                    // and the throttle restarts once it is answered
                    q.get_all = GetAll::Done;
                    q.next_request = now + q.limits.interval();
                }
            }
            Err(e) => {
                self.errors_logged += 1;
                if self.errors_logged <= 10 {
                    log::warn!(
                        "RenderMaterials {} failed: {e} (region paused {} s)",
                        if get_all { "GET all" } else { "request" },
                        ERROR_BACKOFF.as_secs()
                    );
                }
                for id in asked {
                    if matches!(self.entries.get(&id), Some(State::Fetching)) {
                        self.entries.insert(
                            id,
                            State::Failed {
                                retry_at: now + FAILED_RETRY,
                            },
                        );
                    }
                }
                if let Some(q) = q {
                    if get_all {
                        q.get_all = GetAll::Done;
                    }
                    q.next_request = q.next_request.max(now + ERROR_BACKOFF);
                }
            }
        }
        arrived
    }

    /// LLMaterialMgr::processGetQueue: at most one request per region whose
    /// throttle expired, the GET-all first, then batches of the queued ids;
    /// MAX_IN_FLIGHT requests at once for all regions.
    fn plan(&mut self, now: Instant) -> Vec<Request> {
        // failed entries expire: the next `get` queues them again
        self.entries
            .retain(|_, s| !matches!(s, State::Failed { retry_at } if now >= *retry_at));
        let mut out = Vec::new();
        for (cap, q) in self.regions.iter_mut() {
            if self.in_flight >= MAX_IN_FLIGHT {
                break;
            }
            if now < q.next_request || q.get_all == GetAll::Pending {
                continue;
            }
            // drop the ids answered by the GET-all
            q.queue.retain(|id| matches!(self.entries.get(id), Some(State::Wanted)));
            if q.queue.is_empty() {
                continue;
            }
            let ids = if q.get_all == GetAll::NotAsked {
                q.get_all = GetAll::Pending;
                None
            } else {
                let n = q.queue.len().min(q.limits.max_per_request);
                let ids: Vec<Uuid> = q.queue.drain(..n).collect();
                for id in &ids {
                    self.entries.insert(*id, State::Fetching);
                }
                q.next_request = now + q.limits.interval();
                Some(ids)
            };
            self.in_flight += 1;
            out.push(Request { cap: cap.clone(), ids });
        }
        out
    }
}

/// GET-all (`ids: None`) or POST of a batch of ids.
async fn fetch(http: &reqwest::Client, req: &Request) -> Result<Vec<(Uuid, LegacyMaterial)>, String> {
    let builder = match &req.ids {
        None => http.get(&req.cap),
        Some(ids) => http
            .post(&req.cap)
            .header("Content-Type", "application/llsd+xml")
            .body(aurora_llsd::to_xml(&request_body(ids))),
    };
    let resp = builder
        .header("Accept", "application/llsd+xml")
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let llsd = aurora_llsd::from_xml(&bytes).map_err(|e| e.to_string())?;
    Ok(parse_response(&llsd))
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

    #[test]
    fn region_limits_from_features() {
        assert_eq!(RegionLimits::from_features(None, None), RegionLimits::default());
        let l = RegionLimits::from_features(Some(0.0), Some(0));
        assert_eq!((l.rate, l.max_per_request), (1.0, 50));
        let l = RegionLimits::from_features(Some(4.0), Some(20));
        assert_eq!((l.rate, l.max_per_request), (4.0, 20));
        assert_eq!(l.interval(), Duration::from_millis(250));
    }

    fn region(url: &str) -> RegionCap<'_> {
        RegionCap {
            url,
            limits: RegionLimits::default(),
        }
    }

    #[test]
    fn get_all_first_then_throttled_batches() {
        let mut mats = LegacyMaterials::default();
        let ids: Vec<Uuid> = (1..=120u128).map(Uuid::from_u128).collect();
        for id in &ids {
            assert!(mats.get(id, Some(region("a"))).is_none());
        }
        let t0 = Instant::now();
        // the GET-all goes alone, nothing else to the region while it runs
        let reqs = mats.plan(t0);
        assert_eq!(
            reqs,
            vec![Request {
                cap: "a".into(),
                ids: None
            }]
        );
        assert!(mats.plan(t0 + Duration::from_secs(5)).is_empty());
        // it answers 10 materials; the other 110 go in batches of 50, 1 per second
        let known = ids[..10].iter().map(|id| (*id, LegacyMaterial::from_llsd(&Llsd::Undef))).collect();
        assert!(mats.receive((reqs[0].clone(), Ok(known)), t0));
        assert!(mats.plan(t0).is_empty());
        let mut t = t0;
        let mut sizes = Vec::new();
        for _ in 0..3 {
            t += Duration::from_secs(1);
            let reqs = mats.plan(t);
            assert_eq!(reqs.len(), 1);
            assert!(mats.plan(t).is_empty(), "throttled");
            let asked = reqs[0].ids.clone().expect("batch");
            sizes.push(asked.len());
            mats.receive((reqs[0].clone(), Ok(Vec::new())), t);
        }
        assert_eq!(sizes, vec![50, 50, 10]);
        assert_eq!(mats.counts(), (10, 0, 110));
    }

    #[test]
    fn in_flight_limit_and_error_backoff() {
        let mut mats = LegacyMaterials::default();
        let id_of = |cap: &str| Uuid::from_u128(cap.as_bytes()[0] as u128);
        for cap in ["a", "b", "c"] {
            mats.get(&id_of(cap), Some(region(cap)));
        }
        let t0 = Instant::now();
        let reqs = mats.plan(t0);
        assert_eq!(reqs.len(), MAX_IN_FLIGHT);
        // a 503 pauses its region and frees a slot for the third one
        mats.receive((reqs[0].clone(), Err("HTTP 503 Service Unavailable".into())), t0);
        let third = mats.plan(t0);
        assert_eq!(third.len(), 1);
        assert!(third[0].cap != reqs[0].cap && third[0].cap != reqs[1].cap);
        for r in [&reqs[1], &third[0]] {
            let found = vec![(id_of(&r.cap), LegacyMaterial::from_llsd(&Llsd::Undef))];
            mats.receive((r.clone(), Ok(found)), t0);
        }
        // the failed GET-all falls back to a batch after the pause
        assert!(mats.plan(t0 + Duration::from_secs(5)).is_empty());
        let failed = &reqs[0].cap;
        let later = mats.plan(t0 + ERROR_BACKOFF);
        assert_eq!(
            later,
            vec![Request {
                cap: failed.clone(),
                ids: Some(vec![id_of(failed)])
            }]
        );
    }
}
