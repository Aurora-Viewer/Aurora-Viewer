//! The session task: owns the UDP socket and all simulator circuits,
//! drives login, capabilities, the event queue and agent updates.
//!
//! Message flow follows `newview/llstartup.cpp` and `llviewermessage.cpp`.

mod build_cmds;
mod object_actions;

use crate::caps::{self, EqEvent};
use crate::circuit::Circuit;
use crate::login::{self, LoginError, LoginRequest, LoginResponse};
use crate::objects;
use crate::stats::NetStats;
use crate::terrain;
use crate::types::*;
use aurora_llsd::Llsd;
use aurora_msg::msgs::{self, *};
use aurora_msg::{IncomingPacket, Msg, field_str, parse_packet, str_field};
use glam::Vec3;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Mute list, groups and chat sessions.
#[path = "session_social.rs"]
mod social_net;

/// About Land: parcel selection, lists, covenant (land.rs).
#[path = "session_land.rs"]
mod land_net;
use crate::build::BUILD_PARCEL_SEQ;

const SIM_TIMEOUT: Duration = Duration::from_secs(60);
const AGENT_UPDATE_INTERVAL: Duration = Duration::from_millis(100);
const AGENT_UPDATE_KEEPALIVE: Duration = Duration::from_secs(1);
const PING_INTERVAL: Duration = Duration::from_secs(5);

struct Sim {
    circuit: Circuit,
    handle: RegionHandle,
    name: String,
    size: (u32, u32),
    seed: Option<String>,
    caps_requested: bool,
    caps: Arc<HashMap<String, String>>,
    eq_task: Option<JoinHandle<()>>,
    last_ping: Instant,
    time_dilation: f32,
    /// UseCircuitCode sequence; CompleteAgentMovement waits for its ack.
    ucc_seq: Option<u32>,
    pending_cam: bool,
    ucc_sent_at: Instant,
}

impl Sim {
    fn new(addr: SocketAddr, handle: RegionHandle) -> Sim {
        Sim {
            circuit: Circuit::new(addr),
            handle,
            name: String::new(),
            size: (256, 256),
            seed: None,
            caps_requested: false,
            caps: Arc::new(HashMap::new()),
            eq_task: None,
            last_ping: Instant::now(),
            time_dilation: 1.0,
            ucc_seq: None,
            pending_cam: false,
            ucc_sent_at: Instant::now(),
        }
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        if let Some(t) = self.eq_task.take() {
            t.abort();
        }
    }
}

pub(crate) struct Shared {
    pub http: reqwest::Client,
    pub caps_http: reqwest::Client,
    pub events: crossbeam_channel::Sender<NetEvent>,
    pub controls: Arc<Mutex<AgentControls>>,
    pub stats: Arc<NetStats>,
    /// Directory of the per-region object caches (None = disabled).
    pub object_cache_dir: Arc<Mutex<Option<std::path::PathBuf>>>,
}

struct Session<'a> {
    sh: &'a Shared,
    sock: Arc<UdpSocket>,
    login: Arc<LoginResponse>,
    sims: HashMap<SocketAddr, Sim>,
    main: Option<SocketAddr>,
    eq_tx: mpsc::UnboundedSender<EqEvent>,
    caps_tx: mpsc::UnboundedSender<(SocketAddr, Result<HashMap<String, String>, String>)>,
    last_controls: AgentControls,
    last_agent_update: Instant,
    one_shot: u32,
    /// "No agent updates" already reported for this CompleteAgentMovement.
    cam_warned: bool,
    far: f32,
    logout_started: Option<Instant>,
    finished: bool,
    agent_moved: bool,
    cam_requested_at: Option<Instant>,
    /// Agent parcel in the main region: (local id, environment version).
    agent_parcel: Option<(i32, i32)>,
    /// Teleport by region name waiting for its MapBlockReply (lowercase name, position).
    pending_named_tp: Option<(String, Vec3)>,
    obj_cache_dir: Option<std::path::PathBuf>,
    obj_caches: HashMap<SocketAddr, crate::objcache::RegionObjectCache>,
    last_cache_save: Instant,
    /// Mute list download, groups, chat sessions (session_social.rs).
    social: social_net::SocialNet,
    tasks: object_actions::TaskRequests,
    /// Covenant transfer of the About Land floater (session_land.rs).
    land: land_net::LandNet,
}

fn emit(sh: &Shared, ev: NetEvent) {
    let _ = sh.events.send(ev);
}

fn ip_from_llsd(v: &Llsd) -> Option<Ipv4Addr> {
    match v {
        Llsd::Binary(b) if b.len() == 4 => Some(Ipv4Addr::new(b[0], b[1], b[2], b[3])),
        Llsd::String(s) => s.parse().ok(),
        Llsd::Integer(i) => Some(Ipv4Addr::from((*i as u32).to_be_bytes())),
        _ => None,
    }
}

/// Run one complete session (login .. logout/disconnect).
pub(crate) async fn run_session(sh: &Shared, req: LoginRequest, cmd_rx: &mut mpsc::UnboundedReceiver<NetCommand>) {
    emit(
        sh,
        NetEvent::LoginProgress {
            message: "Connexion au serveur de login…".into(),
            fraction: 0.1,
        },
    );
    let login = match login::login(&sh.http, &req).await {
        Ok(l) => Arc::new(l),
        Err(e) => {
            let (mfa, tos) = (matches!(e, LoginError::MfaRequired(_)), matches!(e, LoginError::TosRequired(_)));
            let bad_credentials = matches!(&e, LoginError::Refused { reason, .. } if reason == "key");
            emit(
                sh,
                NetEvent::LoginFailed {
                    error: e.to_string(),
                    mfa_required: mfa,
                    tos_required: tos,
                    bad_credentials,
                },
            );
            return;
        }
    };
    emit(sh, NetEvent::LoggedIn(login.clone()));
    emit(
        sh,
        NetEvent::LoginProgress {
            message: "Connexion à la région…".into(),
            fraction: 0.3,
        },
    );

    let sock = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            emit(
                sh,
                NetEvent::LoginFailed {
                    error: format!("UDP bind failed: {e}"),
                    mfa_required: false,
                    tos_required: false,
                    bad_credentials: false,
                },
            );
            return;
        }
    };

    let (eq_tx, mut eq_rx) = mpsc::unbounded_channel();
    let (caps_tx, mut caps_rx) = mpsc::unbounded_channel();
    let mut s = Session {
        sh,
        sock: sock.clone(),
        login: login.clone(),
        sims: HashMap::new(),
        main: None,
        eq_tx,
        caps_tx,
        last_controls: AgentControls::default(),
        last_agent_update: Instant::now(),
        one_shot: 0,
        cam_warned: false,
        far: 128.0,
        logout_started: None,
        finished: false,
        agent_moved: false,
        cam_requested_at: None,
        agent_parcel: None,
        pending_named_tp: None,
        obj_cache_dir: sh.object_cache_dir.lock().clone(),
        obj_caches: HashMap::new(),
        last_cache_save: Instant::now(),
        social: Default::default(),
        tasks: Default::default(),
        land: Default::default(),
    };

    let addr = SocketAddr::V4(SocketAddrV4::new(login.sim_ip, login.sim_port));
    s.add_sim(addr, login.region_handle(), (login.region_size_x, login.region_size_y));
    s.set_main(addr, Some(login.seed_capability.clone()));
    s.complete_agent_movement(addr);

    let mut buf = vec![0u8; 16384];
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while !s.finished {
        tokio::select! {
            r = sock.recv_from(&mut buf) => {
                match r {
                    Ok((n, from)) => s.on_datagram(&buf[..n], from),
                    Err(e) => {
                        // Windows reports ICMP port unreachable as recv errors.
                        log::debug!("udp recv error: {e}");
                    }
                }
            }
            _ = tick.tick() => s.tick(),
            c = cmd_rx.recv() => match c {
                Some(c) => s.on_command(c),
                None => break,
            },
            Some(e) = eq_rx.recv() => s.on_eq_event(e),
            Some((addr, caps)) = caps_rx.recv() => s.on_caps(addr, caps),
        }
    }
    s.save_object_caches();
    // Close every circuit politely.
    for (addr, sim) in s.sims.iter_mut() {
        let m = CloseCircuit::default();
        let _ = sim.circuit.send(&s.sock, &s.sh.stats, &m, false);
        log::debug!("closed circuit {addr}");
    }
    sh.stats.sim_count.store(0, Ordering::Relaxed);
    *sh.stats.simulator_motion.lock() = Default::default();
}

impl Session<'_> {
    fn agent_id(&self) -> uuid::Uuid {
        self.login.agent_id
    }
    fn session_id(&self) -> uuid::Uuid {
        self.login.session_id
    }

    fn send<M: Msg>(&mut self, addr: SocketAddr, msg: &M, reliable: bool) {
        if let Some(sim) = self.sims.get_mut(&addr) {
            let _ = sim.circuit.send(&self.sock, &self.sh.stats, msg, reliable);
        }
    }

    fn send_main<M: Msg>(&mut self, msg: &M, reliable: bool) {
        if let Some(a) = self.main {
            self.send(a, msg, reliable);
        }
    }

    fn add_sim(&mut self, addr: SocketAddr, handle: RegionHandle, size: (u32, u32)) {
        if self.sims.contains_key(&addr) {
            return;
        }
        let mut sim = Sim::new(addr, handle);
        sim.size = size;
        self.sims.insert(addr, sim);
        let mut m = UseCircuitCode::default();
        m.circuit_code.code = self.login.circuit_code;
        m.circuit_code.session_id = self.session_id();
        m.circuit_code.id = self.agent_id();
        if let Some(sim) = self.sims.get_mut(&addr) {
            let seq = sim.circuit.send(&self.sock, &self.sh.stats, &m, true);
            sim.ucc_seq = Some(seq);
            sim.ucc_sent_at = Instant::now();
        }
        self.sh.stats.sim_count.store(self.sims.len() as u32, Ordering::Relaxed);
    }

    fn set_main(&mut self, addr: SocketAddr, seed: Option<String>) {
        let changed = self.main != Some(addr);
        self.main = Some(addr);
        if changed {
            self.agent_parcel = None;
            // capabilities of a neighbour we already know: no new caps event
            if self.sims.get(&addr).is_some_and(|s| !s.caps.is_empty()) {
                self.request_environment(None);
            }
        }
        let mut handle = 0;
        if let Some(sim) = self.sims.get_mut(&addr) {
            if seed.is_some() {
                sim.seed = seed;
            }
            handle = sim.handle;
        }
        self.publish_motion();
        self.request_caps(addr);
        emit(self.sh, NetEvent::MainRegionChanged { handle });
    }

    /// Queue CompleteAgentMovement until UseCircuitCode is acknowledged
    /// (llstartup.cpp waits for gGotUseCircuitCodeAck).
    fn complete_agent_movement(&mut self, addr: SocketAddr) {
        if let Some(sim) = self.sims.get_mut(&addr) {
            sim.pending_cam = true;
        }
        self.cam_requested_at = Some(Instant::now());
        self.cam_warned = false;
    }

    fn send_complete_agent_movement(&mut self, addr: SocketAddr) {
        let mut m = CompleteAgentMovement::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        m.agent_data.circuit_code = self.login.circuit_code;
        self.send(addr, &m, true);

        // Throttle: resend, land, wind, cloud, task, texture, asset (bits/s)
        let throttles: [f32; 7] = [150_000.0, 170_000.0, 34_000.0, 34_000.0, 800_000.0, 300_000.0, 300_000.0];
        let mut t = AgentThrottle::default();
        t.agent_data.agent_id = self.agent_id();
        t.agent_data.session_id = self.session_id();
        t.agent_data.circuit_code = self.login.circuit_code;
        t.throttle.gen_counter = 0;
        t.throttle.throttles = throttles.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.send(addr, &t, true);

        let mut hw = AgentHeightWidth::default();
        hw.agent_data.agent_id = self.agent_id();
        hw.agent_data.session_id = self.session_id();
        hw.agent_data.circuit_code = self.login.circuit_code;
        hw.height_width_block.height = 1080;
        hw.height_width_block.width = 1920;
        self.send(addr, &hw, true);
    }

    fn request_caps(&mut self, addr: SocketAddr) {
        let Some(sim) = self.sims.get_mut(&addr) else {
            return;
        };
        let Some(seed) = sim.seed.clone() else {
            return;
        };
        if sim.caps_requested || seed.is_empty() {
            return;
        }
        sim.caps_requested = true;
        let http = self.sh.caps_http.clone();
        let tx = self.caps_tx.clone();
        tokio::spawn(async move {
            let r = caps::resolve_seed(&http, &seed).await.map_err(|e| e.to_string());
            let _ = tx.send((addr, r));
        });
    }

    fn on_caps(&mut self, addr: SocketAddr, r: Result<HashMap<String, String>, String>) {
        let caps = match r {
            Ok(c) => c,
            Err(e) => {
                log::warn!("capabilities for {addr} failed: {e}");
                if self.main == Some(addr) {
                    emit(
                        self.sh,
                        NetEvent::Alert {
                            message: format!(
                                "Les capacités de la région sont indisponibles ({e}) : textures, inventaire et téléportations seront limités."
                            ),
                        },
                    );
                }
                return;
            }
        };
        let Some(sim) = self.sims.get_mut(&addr) else {
            return;
        };
        log::info!("{} capabilities for {} ({})", caps.len(), addr, sim.name);
        if let Some(eq) = caps.get("EventQueueGet").cloned() {
            if let Some(t) = sim.eq_task.take() {
                t.abort();
            }
            sim.eq_task = Some(tokio::spawn(caps::event_queue_loop(
                self.sh.caps_http.clone(),
                eq,
                addr,
                self.eq_tx.clone(),
            )));
        }
        let caps = Arc::new(caps);
        sim.caps = caps.clone();
        emit(self.sh, NetEvent::Capabilities { handle: sim.handle, caps });
        let features = sim.caps.get("SimulatorFeatures").cloned().map(|u| (sim.handle, u));
        if let Some((handle, url)) = features {
            self.request_features(handle, url);
        }
        if self.main == Some(addr) {
            // LLEnvironment::onRegionChange -> requestRegion once caps arrive
            self.request_environment(None);
        }
    }

    /// SimulatorFeatures (LLViewerRegion::requestSimulatorFeatures): the voice
    /// server type and the RenderMaterials limits are used for now.
    fn request_features(&self, handle: RegionHandle, url: String) {
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(bytes) = resp.bytes().await
                        && let Ok(v) = aurora_llsd::from_xml(&bytes)
                    {
                        let voice_server_type = v["VoiceServerType"].to_string_value();
                        let rate = &v["RenderMaterialsCapability"];
                        let materials_rate = (!rate.is_undef()).then(|| rate.as_f32());
                        let max = &v["MaxMaterialsPerTransaction"];
                        let materials_max = (!max.is_undef()).then(|| max.as_u32());
                        let _ = events.send(NetEvent::SimulatorFeatures {
                            handle,
                            voice_server_type,
                            materials_rate,
                            materials_max,
                        });
                    }
                }
                Ok(resp) => log::info!("SimulatorFeatures: HTTP {}", resp.status()),
                Err(e) => log::info!("SimulatorFeatures failed: {e}"),
            }
        });
    }

    /// Fetch the EEP environment of the main region, or of one of its parcels
    /// (LLEnvironment::coroRequestEnvironment: `ExtEnvironment?parcelid=N`).
    fn request_environment(&mut self, parcel: Option<i32>) {
        let Some(sim) = self.main.and_then(|a| self.sims.get(&a)) else {
            return;
        };
        let Some(mut url) = sim.caps.get("ExtEnvironment").cloned() else {
            return;
        };
        if let Some(p) = parcel {
            url.push_str(&format!("?parcelid={p}"));
        }
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let handle = sim.handle;
        // the answer carries what was asked: requests are independent tasks
        // whose answers may arrive out of order (the viewer drops stale ones,
        // as LLEnvironment::recordEnvironment does)
        let parcel_id = parcel.unwrap_or(-1);
        tokio::spawn(async move {
            match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(bytes) = resp.bytes().await
                        && let Ok(v) = aurora_llsd::from_xml(&bytes)
                    {
                        // coroRequestEnvironment: an answer without "environment" is ignored
                        let environment = &v["environment"];
                        if environment.is_undef() {
                            log::info!("ExtEnvironment (parcel {parcel_id}): no environment in the answer");
                            return;
                        }
                        let _ = events.send(NetEvent::Environment {
                            handle,
                            parcel_id,
                            environment: environment.clone(),
                        });
                    }
                }
                Ok(resp) => log::info!("ExtEnvironment: HTTP {}", resp.status()),
                Err(e) => log::info!("ExtEnvironment failed: {e}"),
            }
        });
    }

    /// ParcelProperties (event queue): request the parcel environment when
    /// the agent enters another parcel or its environment version changes
    /// (LLViewerParcelMgr::processParcelProperties).
    fn on_parcel_properties(&mut self, sim: SocketAddr, b: &Llsd) {
        // COLLISION_NOT_IN_GROUP / BANNED / NOT_ON_LIST_PARCEL_SEQ_ID: ban lines
        let seq = b["ParcelData"][0]["SequenceID"].as_i32();
        if matches!(seq, -20000 | -30000 | -40000) {
            let pd = &b["ParcelData"][0];
            let flags = match &pd["ParcelFlags"] {
                Llsd::Binary(v) if v.len() == 4 => u32::from_be_bytes([v[0], v[1], v[2], v[3]]),
                other => other.as_i32() as u32,
            };
            let bitmap = match &pd["Bitmap"] {
                Llsd::Binary(v) => v.clone(),
                _ => Vec::new(),
            };
            emit(
                self.sh,
                NetEvent::ParcelCollision {
                    handle: self.handle_of(sim),
                    kind: match seq {
                        -30000 => 1,
                        -20000 => 2,
                        _ => 3,
                    },
                    use_pass: flags & (1 << 11) != 0, // PF_USE_PASS_LIST
                    bitmap,
                },
            );
            return;
        }
        let info = crate::land::parse_parcel_properties(b);
        if info.sequence_id == crate::land::SELECTED_PARCEL_SEQ_ID {
            // the About Land selection, in whichever region it lies
            if info.request_result != crate::land::PARCEL_RESULT_NO_DATA {
                let handle = self.handle_of(sim);
                let result = info.request_result;
                emit(
                    self.sh,
                    NetEvent::Land(crate::land::LandEvent::Selected {
                        handle,
                        info: Arc::new(info),
                        result,
                    }),
                );
            }
            return;
        }
        if info.sequence_id >= BUILD_PARCEL_SEQ {
            // the build tools' requests (capacity line, land tool), any region
            let handle = self.handle_of(sim);
            let sequence = info.sequence_id;
            emit(
                self.sh,
                NetEvent::SelectedParcel {
                    handle,
                    sequence,
                    parcel: Arc::new(info),
                },
            );
            return;
        }
        if self.main != Some(sim) || info.sequence_id < 0 {
            // other regions, hover requests
            return;
        }
        let (local_id, version) = (info.local_id, info.env_version);
        emit(self.sh, NetEvent::AgentParcel(Arc::new(info)));
        if self.agent_parcel != Some((local_id, version)) {
            self.agent_parcel = Some((local_id, version));
            self.request_environment(Some(local_id));
        }
    }

    fn save_object_caches(&mut self) {
        for c in self.obj_caches.values_mut() {
            c.save();
        }
    }

    fn remove_sim(&mut self, addr: SocketAddr) {
        if let Some(mut c) = self.obj_caches.remove(&addr) {
            c.save();
        }
        if let Some(sim) = self.sims.remove(&addr) {
            emit(self.sh, NetEvent::RegionRemoved { handle: sim.handle });
        }
        if self.main == Some(addr) {
            self.main = None;
        }
        self.sh.stats.sim_count.store(self.sims.len() as u32, Ordering::Relaxed);
    }

    fn publish_motion(&self) {
        *self.sh.stats.simulator_motion.lock() = self
            .main
            .and_then(|addr| self.sims.get(&addr))
            .map_or_else(Default::default, |sim| crate::stats::SimulatorMotion {
                handle: Some(sim.handle),
                time_dilation: sim.time_dilation,
                last_packet: Some(sim.circuit.last_recv),
            });
    }

    // LLViewerObject::processUpdateMessage: U16 region time dilation is
    // scaled by 65535, and belongs to the sending simulator, not every circuit.
    fn set_time_dilation(&mut self, from: SocketAddr, value: u16) {
        if let Some(sim) = self.sims.get_mut(&from) {
            sim.time_dilation = value as f32 / 65535.0;
        }
        self.publish_motion();
    }

    // ----------------------------------------------------------------- tick

    fn tick(&mut self) {
        self.expire_task_inventory();
        let now = Instant::now();
        self.publish_motion();
        if self.last_cache_save.elapsed() > Duration::from_secs(120) {
            self.last_cache_save = Instant::now();
            for c in self.obj_caches.values_mut() {
                c.save();
            }
        }
        let stats = self.sh.stats.clone();
        // Deferred CompleteAgentMovement once UseCircuitCode is acked (or 2 s passed).
        let ready: Vec<SocketAddr> = self
            .sims
            .iter()
            .filter(|(_, s)| {
                s.pending_cam && (s.ucc_seq.is_none_or(|q| !s.circuit.is_unacked(q)) || s.ucc_sent_at.elapsed() > Duration::from_secs(2))
            })
            .map(|(a, _)| *a)
            .collect();
        for addr in ready {
            if let Some(sim) = self.sims.get_mut(&addr) {
                sim.pending_cam = false;
            }
            self.send_complete_agent_movement(addr);
        }
        if let Some(t) = self.cam_requested_at {
            // diagnostic: no AgentUpdate is sent until AgentMovementComplete
            if !self.agent_moved && !self.cam_warned && t.elapsed() > Duration::from_secs(5) {
                self.cam_warned = true;
                log::warn!("no AgentMovementComplete 5 s after CompleteAgentMovement: agent updates (movement) paused");
            }
            if !self.agent_moved && t.elapsed() > Duration::from_secs(90) {
                emit(
                    self.sh,
                    NetEvent::Disconnected {
                        reason: "La région n'a pas répondu à temps (AgentMovementComplete).".into(),
                    },
                );
                self.finished = true;
                return;
            }
        }
        let mut unacked = 0;
        let mut ping_targets = Vec::new();
        let mut dead = Vec::new();
        for (addr, sim) in self.sims.iter_mut() {
            sim.circuit.tick(&self.sock, &stats);
            unacked += sim.circuit.unacked_count();
            if now.duration_since(sim.last_ping) > PING_INTERVAL {
                sim.last_ping = now;
                ping_targets.push(*addr);
            }
            if now.duration_since(sim.circuit.last_recv) > SIM_TIMEOUT {
                dead.push(*addr);
            }
        }
        stats.unacked.store(unacked as u32, Ordering::Relaxed);
        for addr in ping_targets {
            if let Some(sim) = self.sims.get_mut(&addr) {
                let (id, oldest) = sim.circuit.next_ping();
                let mut m = StartPingCheck::default();
                m.ping_id.ping_id = id;
                m.ping_id.oldest_unacked = oldest;
                let _ = sim.circuit.send(&self.sock, &stats, &m, false);
            }
        }
        for addr in dead {
            let was_main = self.main == Some(addr);
            log::warn!("simulator {addr} timed out");
            self.remove_sim(addr);
            if was_main {
                emit(
                    self.sh,
                    NetEvent::Disconnected {
                        reason: "La connexion avec la région a été perdue.".into(),
                    },
                );
                self.finished = true;
                return;
            }
        }

        if let Some(t) = self.logout_started
            && t.elapsed() > Duration::from_secs(6)
        {
            emit(self.sh, NetEvent::LoggedOut);
            self.finished = true;
            return;
        }

        if self.agent_moved && self.logout_started.is_none() {
            self.maybe_send_agent_update(now);
        }
    }

    fn maybe_send_agent_update(&mut self, now: Instant) {
        let mut c = *self.sh.controls.lock();
        c.far = self.far;
        c.control_flags |= self.one_shot;
        let since = now.duration_since(self.last_agent_update);
        let changed = c.significantly_differs(&self.last_controls);
        if (changed && since >= AGENT_UPDATE_INTERVAL) || since >= AGENT_UPDATE_KEEPALIVE || self.one_shot != 0 {
            let mut m = AgentUpdate::default();
            let a = &mut m.agent_data;
            a.agent_id = self.agent_id();
            a.session_id = self.session_id();
            a.body_rotation = c.body_rotation;
            a.head_rotation = c.head_rotation;
            a.state = c.state;
            a.camera_center = c.camera_center;
            a.camera_at_axis = c.camera_at;
            a.camera_left_axis = c.camera_left;
            a.camera_up_axis = c.camera_up;
            a.far = c.far;
            a.control_flags = c.control_flags;
            a.flags = c.flags;
            self.send_main(&m, false);
            self.last_controls = c;
            self.last_agent_update = now;
            self.one_shot = 0;
        }
    }

    // -------------------------------------------------------------- commands

    fn on_command(&mut self, c: NetCommand) {
        if self.on_object_action_command(&c) || self.on_social_command(&c) {
            return;
        }
        match c {
            NetCommand::Login(_) => log::warn!("already logged in"),
            NetCommand::Logout => {
                if self.logout_started.is_none() {
                    let mut m = LogoutRequest::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    self.send_main(&m, true);
                    self.logout_started = Some(Instant::now());
                }
            }
            NetCommand::Chat {
                message,
                channel,
                chat_type,
            } => {
                // SL limits chat to 1023 bytes; split longer messages.
                let mut rest = message.as_str();
                while !rest.is_empty() {
                    let mut cut = rest.len().min(1000);
                    while !rest.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    let (part, r) = rest.split_at(cut);
                    rest = r;
                    let mut m = ChatFromViewer::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.chat_data.message = str_field(part);
                    m.chat_data.type_ = chat_type.to_u8();
                    m.chat_data.channel = channel;
                    self.send_main(&m, true);
                }
            }
            NetCommand::RequestObjects { handle, local_ids } => {
                if let Some(addr) = self.sim_for_handle(handle) {
                    self.request_objects(addr, &local_ids);
                }
            }
            NetCommand::TeleportHome => {
                let mut m = TeleportLandmarkRequest::default();
                m.info.agent_id = self.agent_id();
                m.info.session_id = self.session_id();
                m.info.landmark_id = uuid::Uuid::nil();
                self.send_main(&m, true);
            }
            NetCommand::TeleportTo { handle, position, look_at } => {
                let mut m = TeleportLocationRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.info.region_handle = handle;
                m.info.position = position;
                m.info.look_at = look_at;
                self.send_main(&m, true);
            }
            NetCommand::TeleportToRegion { name, position } => {
                // LLWorldMapMessage::sendNamedRegionRequest: the reply gives the grid position
                self.pending_named_tp = Some((name.trim().to_lowercase(), position));
                let mut m = MapNameRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.agent_data.flags = 2; // LAYER_FLAG
                m.name_data.name = str_field(name.trim());
                self.send_main(&m, true);
            }
            NetCommand::SetDrawDistance(d) => self.far = d.clamp(32.0, 1024.0),
            NetCommand::SendImDialog {
                to,
                dialog,
                id,
                message,
                bucket,
            } => {
                let mut m = ImprovedInstantMessage::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                let b = &mut m.message_block;
                b.to_agent_id = to;
                b.dialog = dialog;
                b.id = id;
                b.from_agent_name = str_field(&format!("{} {}", self.login.first_name, self.login.last_name));
                b.message = str_field(&message);
                b.binary_bucket = if bucket.is_empty() { vec![0] } else { bucket };
                self.send_main(&m, true);
            }
            NetCommand::AcceptLure { lure_id } => {
                let mut m = TeleportLureRequest::default();
                m.info.agent_id = self.agent_id();
                m.info.session_id = self.session_id();
                m.info.lure_id = lure_id;
                m.info.teleport_flags = 1 << 2; // TELEPORT_FLAGS_VIA_LURE
                self.send_main(&m, true);
            }
            NetCommand::AcceptFriendship { transaction, folder } => {
                let mut m = AcceptFriendship::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.transaction_block.transaction_id = transaction;
                m.folder_data = vec![accept_friendship::FolderData { folder_id: folder }];
                self.send_main(&m, true);
            }
            NetCommand::DeclineFriendship { transaction } => {
                let mut m = DeclineFriendship::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.transaction_block.transaction_id = transaction;
                self.send_main(&m, true);
            }
            NetCommand::ScriptDialogReply {
                object_id,
                channel,
                index,
                label,
            } => {
                let mut m = ScriptDialogReply::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.data.object_id = object_id;
                m.data.chat_channel = channel;
                m.data.button_index = index;
                m.data.button_label = str_field(&label);
                self.send_main(&m, true);
            }
            NetCommand::ScriptAnswer {
                task_id,
                item_id,
                questions,
            } => {
                let mut m = ScriptAnswerYes::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.data.task_id = task_id;
                m.data.item_id = item_id;
                m.data.questions = questions;
                self.send_main(&m, true);
            }
            NetCommand::Touch { local_id } => {
                // a click is a grab immediately followed by a release (LLToolGrab)
                let mut g = ObjectGrab::default();
                g.agent_data.agent_id = self.agent_id();
                g.agent_data.session_id = self.session_id();
                g.object_data.local_id = local_id;
                g.object_data.grab_offset = Vec3::ZERO;
                self.send_main(&g, true);
                let mut d = ObjectDeGrab::default();
                d.agent_data.agent_id = self.agent_id();
                d.agent_data.session_id = self.session_id();
                d.object_data.local_id = local_id;
                self.send_main(&d, true);
            }
            NetCommand::RequestSit { handle, target, offset } => {
                // handle_object_sit (Firestorm llviewermenu.cpp) sends to the
                // object's region, which can differ from the agent's region.
                if let Some(addr) = self.sim_for_handle(handle) {
                    let mut m = AgentRequestSit::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.target_object.target_id = target;
                    m.target_object.offset = offset;
                    self.send(addr, &m, true);
                }
            }
            NetCommand::RequestObjectProperties { handle, object } => {
                if let Some(addr) = self.sim_for_handle(handle) {
                    let mut m = RequestObjectPropertiesFamily::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.object_data.object_id = object;
                    self.send(addr, &m, true);
                }
            }
            NetCommand::RequestPayPrice { handle, object } => {
                if let Some(addr) = self.sim_for_handle(handle) {
                    let mut m = RequestPayPrice::default();
                    m.object_data.object_id = object;
                    self.send(addr, &m, true);
                }
            }
            NetCommand::BuyObject {
                handle,
                local_id,
                folder,
                sale_type,
                price,
            } => {
                // Port of LLSelectMgr::sendBuy / packBuyObjectIDs (llselectmgr.cpp).
                if (1..=3).contains(&sale_type)
                    && price >= 0
                    && !folder.is_nil()
                    && let Some(addr) = self.sim_for_handle(handle)
                {
                    let mut m = ObjectBuy::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.agent_data.category_id = folder;
                    m.object_data.push(object_buy::ObjectData {
                        object_local_id: local_id,
                        sale_type,
                        sale_price: price,
                    });
                    self.send(addr, &m, true);
                }
            }
            NetCommand::PayObject {
                handle,
                object,
                amount,
                description,
            } => {
                // Port of give_money (llviewermessage.cpp): TRANS_PAY_OBJECT,
                // no group transfer flags, destination is the actual clicked prim.
                if amount > 0
                    && !object.is_nil()
                    && let Some(addr) = self.sim_for_handle(handle)
                {
                    let mut m = MoneyTransferRequest::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.money_data.source_id = self.agent_id();
                    m.money_data.dest_id = object;
                    m.money_data.amount = amount;
                    m.money_data.transaction_type = 5008;
                    m.money_data.description = str_field(&description);
                    self.send(addr, &m, true);
                }
            }
            NetCommand::RequestProfile(id) => {
                let cap = self
                    .main
                    .and_then(|a| self.sims.get(&a))
                    .and_then(|s| s.caps.get("AgentProfile").cloned());
                match cap {
                    Some(url) => {
                        // LLAvatarPropertiesProcessor: GET <AgentProfile>/<avatar id>
                        let http = self.sh.caps_http.clone();
                        let events = self.sh.events.clone();
                        tokio::spawn(async move {
                            let url = format!("{}/{id}", url.trim_end_matches('/'));
                            match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                                Ok(resp) if resp.status().is_success() => {
                                    if let Ok(v) = resp
                                        .bytes()
                                        .await
                                        .map_err(|_| ())
                                        .and_then(|b| aurora_llsd::from_xml(&b).map_err(|_| ()))
                                    {
                                        match crate::profile::parse_agent_profile(id, &v) {
                                            Some(p) => {
                                                let _ = events.send(NetEvent::AvatarProfile(Box::new(p)));
                                            }
                                            None => log::debug!("AgentProfile {id}: reply about another avatar"),
                                        }
                                    }
                                }
                                Ok(resp) => log::debug!("AgentProfile {id}: HTTP {}", resp.status()),
                                Err(e) => log::debug!("AgentProfile {id}: {e}"),
                            }
                        });
                    }
                    None => {
                        let mut m = AvatarPropertiesRequest::default();
                        m.agent_data.agent_id = self.agent_id();
                        m.agent_data.session_id = self.session_id();
                        m.agent_data.avatar_id = id;
                        self.send_main(&m, true);
                    }
                }
            }
            NetCommand::UpdateProfile { target, data } => self.update_profile(target, data),
            NetCommand::PickInfoRequest { creator, pick } => {
                // LLAvatarPropertiesProcessor::sendPickInfoRequest
                self.send_generic("pickinforequest", &[creator.to_string(), pick.to_string()]);
            }
            NetCommand::PickUpdate(pick) => {
                // LLPanelProfilePick::sendUpdate
                let mut m = PickInfoUpdate::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                let d = &mut m.data;
                d.pick_id = pick.id;
                d.creator_id = self.agent_id();
                d.top_pick = false;
                d.parcel_id = pick.parcel;
                d.name = str_field(&pick.name);
                d.desc = str_field(&pick.desc);
                d.snapshot_id = pick.snapshot;
                d.pos_global = pick.pos_global;
                d.sort_order = 0;
                d.enabled = true;
                self.send_main(&m, true);
            }
            NetCommand::PickDelete(id) => {
                let mut m = PickDelete::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.data.pick_id = id;
                self.send_main(&m, true);
            }
            NetCommand::ClassifiedsRequest(id) => self.send_generic("avatarclassifiedsrequest", &[id.to_string()]),
            NetCommand::ClassifiedInfoRequest(id) => {
                let mut m = ClassifiedInfoRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.data.classified_id = id;
                self.send_main(&m, true);
            }
            NetCommand::ClassifiedDelete(id) => {
                let mut m = ClassifiedDelete::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.data.classified_id = id;
                self.send_main(&m, true);
            }
            NetCommand::Land(l) => self.on_land_command(l),
            NetCommand::ParcelInfoRequest(id) => {
                let mut m = ParcelInfoRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.data.parcel_id = id;
                self.send_main(&m, true);
            }
            NetCommand::GrantUserRights { friend, rights } => {
                // LLAvatarActions / LLAvatarTracker::sendRightsGrantedUpdate
                let mut m = GrantUserRights::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.rights.push(msgs::grant_user_rights::Rights {
                    agent_related: friend,
                    related_rights: rights,
                });
                self.send_main(&m, true);
            }
            NetCommand::LookAt {
                effect,
                target,
                offset,
                kind,
                duration,
            } => {
                let mut m = ViewerEffect::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.effect
                    .push(lookat_effect(self.agent_id(), effect, target, offset, kind, duration));
                self.send_main(&m, false);
            }
            NetCommand::SetAlwaysRun(run) => {
                let mut m = SetAlwaysRun::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.agent_data.always_run = run;
                self.send_main(&m, true);
            }
            NetCommand::RequestNames(ids) => {
                for chunk in ids.chunks(64) {
                    let m = UUIDNameRequest {
                        uuid_name_block: chunk.iter().map(|id| uuid_name_request::UUIDNameBlock { id: *id }).collect(),
                    };
                    self.send_main(&m, true);
                }
            }
            NetCommand::RequestDisplayNames(ids) => self.request_display_names(ids),
            NetCommand::SetDisplayName { old, new } => self.set_display_name(old, new),
            NetCommand::SendIm { to, message } => self.send_im(to, &message, 0),
            NetCommand::OfferTeleport { to, message } => {
                let mut m = StartLure::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.info.lure_type = 0;
                m.info.message = str_field(&message);
                m.target_data.push(start_lure::TargetData { target_id: to });
                self.send_main(&m, true);
            }
            NetCommand::MapBlockRequest {
                min_x,
                min_y,
                max_x,
                max_y,
                null_sims,
            } => {
                // LLWorldMapMessage::sendMapBlockRequest
                let mut m = MapBlockRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.agent_data.flags = if null_sims { 0x0001_0000 } else { 2 }; // MAP_SIM_RETURN_NULL_SIMS / LAYER_FLAG
                m.position_data.min_x = min_x;
                m.position_data.min_y = min_y;
                m.position_data.max_x = max_x;
                m.position_data.max_y = max_y;
                self.send_main(&m, true);
            }
            NetCommand::MapNameRequest { name } => {
                let mut m = MapNameRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.agent_data.flags = 2; // LAYER_FLAG
                m.name_data.name = str_field(name.trim());
                self.send_main(&m, true);
            }
            NetCommand::MapItemRequest { item_type, handle } => {
                let mut m = MapItemRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.agent_data.flags = 2; // LAYER_FLAG
                m.request_data.item_type = item_type;
                m.request_data.region_handle = handle;
                self.send_main(&m, true);
            }
            NetCommand::TeleportLandmark(asset) => {
                let mut m = TeleportLandmarkRequest::default();
                m.info.agent_id = self.agent_id();
                m.info.session_id = self.session_id();
                m.info.landmark_id = asset;
                self.send_main(&m, true);
            }
            NetCommand::FetchInventory { folders, owner, library } => self.fetch_inventory(folders, owner, library),
            NetCommand::FetchItems { items, owner } => self.fetch_items(items, owner),
            NetCommand::RezAttachments(list) => {
                // one object per message: batching makes the servers drop
                // attachments (LLAttachmentsMgr, FIRE-6070)
                for a in list {
                    let mut m = RezMultipleAttachmentsFromInv::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.header_data.compound_msg_id = uuid::Uuid::new_v4();
                    m.header_data.total_objects = 1;
                    m.header_data.first_detach_all = false;
                    m.object_data = vec![rez_multiple_attachments_from_inv::ObjectData {
                        item_id: a.item_id,
                        owner_id: a.owner_id,
                        attachment_pt: a.point | if a.add { 0x80 } else { 0 },
                        item_flags: a.flags,
                        group_mask: a.group_mask,
                        everyone_mask: a.everyone_mask,
                        next_owner_mask: a.next_owner_mask,
                        name: str_field(&a.name),
                        description: str_field(&a.desc),
                    }];
                    self.send_main(&m, true);
                }
            }
            NetCommand::RequestServerAppearance { cof_version } => self.request_server_appearance(cof_version),
            NetCommand::DummyWearablesUpdate => {
                // the 4 standard placeholder ids LL's viewer sends
                let mut m = AgentIsNowWearing::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.wearable_data = [
                    (1u8, "db5a4e5f-9da3-44c8-992d-1181c5795498"),
                    (2, "6969c7cc-f72f-4a76-a19b-c293cce8ce4f"),
                    (3, "7999702b-b291-48f9-8903-c91dfb828408"),
                    (4, "566cb59e-ef60-41d7-bfa6-e0f293fbea40"),
                ]
                .iter()
                .map(|(t, id)| agent_is_now_wearing::WearableData {
                    item_id: uuid::Uuid::parse_str(id).unwrap_or_default(),
                    wearable_type: *t,
                })
                .collect();
                self.send_main(&m, true);
            }
            NetCommand::OneShotControl(f) => self.one_shot |= f,
            NetCommand::ClearObjectCache => {
                for c in self.obj_caches.values_mut() {
                    c.reset();
                }
            }
            NetCommand::Build(b) => self.on_build(b),
            // handled by on_social_command
            NetCommand::RequestMuteList { .. }
            | NetCommand::UpdateMute { .. }
            | NetCommand::RemoveMute { .. }
            | NetCommand::RequestGroups
            | NetCommand::ActivateGroup(_)
            | NetCommand::LeaveGroup(_)
            | NetCommand::TerminateFriendship(_)
            | NetCommand::AgentAnimation { .. }
            | NetCommand::ObjectGrab { .. }
            | NetCommand::ObjectGrabUpdate { .. }
            | NetCommand::ObjectRelease { .. }
            | NetCommand::RequestTaskInventory { .. }
            | NetCommand::ChatSession { .. } => {}
        }
    }

    /// LLAvatarNameCache::requestNamesViaCapability: GET
    /// `<GetDisplayNames>/?ids=<id>&ids=<id>…`, URLs kept under ~3500
    /// characters. Without the capability (or on an HTTP error) the ids are
    /// handed back as failed, for the legacy UUIDNameRequest.
    fn request_display_names(&mut self, ids: Vec<uuid::Uuid>) {
        const NAME_URL_SEND_THRESHOLD: usize = 3500;
        let cap = self
            .main
            .and_then(|a| self.sims.get(&a))
            .and_then(|s| s.caps.get("GetDisplayNames").cloned());
        let Some(cap) = cap else {
            emit(self.sh, NetEvent::DisplayNamesFailed(ids));
            return;
        };
        // caps are granted without the slash the query needs
        let base = if cap.ends_with('/') { cap } else { format!("{cap}/") };
        let mut batches: Vec<(String, Vec<uuid::Uuid>)> = Vec::new();
        for id in ids {
            match batches.last_mut() {
                Some((url, list)) if url.len() <= NAME_URL_SEND_THRESHOLD => {
                    url.push_str("&ids=");
                    url.push_str(&id.to_string());
                    list.push(id);
                }
                _ => batches.push((format!("{base}?ids={id}"), vec![id])),
            }
        }
        for (url, ids) in batches {
            let http = self.sh.caps_http.clone();
            let events = self.sh.events.clone();
            tokio::spawn(async move {
                let ev = match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                    Ok(resp) if resp.status().is_success() => {
                        // Cache-Control: max-age=<seconds> (expirationFromCacheControl)
                        let max_age = resp.headers().get("cache-control").and_then(|v| v.to_str().ok()).and_then(|s| {
                            s.split(',')
                                .filter_map(|p| p.trim().strip_prefix("max-age="))
                                .find_map(|v| v.trim().parse::<u64>().ok())
                        });
                        match resp.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok()) {
                            Some(v) if v.is_map() => {
                                let names = v["agents"]
                                    .as_array()
                                    .iter()
                                    .map(AvatarNameData::from_llsd)
                                    .filter(|n| !n.id.is_nil())
                                    .collect();
                                let bad_ids = v["bad_ids"]
                                    .as_array()
                                    .iter()
                                    .map(|i| i.as_uuid())
                                    .filter(|i| !i.is_nil())
                                    .collect();
                                NetEvent::DisplayNames { names, bad_ids, max_age }
                            }
                            _ => {
                                log::warn!("GetDisplayNames: invalid reply for {} ids", ids.len());
                                NetEvent::DisplayNamesFailed(ids)
                            }
                        }
                    }
                    Ok(resp) => {
                        log::warn!("GetDisplayNames: HTTP {} for {} ids", resp.status(), ids.len());
                        NetEvent::DisplayNamesFailed(ids)
                    }
                    Err(e) => {
                        log::warn!("GetDisplayNames: {e}");
                        NetEvent::DisplayNamesFailed(ids)
                    }
                };
                let _ = events.send(ev);
            });
        }
    }

    /// LLViewerDisplayName::set: POST {display_name: [old, new]} to
    /// SetDisplayName. The answer comes later as SetDisplayNameReply on the
    /// event queue; only a failed POST is reported here.
    fn set_display_name(&mut self, old: String, new: String) {
        let cap = self
            .main
            .and_then(|a| self.sims.get(&a))
            .and_then(|s| s.caps.get("SetDisplayName").cloned());
        let Some(url) = cap else {
            emit(
                self.sh,
                NetEvent::SetDisplayNameReply {
                    status: 0,
                    reason: "unsupported".into(),
                    content: Llsd::Undef,
                },
            );
            return;
        };
        log::info!("SetDisplayName: {old:?} -> {new:?}");
        let mut body = Llsd::new_map();
        body.insert("display_name", Llsd::Array(vec![Llsd::from(old), Llsd::from(new)]));
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            // the People API answers errors in the language asked for
            let resp = http
                .post(&url)
                .header("Content-Type", "application/llsd+xml")
                .header("Accept", "application/llsd+xml")
                .header("Accept-Language", "fr")
                .body(aurora_llsd::to_xml(&body))
                .send()
                .await;
            let failed = match resp {
                Ok(r) if r.status().is_success() => None,
                Ok(r) => Some((r.status().as_u16() as i32, r.status().to_string())),
                Err(e) => Some((0, e.to_string())),
            };
            if let Some((status, reason)) = failed {
                log::warn!("SetDisplayName failed: {reason}");
                let _ = events.send(NetEvent::SetDisplayNameReply {
                    status,
                    reason,
                    content: Llsd::Undef,
                });
            }
        });
    }

    /// LLPanelProfileTab saveAgentUserInfoCoro: PUT <AgentProfile>/<target>;
    /// without the capability only the notes have a UDP message
    /// (AvatarNotesUpdate).
    fn update_profile(&mut self, target: uuid::Uuid, data: Llsd) {
        let cap = self
            .main
            .and_then(|a| self.sims.get(&a))
            .and_then(|s| s.caps.get("AgentProfile").cloned());
        let Some(url) = cap else {
            if data.has("notes") {
                let mut m = AvatarNotesUpdate::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.data.target_id = target;
                m.data.notes = str_field(data["notes"].as_str());
                self.send_main(&m, true);
            } else {
                emit(
                    self.sh,
                    NetEvent::ProfileSaveFailed {
                        target,
                        reason: "la région n'a pas la capability AgentProfile".into(),
                    },
                );
            }
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let url = format!("{}/{target}", url.trim_end_matches('/'));
            let resp = http
                .put(&url)
                .header("Content-Type", "application/llsd+xml")
                .header("Accept", "application/llsd+xml")
                .body(aurora_llsd::to_xml(&data))
                .send()
                .await;
            let reason = match resp {
                Ok(r) if r.status().is_success() => return,
                Ok(r) => format!("HTTP {}", r.status().as_u16()),
                Err(e) => e.to_string(),
            };
            log::warn!("AgentProfile PUT {target}: {reason}");
            let _ = events.send(NetEvent::ProfileSaveFailed { target, reason });
        });
    }

    /// send_generic_message (llviewergenericmessage.cpp): one parameter
    /// block per string, an empty one when there are none.
    fn send_generic(&mut self, method: &str, params: &[String]) {
        let mut m = GenericMessage::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        m.method_data.method = str_field(method);
        let blocks: Vec<&str> = if params.is_empty() {
            vec![""]
        } else {
            params.iter().map(String::as_str).collect()
        };
        m.param_list = blocks
            .into_iter()
            .map(|p| msgs::generic_message::ParamList { parameter: str_field(p) })
            .collect();
        self.send_main(&m, true);
    }

    fn send_im(&mut self, to: uuid::Uuid, text: &str, dialog: u8) {
        let mut m = ImprovedInstantMessage::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        let b = &mut m.message_block;
        b.from_group = false;
        b.to_agent_id = to;
        b.offline = 0;
        b.dialog = dialog;
        // Session id for 1:1 IMs is agent XOR other (LLIMMgr::computeSessionID).
        let a = self.agent_id().as_u128();
        b.id = uuid::Uuid::from_u128(a ^ to.as_u128());
        b.timestamp = 0;
        b.from_agent_name = str_field(&format!("{} {}", self.login.first_name, self.login.last_name));
        b.message = str_field(text);
        b.binary_bucket = vec![0];
        self.send_main(&m, true);
    }

    fn fetch_inventory(&mut self, folders: Vec<uuid::Uuid>, owner: uuid::Uuid, library: bool) {
        let cap_name = if library {
            "FetchLibDescendents2"
        } else {
            "FetchInventoryDescendents2"
        };
        let url = self
            .main
            .and_then(|a| self.sims.get(&a))
            .and_then(|s| s.caps.get(cap_name).cloned());
        let Some(url) = url else {
            emit(self.sh, NetEvent::InventoryFetchFailed { folders });
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            for chunk in folders.chunks(16) {
                let body = crate::inventory::fetch_request_body(chunk, owner);
                match caps::post_llsd(&http, &url, &body).await {
                    Ok(v) => {
                        let contents = crate::inventory::parse_fetch_response(&v);
                        let _ = events.send(NetEvent::InventoryContents(contents));
                    }
                    Err(e) => {
                        log::warn!("inventory fetch failed: {e}");
                        let _ = events.send(NetEvent::InventoryFetchFailed { folders: chunk.to_vec() });
                    }
                }
            }
        });
    }

    fn main_cap(&self, name: &str) -> Option<String> {
        self.main.and_then(|a| self.sims.get(&a)).and_then(|s| s.caps.get(name).cloned())
    }

    fn fetch_items(&mut self, items: Vec<uuid::Uuid>, owner: uuid::Uuid) {
        let Some(url) = self.main_cap("FetchInventory2") else {
            emit(self.sh, NetEvent::InventoryItemsFailed { items });
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            for chunk in items.chunks(50) {
                let body = crate::inventory::fetch_items_body(chunk, owner);
                match caps::post_llsd(&http, &url, &body).await {
                    Ok(v) => {
                        let _ = events.send(NetEvent::InventoryItems(crate::inventory::parse_items_response(&v)));
                    }
                    Err(e) => {
                        log::warn!("item fetch failed: {e}");
                        let _ = events.send(NetEvent::InventoryItemsFailed { items: chunk.to_vec() });
                    }
                }
            }
        });
    }

    /// LLAppearanceMgr::serverAppearanceUpdateCoro: POST the COF version;
    /// the server bakes the outfit and sends a new AvatarAppearance.
    fn request_server_appearance(&mut self, cof_version: i32) {
        let Some(url) = self.main_cap("UpdateAvatarAppearance") else {
            log::warn!("no UpdateAvatarAppearance capability: appearance not requested");
            emit(
                self.sh,
                NetEvent::AppearanceRequestResult {
                    cof_version,
                    success: false,
                    expected: None,
                },
            );
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let body = aurora_llsd::to_xml(&aurora_llsd::llsd_map! { "cof_version" => cof_version });
            let result = http
                .post(&url)
                .header("Content-Type", "application/llsd+xml")
                .header("Accept", "application/llsd+xml")
                .body(body)
                .send()
                .await;
            let (success, expected) = match result {
                Ok(resp) => {
                    let status = resp.status();
                    let doc = resp.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok());
                    match doc {
                        Some(d) => {
                            let ok = status.is_success() && d["success"].as_bool();
                            if !ok {
                                log::warn!(
                                    "appearance request for COF {cof_version} failed: HTTP {status} {}",
                                    d["error"].to_string_value()
                                );
                            }
                            (ok, d.has("expected").then(|| d["expected"].as_i32()))
                        }
                        None => {
                            log::warn!("appearance request for COF {cof_version}: HTTP {status}");
                            (false, None)
                        }
                    }
                }
                Err(e) => {
                    log::warn!("appearance request failed: {e}");
                    (false, None)
                }
            };
            if success {
                log::info!("server appearance requested for COF version {cof_version}");
            }
            let _ = events.send(NetEvent::AppearanceRequestResult {
                cof_version,
                success,
                expected,
            });
        });
    }

    fn request_objects(&mut self, addr: SocketAddr, ids: &[u32]) {
        for chunk in ids.chunks(255) {
            let mut m = RequestMultipleObjects::default();
            m.agent_data.agent_id = self.agent_id();
            m.agent_data.session_id = self.session_id();
            m.object_data = chunk
                .iter()
                .map(|&id| request_multiple_objects::ObjectData { cache_miss_type: 0, id })
                .collect();
            self.send(addr, &m, true);
        }
    }

    // --------------------------------------------------------- event queue

    fn sim_for_handle(&self, handle: RegionHandle) -> Option<SocketAddr> {
        self.sims.iter().find(|(_, s)| s.handle == handle).map(|(a, _)| *a)
    }

    fn on_eq_event(&mut self, e: EqEvent) {
        log::debug!("EQ {} from {}", e.message, e.sim);
        let b = &e.body;
        if self.on_social_eq(&e.message, b) || self.on_land_eq(&e.message, b) || self.on_build_eq(&e.message, b) {
            return;
        }
        match e.message.as_str() {
            "EnableSimulator" => {
                for info in b["SimulatorInfo"].as_array() {
                    let handle = info["Handle"].as_u64();
                    let Some(ip) = ip_from_llsd(&info["IP"]) else {
                        continue;
                    };
                    let port = info["Port"].as_i32() as u16;
                    let size = (info["RegionSizeX"].as_u32().max(256), info["RegionSizeY"].as_u32().max(256));
                    let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
                    self.add_sim(addr, handle, size);
                }
            }
            "EstablishAgentCommunication" => {
                let ipport = b["sim-ip-and-port"].as_str();
                let seed = b["seed-capability"].as_str().to_owned();
                if let Ok(addr) = ipport.parse::<SocketAddr>() {
                    if let Some(sim) = self.sims.get_mut(&addr) {
                        sim.seed = Some(seed);
                    }
                    self.request_caps(addr);
                }
            }
            "TeleportFinish" => {
                let info = &b["Info"][0];
                emit(
                    self.sh,
                    NetEvent::TeleportFinished {
                        handle: info["RegionHandle"].as_u64(),
                    },
                );
                log::info!(
                    "TeleportFinish: region {:x}, flags {:#x}",
                    info["RegionHandle"].as_u64(),
                    info["TeleportFlags"].as_u64()
                );
                self.switch_region(
                    info["RegionHandle"].as_u64(),
                    ip_from_llsd(&info["SimIP"]),
                    info["SimPort"].as_i32() as u16,
                    info["SeedCapability"].as_str().to_owned(),
                    (info["RegionSizeX"].as_u32().max(256), info["RegionSizeY"].as_u32().max(256)),
                );
            }
            "CrossedRegion" => {
                let rd = &b["RegionData"][0];
                self.switch_region(
                    rd["RegionHandle"].as_u64(),
                    ip_from_llsd(&rd["SimIP"]),
                    rd["SimPort"].as_i32() as u16,
                    rd["SeedCapability"].as_str().to_owned(),
                    (rd["RegionSizeX"].as_u32().max(256), rd["RegionSizeY"].as_u32().max(256)),
                );
            }
            "ParcelProperties" => {
                let sim = e.sim;
                self.on_parcel_properties(sim, b);
            }
            "TeleportFailed" => {
                let reason = b["Info"][0]["Reason"].as_str().to_owned();
                emit(self.sh, NetEvent::TeleportFailed { reason });
            }
            "DisableSimulator" => {
                if self.main != Some(e.sim) {
                    self.remove_sim(e.sim);
                }
            }
            // LLSetDisplayNameReply (llviewerdisplayname.cpp)
            "SetDisplayNameReply" => {
                let status = b["status"].as_i32();
                let reason = b["reason"].to_string_value();
                log::info!("SetDisplayNameReply: {status} {reason}");
                emit(
                    self.sh,
                    NetEvent::SetDisplayNameReply {
                        status,
                        reason,
                        content: b["content"].clone(),
                    },
                );
            }
            // LLDisplayNameUpdate (llviewerdisplayname.cpp)
            "DisplayNameUpdate" => {
                let mut name = AvatarNameData::from_llsd(&b["agent"]);
                let agent_id = b["agent_id"].as_uuid();
                if !agent_id.is_nil() {
                    name.id = agent_id;
                }
                if !name.id.is_nil() {
                    let old_display_name = b["old_display_name"].to_string_value();
                    emit(self.sh, NetEvent::DisplayNameUpdate { name, old_display_name });
                }
            }
            _ => {}
        }
    }

    fn switch_region(&mut self, handle: RegionHandle, ip: Option<Ipv4Addr>, port: u16, seed: String, size: (u32, u32)) {
        let Some(ip) = ip else {
            log::warn!("region switch without IP");
            return;
        };
        let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
        self.add_sim(addr, handle, size);
        if let Some(sim) = self.sims.get_mut(&addr) {
            sim.handle = handle;
            // A new seed means new capabilities.
            if sim.seed.as_deref() != Some(seed.as_str()) {
                sim.caps_requested = false;
            }
        }
        self.agent_moved = false;
        self.set_main(addr, Some(seed));
        self.complete_agent_movement(addr);
    }

    // ------------------------------------------------------------- packets

    fn on_datagram(&mut self, data: &[u8], from: SocketAddr) {
        let stats = &self.sh.stats;
        stats.packets_in.fetch_add(1, Ordering::Relaxed);
        stats.bytes_in.fetch_add(data.len() as u64, Ordering::Relaxed);
        let pkt = match parse_packet(data) {
            Ok(p) => p,
            Err(e) => {
                log::debug!("bad packet from {from}: {e}");
                stats.decode_errors.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        let Some(sim) = self.sims.get_mut(&from) else {
            return;
        };
        if !sim.circuit.on_receive(pkt.flags, pkt.sequence, &pkt.acks) {
            stats.duplicates.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if let Err(e) = self.dispatch(from, &pkt) {
            log::debug!(
                "failed to decode {} from {from}: {e}",
                aurora_msg::message_info(pkt.id).map(|i| i.name).unwrap_or("?")
            );
            self.sh.stats.decode_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn handle_of(&self, addr: SocketAddr) -> RegionHandle {
        self.sims.get(&addr).map(|s| s.handle).unwrap_or(0)
    }

    fn dispatch(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<(), aurora_msg::DecodeError> {
        let id = pkt.id;
        if self.dispatch_object_actions(from, pkt)?
            || self.dispatch_social(from, pkt)?
            || self.dispatch_land(from, pkt)?
            || self.dispatch_build(pkt)?
        {
            return Ok(());
        }
        if id == PacketAck::ID {
            let m: PacketAck = pkt.decode()?;
            if let Some(sim) = self.sims.get_mut(&from) {
                sim.circuit.ack_received(m.packets.iter().map(|p| p.id));
            }
        } else if id == StartPingCheck::ID {
            let m: StartPingCheck = pkt.decode()?;
            let mut r = CompletePingCheck::default();
            r.ping_id.ping_id = m.ping_id.ping_id;
            self.send(from, &r, false);
        } else if id == CompletePingCheck::ID {
            let m: CompletePingCheck = pkt.decode()?;
            let is_main = self.main == Some(from);
            if let Some(sim) = self.sims.get_mut(&from)
                && let Some(rtt) = sim.circuit.on_ping_reply(m.ping_id.ping_id)
                && is_main
            {
                self.sh.stats.ping_ms.store(rtt.as_millis() as u32, Ordering::Relaxed);
            }
        } else if id == ObjectUpdate::ID {
            let m: ObjectUpdate = pkt.decode()?;
            self.set_time_dilation(from, m.region_data.time_dilation);
            let objects: Vec<_> = m.object_data.iter().filter_map(objects::parse_full).collect();
            if !objects.is_empty() {
                emit(
                    self.sh,
                    NetEvent::ObjectUpdates {
                        handle: self.handle_of(from),
                        objects,
                    },
                );
            }
        } else if id == ObjectUpdateCompressed::ID {
            let m: ObjectUpdateCompressed = pkt.decode()?;
            self.set_time_dilation(from, m.region_data.time_dilation);
            let mut cache = self.obj_caches.get_mut(&from);
            let objects: Vec<_> = m
                .object_data
                .iter()
                .filter_map(|b| {
                    let u = objects::parse_compressed(&b.data, b.update_flags)?;
                    if let Some(c) = cache.as_mut() {
                        c.put(u.local_id, u.crc, b.update_flags, &b.data);
                    }
                    Some(u)
                })
                .collect();
            if !objects.is_empty() {
                emit(
                    self.sh,
                    NetEvent::ObjectUpdates {
                        handle: self.handle_of(from),
                        objects,
                    },
                );
            }
        } else if id == ImprovedTerseObjectUpdate::ID {
            let m: ImprovedTerseObjectUpdate = pkt.decode()?;
            self.set_time_dilation(from, m.region_data.time_dilation);
            let updates: Vec<_> = m.object_data.iter().filter_map(objects::parse_terse).collect();
            if !updates.is_empty() {
                emit(
                    self.sh,
                    NetEvent::TerseUpdates {
                        handle: self.handle_of(from),
                        updates,
                    },
                );
            }
        } else if id == ObjectUpdateCached::ID {
            let m: ObjectUpdateCached = pkt.decode()?;
            self.set_time_dilation(from, m.region_data.time_dilation);
            // cache hits (same CRC) are applied locally, misses requested
            let mut hits = Vec::new();
            let mut misses = Vec::new();
            // the simulator does not resend the overrides of a cache hit
            let mut overrides = Vec::new();
            for o in &m.object_data {
                let cache = self.obj_caches.get(&from);
                let cached = cache
                    .and_then(|c| c.get(o.id, o.crc))
                    .and_then(|e| objects::parse_compressed(&e.data, o.update_flags));
                match cached {
                    Some(u) => {
                        if let Some(ov) = cache.and_then(|c| c.gltf_override(o.id)).and_then(objects::parse_gltf_override) {
                            overrides.push(ov);
                        }
                        hits.push(u)
                    }
                    None => misses.push(o.id),
                }
            }
            self.sh.stats.object_cache_hits.fetch_add(hits.len() as u64, Ordering::Relaxed);
            self.sh.stats.object_cache_misses.fetch_add(misses.len() as u64, Ordering::Relaxed);
            if !hits.is_empty() {
                emit(
                    self.sh,
                    NetEvent::ObjectUpdates {
                        handle: self.handle_of(from),
                        objects: hits,
                    },
                );
            }
            for (local_id, sides) in overrides {
                emit(
                    self.sh,
                    NetEvent::GltfOverrides {
                        handle: self.handle_of(from),
                        local_id,
                        sides,
                    },
                );
            }
            self.request_objects(from, &misses);
        } else if id == GenericStreamingMessage::ID {
            let m: GenericStreamingMessage = pkt.decode()?;
            if m.method_data.method == objects::METHOD_GLTF_MATERIAL_OVERRIDE {
                let payload = &m.data_block.data;
                match objects::parse_gltf_override(payload) {
                    Some((local_id, sides)) => {
                        if let Some(c) = self.obj_caches.get_mut(&from) {
                            c.set_gltf_override(local_id, (!sides.is_empty()).then_some(payload.as_slice()));
                        }
                        emit(
                            self.sh,
                            NetEvent::GltfOverrides {
                                handle: self.handle_of(from),
                                local_id,
                                sides,
                            },
                        );
                    }
                    None => log::warn!("malformed GLTF material override ({} bytes)", payload.len()),
                }
            } else {
                log::debug!("GenericStreamingMessage: unknown method {:#06x}", m.method_data.method);
            }
        } else if id == PayPriceReply::ID {
            let m: PayPriceReply = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::PayPrice {
                    object: m.object_data.object_id,
                    default: m.object_data.default_pay_price,
                    buttons: m.button_data.iter().take(4).map(|b| b.pay_button).collect(),
                },
            );
        } else if id == ObjectProperties::ID {
            let m: ObjectProperties = pkt.decode()?;
            emit(self.sh, NetEvent::ObjectProperties(build_cmds::parse_properties(&m)));
        } else if id == ObjectPropertiesFamily::ID {
            let m: ObjectPropertiesFamily = pkt.decode()?;
            emit(self.sh, NetEvent::ObjectProperties(vec![build_cmds::parse_family(&m)]));
        } else if id == KillObject::ID {
            let m: KillObject = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::ObjectsKilled {
                    handle: self.handle_of(from),
                    local_ids: m.object_data.iter().map(|o| o.id).collect(),
                },
            );
        } else if id == LayerData::ID {
            let m: LayerData = pkt.decode()?;
            let patches = terrain::decode_land_layer(m.layer_id.type_, &m.layer_data.data);
            if !patches.is_empty() {
                emit(
                    self.sh,
                    NetEvent::Terrain {
                        handle: self.handle_of(from),
                        patches,
                    },
                );
            }
        } else if id == ScriptDialog::ID {
            let m: ScriptDialog = pkt.decode()?;
            let d = &m.data;
            let owner = format!("{} {}", field_str(&d.first_name), field_str(&d.last_name));
            emit(
                self.sh,
                NetEvent::ScriptDialog {
                    object_id: d.object_id,
                    object_name: field_str(&d.object_name),
                    owner_name: owner.trim().to_owned(),
                    message: field_str(&d.message),
                    channel: d.chat_channel,
                    buttons: m.buttons.iter().map(|b| field_str(&b.button_label)).collect(),
                },
            );
        } else if id == ScriptQuestion::ID {
            let m: ScriptQuestion = pkt.decode()?;
            let d = &m.data;
            emit(
                self.sh,
                NetEvent::ScriptQuestion {
                    task_id: d.task_id,
                    item_id: d.item_id,
                    object_name: field_str(&d.object_name),
                    owner_name: field_str(&d.object_owner),
                    questions: d.questions,
                },
            );
        } else if id == LoadURL::ID {
            let m: LoadURL = pkt.decode()?;
            let d = &m.data;
            emit(
                self.sh,
                NetEvent::LoadUrl {
                    object_name: field_str(&d.object_name),
                    object_id: d.object_id,
                    message: field_str(&d.message),
                    url: field_str(&d.url),
                },
            );
        } else if id == AvatarSitResponse::ID {
            // the simulator sits us itself unless it asks for the autopilot walk
            // (process_avatar_sit_response); we skip the walk and sit at once
            let m: AvatarSitResponse = pkt.decode()?;
            let st = &m.sit_transform;
            emit(
                self.sh,
                NetEvent::SitResponse {
                    object: m.sit_object.id,
                    camera_eye: st.camera_eye_offset,
                    camera_at: st.camera_at_offset,
                    force_mouselook: st.force_mouselook,
                },
            );
            if m.sit_transform.auto_pilot {
                let mut s = AgentSit::default();
                s.agent_data.agent_id = self.agent_id();
                s.agent_data.session_id = self.session_id();
                self.send(from, &s, true);
            }
        } else if id == CameraConstraint::ID {
            // the plane the simulator keeps our camera in front of (process_camera_constraint)
            if self.main == Some(from) {
                let m: CameraConstraint = pkt.decode()?;
                emit(self.sh, NetEvent::CameraConstraint(m.camera_collide_plane.plane));
            }
        } else if id == PickInfoReply::ID {
            let m: PickInfoReply = pkt.decode()?;
            let d = &m.data;
            let pick = crate::PickInfo {
                id: d.pick_id,
                creator: d.creator_id,
                parcel: d.parcel_id,
                name: field_str(&d.name),
                desc: field_str(&d.desc),
                snapshot: d.snapshot_id,
                sim_name: field_str(&d.sim_name),
                pos_global: d.pos_global,
                sort_order: d.sort_order,
                enabled: d.enabled,
            };
            emit(self.sh, NetEvent::PickInfo(Box::new(pick)));
        } else if id == AvatarClassifiedReply::ID {
            let m: AvatarClassifiedReply = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::AvatarClassifieds {
                    target: m.agent_data.target_id,
                    list: m.data.iter().map(|d| (d.classified_id, field_str(&d.name))).collect(),
                },
            );
        } else if id == ClassifiedInfoReply::ID {
            let m: ClassifiedInfoReply = pkt.decode()?;
            let d = &m.data;
            let c = crate::ClassifiedInfo {
                id: d.classified_id,
                creator: d.creator_id,
                creation_date: d.creation_date,
                expiration_date: d.expiration_date,
                category: d.category,
                name: field_str(&d.name),
                desc: field_str(&d.desc),
                parcel: d.parcel_id,
                snapshot: d.snapshot_id,
                sim_name: field_str(&d.sim_name),
                pos_global: d.pos_global,
                parcel_name: field_str(&d.parcel_name),
                flags: d.classified_flags,
                price: d.price_for_listing,
            };
            emit(self.sh, NetEvent::ClassifiedInfo(Box::new(c)));
        } else if id == ParcelInfoReply::ID {
            let m: ParcelInfoReply = pkt.decode()?;
            let d = &m.data;
            let p = crate::ParcelSummary {
                id: d.parcel_id,
                name: field_str(&d.name),
                sim_name: field_str(&d.sim_name),
                global: glam::DVec3::new(d.global_x as f64, d.global_y as f64, d.global_z as f64),
                snapshot: d.snapshot_id,
            };
            emit(self.sh, NetEvent::ParcelInfo(Box::new(p)));
        } else if id == ChangeUserRights::ID {
            let m: ChangeUserRights = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::UserRights {
                    agent: m.agent_data.agent_id,
                    rights: m.rights.iter().map(|r| (r.agent_related, r.related_rights)).collect(),
                },
            );
        } else if id == AvatarPropertiesReply::ID {
            let m: AvatarPropertiesReply = pkt.decode()?;
            let pd = &m.properties_data;
            // processAvatarPropertiesReply: no groups, picks nor notes here
            let (caption_index, caption_text) = crate::profile::parse_charter_member(&pd.charter_member);
            let p = crate::AvatarProfile {
                id: m.agent_data.avatar_id,
                sl_image: pd.image_id,
                fl_image: pd.fl_image_id,
                partner: pd.partner_id,
                sl_about: field_str(&pd.about_text),
                fl_about: field_str(&pd.fl_about_text),
                born: crate::profile::parse_pdt_date(&field_str(&pd.born_on)),
                online: Some(pd.flags & crate::profile::flags::ONLINE != 0),
                flags: pd.flags,
                caption_index,
                caption_text,
                legacy: true,
                ..Default::default()
            };
            emit(self.sh, NetEvent::AvatarProfile(Box::new(p)));
        } else if id == msgs::RegionInfo::ID {
            // estate changes: water height, and the region environment may
            // have changed (LLRegionInfoModel update -> LLEnvironment::requestRegion)
            let m: msgs::RegionInfo = pkt.decode()?;
            if let Some(handle) = self.sims.get(&from).map(|s| s.handle) {
                let height = m.region_info.water_height;
                if height.is_finite() {
                    emit(self.sh, NetEvent::WaterHeight { handle, height });
                }
                if self.main == Some(from) {
                    self.request_environment(None);
                }
            }
        } else if id == SimStats::ID {
            // process_sim_stats: the region flags ride along (an estate manager
            // turning off fly, voice, push... shows without a new handshake)
            let m: SimStats = pkt.decode()?;
            if let Some(handle) = self.sims.get(&from).map(|s| s.handle) {
                // the flags we use are all in the low 32 bits
                let flags = m
                    .region_info
                    .first()
                    .map_or(m.region.region_flags, |r| r.region_flags_extended as u32);
                let max_tasks = m.region.object_capacity;
                emit(self.sh, NetEvent::RegionFlags { handle, flags, max_tasks });
            }
        } else if id == HealthMessage::ID {
            let m: HealthMessage = pkt.decode()?;
            emit(self.sh, NetEvent::Health(m.health_data.health));
        } else if id == RegionHandshake::ID {
            let m: RegionHandshake = pkt.decode()?;
            self.on_region_handshake(from, m);
        } else if id == AgentMovementComplete::ID {
            let m: AgentMovementComplete = pkt.decode()?;
            self.agent_moved = true;
            self.cam_requested_at = None;
            if let Some(sim) = self.sims.get_mut(&from) {
                sim.handle = m.data.region_handle;
            }
            emit(
                self.sh,
                NetEvent::AgentMovementComplete {
                    handle: m.data.region_handle,
                    position: m.data.position,
                    look_at: m.data.look_at,
                },
            );
            let mut r = AgentDataUpdateRequest::default();
            r.agent_data.agent_id = self.agent_id();
            r.agent_data.session_id = self.session_id();
            self.send(from, &r, true);
            let mut fov = AgentFOV::default();
            fov.agent_data.agent_id = self.agent_id();
            fov.agent_data.session_id = self.session_id();
            fov.agent_data.circuit_code = self.login.circuit_code;
            fov.fov_block.vertical_angle = 1.0;
            self.send(from, &fov, true);
            let mut mb = MoneyBalanceRequest::default();
            mb.agent_data.agent_id = self.agent_id();
            mb.agent_data.session_id = self.session_id();
            self.send(from, &mb, true);
            self.maybe_send_agent_update(Instant::now());
        } else if id == ChatFromSimulator::ID {
            let m: ChatFromSimulator = pkt.decode()?;
            let c = &m.chat_data;
            // 4/5: typing start/stop
            // CHAT_AUDIBLE_FULLY == 1 (faint = 0, inaudible = -1 → 255)
            if c.chat_type != 4 && c.chat_type != 5 && c.audible == 1 {
                emit(
                    self.sh,
                    NetEvent::Chat(ChatMessage {
                        from_name: field_str(&c.from_name),
                        source_id: c.source_id,
                        owner_id: c.owner_id,
                        source_type: match c.source_type {
                            0 => ChatSourceType::System,
                            1 => ChatSourceType::Agent,
                            _ => ChatSourceType::Object,
                        },
                        chat_type: ChatType::from_u8(c.chat_type),
                        position: c.position,
                        message: field_str(&c.message),
                    }),
                );
            }
        } else if id == ImprovedInstantMessage::ID {
            let m: ImprovedInstantMessage = pkt.decode()?;
            let b = &m.message_block;
            if b.dialog != 41 && b.dialog != 42 {
                emit(
                    self.sh,
                    NetEvent::InstantMessage(InstantMessage {
                        from_agent_id: m.agent_data.agent_id,
                        from_name: field_str(&b.from_agent_name),
                        to_agent_id: b.to_agent_id,
                        dialog: b.dialog,
                        session_id: b.id,
                        message: field_str(&b.message),
                        offline: b.offline != 0,
                        from_group: b.from_group,
                        binary_bucket: b.binary_bucket.clone(),
                    }),
                );
            }
        } else if id == msgs::AvatarAppearance::ID {
            let m: msgs::AvatarAppearance = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::Appearance(crate::types::AvatarAppearance {
                    avatar_id: m.sender.id,
                    texture_entry: aurora_prim::parse_texture_entry(&m.object_data.texture_entry).map(Arc::new),
                    visual_params: m.visual_param.iter().map(|p| p.param_value).collect(),
                    cof_version: m.appearance_data.first().map(|a| a.cof_version).unwrap_or(0),
                    hover_height: m.appearance_hover.first().map(|h| h.hover_height.z).unwrap_or(0.0),
                }),
            );
        } else if id == SimulatorViewerTimeMessage::ID {
            let m: SimulatorViewerTimeMessage = pkt.decode()?;
            let t = &m.time_info;
            if self.main == Some(from) {
                emit(
                    self.sh,
                    NetEvent::Sun(SunInfo {
                        sun_direction: t.sun_direction,
                        sun_phase: t.sun_phase,
                        sec_per_day: t.sec_per_day,
                        usec_since_start: t.usec_since_start,
                    }),
                );
            }
        } else if id == CoarseLocationUpdate::ID {
            let m: CoarseLocationUpdate = pkt.decode()?;
            let positions = m
                .location
                .iter()
                .zip(m.agent_data.iter())
                .map(|(l, a)| (a.agent_id, Vec3::new(l.x as f32, l.y as f32, l.z as f32 * 4.0)))
                .collect();
            emit(
                self.sh,
                NetEvent::CoarseLocations {
                    handle: self.handle_of(from),
                    positions,
                },
            );
        } else if id == AlertMessage::ID {
            let m: AlertMessage = pkt.decode()?;
            let msg = field_str(&m.alert_data.message);
            if !msg.is_empty() {
                emit(self.sh, NetEvent::Alert { message: msg });
            }
        } else if id == AgentAlertMessage::ID {
            let m: AgentAlertMessage = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::Alert {
                    message: field_str(&m.alert_data.message),
                },
            );
        } else if id == KickUser::ID {
            let m: KickUser = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::Disconnected {
                    reason: field_str(&m.user_info.reason),
                },
            );
            self.finished = true;
        } else if id == LogoutReply::ID {
            emit(self.sh, NetEvent::LoggedOut);
            self.finished = true;
        } else if id == DisableSimulator::ID {
            if self.main == Some(from) {
                emit(
                    self.sh,
                    NetEvent::Disconnected {
                        reason: "La région a fermé la connexion.".into(),
                    },
                );
                self.finished = true;
            } else {
                self.remove_sim(from);
            }
        } else if id == ParcelOverlay::ID {
            // LLViewerParcelMgr::processParcelOverlay: one quarter of the
            // region's 4 m grid per message (property lines, for sale)
            let m: ParcelOverlay = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::ParcelOverlay {
                    handle: self.handle_of(from),
                    sequence: m.parcel_data.sequence_id,
                    data: m.parcel_data.data,
                },
            );
        } else if id == MapBlockReply::ID {
            let m: MapBlockReply = pkt.decode()?;
            if let Some((want, pos)) = self.pending_named_tp.clone() {
                let found = m.data.iter().find(|d| field_str(&d.name).to_lowercase() == want);
                match found {
                    Some(d) => {
                        self.pending_named_tp = None;
                        let handle = ((d.x as u64 * 256) << 32) | (d.y as u64 * 256);
                        let mut t = TeleportLocationRequest::default();
                        t.agent_data.agent_id = self.agent_id();
                        t.agent_data.session_id = self.session_id();
                        t.info.region_handle = handle;
                        t.info.position = pos;
                        t.info.look_at = Vec3::X;
                        self.send_main(&t, true);
                    }
                    // a name search answers with prefix matches and an empty
                    // end-of-list marker; other replies come from map browsing
                    None if m.data.iter().all(|d| {
                        let n = field_str(&d.name).to_lowercase();
                        n.is_empty() || n.starts_with(&want)
                    }) =>
                    {
                        self.pending_named_tp = None;
                        emit(
                            self.sh,
                            NetEvent::TeleportFailed {
                                reason: format!("Région introuvable : {want}"),
                            },
                        );
                    }
                    None => {}
                }
            }
            // LLWorldMapMessage::processMapBlockReply
            let blocks = m
                .data
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let (mut sx, mut sy) = m.size.get(i).map(|s| (s.size_x, s.size_y)).unwrap_or((256, 256));
                    if sx == 0 || sx % 16 != 0 || sy % 16 != 0 {
                        (sx, sy) = (256, 256);
                    }
                    MapBlock {
                        x: d.x,
                        y: d.y,
                        name: field_str(&d.name),
                        access: d.access,
                        region_flags: d.region_flags,
                        water_height: d.water_height,
                        agents: d.agents,
                        map_image_id: d.map_image_id,
                        size_x: sx,
                        size_y: sy,
                    }
                })
                .collect();
            emit(
                self.sh,
                NetEvent::MapBlocks {
                    blocks,
                    null_sims: m.agent_data.flags != 2,
                },
            );
        } else if id == MapItemReply::ID {
            let m: MapItemReply = pkt.decode()?;
            let items = m
                .data
                .iter()
                .map(|d| MapItem {
                    x: d.x,
                    y: d.y,
                    id: d.id,
                    extra: d.extra,
                    extra2: d.extra2,
                    name: field_str(&d.name),
                })
                .collect();
            emit(
                self.sh,
                NetEvent::MapItems {
                    item_type: m.request_data.item_type,
                    items,
                },
            );
        } else if id == UUIDNameReply::ID {
            let m: UUIDNameReply = pkt.decode()?;
            let names = m
                .uuid_name_block
                .iter()
                .map(|b| (b.id, field_str(&b.first_name), field_str(&b.last_name)))
                .collect();
            emit(self.sh, NetEvent::Names(names));
        } else if id == OnlineNotification::ID {
            let m: OnlineNotification = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::FriendsOnline {
                    ids: m.agent_block.iter().map(|b| b.agent_id).collect(),
                    online: true,
                },
            );
        } else if id == OfflineNotification::ID {
            let m: OfflineNotification = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::FriendsOnline {
                    ids: m.agent_block.iter().map(|b| b.agent_id).collect(),
                    online: false,
                },
            );
        } else if id == AvatarAnimation::ID {
            let m: AvatarAnimation = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::AvatarAnimations {
                    avatar: m.sender.id,
                    anims: m.animation_list.iter().map(|a| (a.anim_id, a.anim_sequence_id)).collect(),
                },
            );
        } else if id == ObjectAnimation::ID {
            let m: ObjectAnimation = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::AvatarAnimations {
                    avatar: m.sender.id,
                    anims: m.animation_list.iter().map(|a| (a.anim_id, a.anim_sequence_id)).collect(),
                },
            );
        } else if id == MoneyBalanceReply::ID {
            let m: MoneyBalanceReply = pkt.decode()?;
            emit(self.sh, NetEvent::Balance(m.money_data.money_balance));
        } else if id == TeleportStart::ID {
            emit(self.sh, NetEvent::TeleportStarted);
        } else if id == TeleportProgress::ID {
            let m: TeleportProgress = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::TeleportProgress {
                    message: field_str(&m.info.message),
                },
            );
        } else if id == TeleportFailed::ID {
            let m: TeleportFailed = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::TeleportFailed {
                    reason: field_str(&m.info.reason),
                },
            );
        } else if id == ParcelMediaCommandMessage::ID {
            let m: ParcelMediaCommandMessage = pkt.decode()?;
            let c = &m.command_block;
            emit(
                self.sh,
                NetEvent::ParcelMediaCommand {
                    flags: c.flags,
                    command: c.command,
                    time: c.time,
                },
            );
        } else if id == ParcelMediaUpdate::ID {
            let m: ParcelMediaUpdate = pkt.decode()?;
            let (d, x) = (&m.data_block, &m.data_block_extended);
            emit(
                self.sh,
                NetEvent::ParcelMediaUpdate {
                    url: field_str(&d.media_url),
                    media_id: d.media_id,
                    auto_scale: d.media_auto_scale != 0,
                    mime: field_str(&x.media_type),
                    desc: field_str(&x.media_desc),
                    width: x.media_width,
                    height: x.media_height,
                    looping: x.media_loop != 0,
                },
            );
        } else if id == ViewerEffect::ID {
            let m: ViewerEffect = pkt.decode()?;
            for e in &m.effect {
                if let Some((source, target, offset, kind)) = parse_lookat(e) {
                    emit(
                        self.sh,
                        NetEvent::LookAt {
                            effect: e.id,
                            source,
                            target,
                            offset,
                            kind,
                            duration: e.duration,
                        },
                    );
                }
            }
        } else if id == SoundTrigger::ID {
            let m: SoundTrigger = pkt.decode()?;
            let d = &m.sound_data;
            emit(
                self.sh,
                NetEvent::SoundTrigger {
                    sound: d.sound_id,
                    owner: d.owner_id,
                    object: d.object_id,
                    parent: d.parent_id,
                    handle: d.handle,
                    position: d.position,
                    gain: d.gain,
                },
            );
        } else if id == AttachedSound::ID {
            let m: AttachedSound = pkt.decode()?;
            let d = &m.data_block;
            emit(
                self.sh,
                NetEvent::AttachedSound {
                    object: d.object_id,
                    sound: d.sound_id,
                    owner: d.owner_id,
                    gain: d.gain,
                    flags: d.flags,
                },
            );
        } else if id == AttachedSoundGainChange::ID {
            let m: AttachedSoundGainChange = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::AttachedSoundGain {
                    object: m.data_block.object_id,
                    gain: m.data_block.gain,
                },
            );
        } else if id == PreloadSound::ID {
            let m: PreloadSound = pkt.decode()?;
            emit(self.sh, NetEvent::PreloadSounds(m.data_block.iter().map(|b| b.sound_id).collect()));
        } else if id == TeleportLocal::ID {
            let m: TeleportLocal = pkt.decode()?;
            // process_teleport_local: flying from the teleport flags, then an
            // agent update at once (send_agent_update(true, true))
            const TELEPORT_FLAGS_IS_FLYING: u32 = 1 << 13;
            let flags = m.info.teleport_flags;
            log::info!(
                "TeleportLocal: position {:.1?}, flags {flags:#x}, agent updates {}",
                m.info.position,
                if self.agent_moved {
                    "on"
                } else {
                    "off (waiting AgentMovementComplete)"
                }
            );
            // the agent is in this region: keep (or start) sending its updates
            self.agent_moved = true;
            self.cam_requested_at = None;
            self.last_agent_update = Instant::now() - AGENT_UPDATE_KEEPALIVE;
            emit(
                self.sh,
                NetEvent::TeleportLocal {
                    flying: flags & TELEPORT_FLAGS_IS_FLYING != 0,
                },
            );
            emit(
                self.sh,
                NetEvent::AgentMovementComplete {
                    handle: self.handle_of(from),
                    position: m.info.position,
                    look_at: m.info.look_at,
                },
            );
        }
        Ok(())
    }

    fn on_region_handshake(&mut self, from: SocketAddr, m: RegionHandshake) {
        let r = &m.region_info;
        let is_main = self.main == Some(from);
        let (handle, size) = match self.sims.get_mut(&from) {
            Some(sim) => {
                sim.name = field_str(&r.sim_name);
                (sim.handle, sim.size)
            }
            None => return,
        };
        let info = crate::types::RegionInfo {
            handle,
            name: field_str(&r.sim_name),
            region_id: m.region_info2.region_id,
            water_height: r.water_height,
            sim_access: r.sim_access,
            region_flags: r.region_flags,
            terrain_base: [r.terrain_base0, r.terrain_base1, r.terrain_base2, r.terrain_base3],
            terrain_detail: [r.terrain_detail0, r.terrain_detail1, r.terrain_detail2, r.terrain_detail3],
            terrain_start_height: [
                r.terrain_start_height00,
                r.terrain_start_height01,
                r.terrain_start_height10,
                r.terrain_start_height11,
            ],
            terrain_height_range: [
                r.terrain_height_range00,
                r.terrain_height_range01,
                r.terrain_height_range10,
                r.terrain_height_range11,
            ],
            size_x: size.0,
            size_y: size.1,
            is_main,
            owner: r.sim_owner,
            is_estate_manager: r.is_estate_manager,
            product_name: field_str(&m.region_info3.product_name),
        };
        emit(self.sh, NetEvent::RegionHandshake(Arc::new(info)));
        if is_main {
            emit(
                self.sh,
                NetEvent::LoginProgress {
                    message: format!("Région {} rejointe", field_str(&r.sim_name)),
                    fraction: 0.6,
                },
            );
        }
        let mut reply = RegionHandshakeReply::default();
        reply.agent_data.agent_id = self.agent_id();
        reply.agent_data.session_id = self.session_id();
        // load the region's object cache before the sim starts sending
        // (LLViewerRegion::loadObjectCache), then tell it whether it is empty
        let region_id = m.region_info2.region_id;
        if let Some(dir) = self.obj_cache_dir.clone()
            && !self.obj_caches.contains_key(&from)
            && !region_id.is_nil()
        {
            let c = crate::objcache::RegionObjectCache::load(&dir, region_id);
            log::info!("object cache for {}: {} entries", field_str(&m.region_info.sim_name), c.len());
            self.obj_caches.insert(from, c);
        }
        let cache_empty = self.obj_caches.get(&from).is_none_or(|c| c.is_empty());
        // self appearance support | send all cacheable objects | cache empty (no probes)
        reply.region_info.flags = 0x4 | 0x1 | if cache_empty { 0x2 } else { 0 };
        self.send(from, &reply, true);
    }
}

/// LLHUDObject::LL_HUD_EFFECT_LOOKAT.
const HUD_EFFECT_LOOKAT: u8 = 14;
/// LLHUDEffectLookAt type data: source avatar (16), target object (16),
/// target offset or global position (3 x F64), look-at type (1).
const LOOKAT_PKT_SIZE: usize = 57;

fn lookat_effect(
    agent: uuid::Uuid,
    effect: uuid::Uuid,
    target: uuid::Uuid,
    offset: [f64; 3],
    kind: u8,
    duration: f32,
) -> msgs::viewer_effect::Effect {
    let mut d = Vec::with_capacity(LOOKAT_PKT_SIZE);
    d.extend_from_slice(agent.as_bytes());
    d.extend_from_slice(target.as_bytes());
    for v in offset {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.push(kind);
    msgs::viewer_effect::Effect {
        id: effect,
        agent_id: agent,
        type_: HUD_EFFECT_LOOKAT,
        duration,
        // LLHUDEffect::packData: RGBA (the look-at color is unused)
        color: vec![255, 255, 255, 255],
        type_data: d,
    }
}

fn parse_lookat(e: &msgs::viewer_effect::Effect) -> Option<(uuid::Uuid, uuid::Uuid, [f64; 3], u8)> {
    if e.type_ != HUD_EFFECT_LOOKAT || e.type_data.len() != LOOKAT_PKT_SIZE {
        return None;
    }
    let d = &e.type_data;
    let source = uuid::Uuid::from_slice(&d[0..16]).ok()?;
    let target = uuid::Uuid::from_slice(&d[16..32]).ok()?;
    let mut offset = [0.0f64; 3];
    for (i, v) in offset.iter_mut().enumerate() {
        let b: [u8; 8] = d[32 + i * 8..40 + i * 8].try_into().ok()?;
        *v = f64::from_le_bytes(b);
    }
    Some((source, target, offset, d[56]))
}
