//! The session task: owns the UDP socket and all simulator circuits,
//! drives login, capabilities, the event queue and agent updates.
//!
//! Message flow follows `newview/llstartup.cpp` and `llviewermessage.cpp`.

mod asset_imports;
mod build_cmds;
mod currency;
mod gestures;
mod inventory_copies;
mod inventory_moves;
mod inventory_preview;
mod inventory_updates;
mod inventory_upload;
mod object_actions;
mod object_cache;
mod scripts;
mod task_notecard;
mod wearable_save;

use crate::agent_update::AgentUpdateRule;
use crate::caps::{self, EqEvent, EqMessage};
use crate::circuit::{Circuit, Resend};
use crate::login::{self, LoginError, LoginRequest, LoginResponse};
use crate::objects;
use crate::stats::NetStats;
use crate::terrain;
use crate::throttle::{self, Throttle};
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
use uuid::Uuid;

/// Mute list, groups and chat sessions.
#[path = "session_social.rs"]
mod social_net;

/// About Land: parcel selection, lists, covenant (land.rs).
#[path = "session_land.rs"]
mod land_net;

/// Instant messages kept while offline and the answers to offline offers.
#[path = "session_offline.rs"]
mod offline_net;

/// Directory search: queries, replies, event and group details (search.rs).
#[path = "session_search.rs"]
mod search_net;
use crate::build::BUILD_PARCEL_SEQ;

const SIM_TIMEOUT: Duration = Duration::from_secs(60);
const PING_INTERVAL: Duration = Duration::from_secs(5);
/// Diagnostic: the RegionHandshake of a region the agent arrived in should
/// come along with AgentMovementComplete; without it the simulator never
/// sends terrain or objects (it waits for RegionHandshakeReply).
const HANDSHAKE_WARN_DELAY: Duration = Duration::from_secs(10);

struct Sim {
    circuit: Circuit,
    handle: RegionHandle,
    name: String,
    size: (u32, u32),
    seed: Option<String>,
    caps_requested: bool,
    caps: Arc<HashMap<String, String>>,
    eq_task: Option<JoinHandle<()>>,
    /// The request of the seed capability, while it is asked for.
    caps_task: Option<JoinHandle<()>>,
    last_ping: Instant,
    time_dilation: f32,
    /// UseCircuitCode sequence; CompleteAgentMovement waits for its ack.
    ucc_seq: Option<u32>,
    pending_cam: bool,
    ucc_sent_at: Instant,
    /// Login circuit: CompleteAgentMovement strictly after the UseCircuitCode
    /// ack, and the login fails if it never comes (llstartup.cpp).
    ucc_login: bool,
    /// RegionHandshake handled (the simulator then waits for our reply).
    handshake_seen: bool,
    /// AgentMovementComplete from this region, for the handshake diagnostic.
    arrived_at: Option<Instant>,
    handshake_warned: bool,
    /// Packets received and packets that failed to decode (diagnostics).
    packets_in: u64,
    decode_failures: u64,
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
            caps_task: None,
            last_ping: Instant::now(),
            time_dilation: 1.0,
            ucc_seq: None,
            pending_cam: false,
            ucc_sent_at: Instant::now(),
            ucc_login: false,
            handshake_seen: false,
            arrived_at: None,
            handshake_warned: false,
            packets_in: 0,
            decode_failures: 0,
        }
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        if let Some(t) = self.eq_task.take() {
            t.abort();
        }
        if let Some(t) = self.caps_task.take() {
            t.abort();
        }
    }
}

pub(crate) struct Shared {
    pub http: reqwest::Client,
    pub caps_http: reqwest::Client,
    pub events: crate::EventSender,
    pub controls: Arc<Mutex<AgentControls>>,
    pub stats: Arc<NetStats>,
    /// Directory of the per-region object caches (None = disabled).
    pub object_cache_dir: Arc<Mutex<Option<std::path::PathBuf>>>,
    pub agent_language: Arc<Mutex<(String, bool)>>,
    /// Bandwidth setting, in kb/s (`NetClient::set_bandwidth`).
    pub bandwidth_kbps: Arc<std::sync::atomic::AtomicU32>,
}

struct Session<'a> {
    sh: &'a Shared,
    sock: Arc<UdpSocket>,
    login: Arc<LoginResponse>,
    sims: HashMap<SocketAddr, Sim>,
    main: Option<SocketAddr>,
    eq_tx: mpsc::UnboundedSender<EqMessage>,
    /// (simulator, seed asked, its capabilities or why they did not come)
    caps_tx: mpsc::UnboundedSender<(SocketAddr, String, Result<HashMap<String, String>, String>)>,
    notecard_copy_tx: mpsc::UnboundedSender<(u32, Result<(), String>)>,
    inventory_move_tx: mpsc::UnboundedSender<inventory_moves::Request>,
    notecard_copies: HashMap<u32, (Uuid, Uuid, bool)>,
    /// When an AgentUpdate leaves (agent_update.rs).
    agent_update: AgentUpdateRule,
    /// Control flags of the last `NetCommand::AgentControls`.
    control_flags: u32,
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
    /// Object cache of each region, its files read and written off this
    /// loop (session/object_cache.rs).
    object_cache: object_cache::ObjectCaches,
    /// What the agent's simulator is asked to send at most (throttle.rs).
    throttle: Throttle,
    /// Mute list download, groups, chat sessions (session_social.rs).
    social: social_net::SocialNet,
    tasks: object_actions::TaskRequests,
    /// Covenant transfer of the About Land floater (session_land.rs).
    land: land_net::LandNet,
    /// Decode failures per message name, to rate-limit their warnings.
    decode_failures: HashMap<&'static str, u64>,
    inventory_uploads: inventory_upload::Uploads,
    asset_imports: asset_imports::Imports,
    inventory_previews: inventory_preview::Previews,
    script_transfers: scripts::ScriptTransfers,
    inventory_callbacks: HashMap<u32, (Instant, bool)>,
    inventory_copies: inventory_copies::Pending,
    inventory_merchant_pending: bool,
    next_inventory_callback: u32,
}

/// The packet counters the bandwidth asked is adjusted from.
fn packets_counted(stats: &NetStats) -> throttle::Counted {
    throttle::Counted {
        received: stats.packets_in.load(Ordering::Relaxed),
        gaps: stats.seq_gaps.load(Ordering::Relaxed),
        late: stats.seq_late.load(Ordering::Relaxed),
    }
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
            message: aurora_i18n::tr!("session-connecting-to-login-server").into(),
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
            message: aurora_i18n::tr!("common-connecting-to-region").into(),
            fraction: 0.3,
        },
    );

    let sock = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => {
            size_socket_buffers(&s);
            Arc::new(s)
        }
        Err(e) => {
            emit(
                sh,
                NetEvent::LoginFailed {
                    error: aurora_i18n::tr!("session-udp-bind-failed", e = format!("{}", e)),
                    mfa_required: false,
                    tos_required: false,
                    bad_credentials: false,
                },
            );
            return;
        }
    };

    // The session only learns the control flags from the changes handed to
    // it: start both sides from none, whatever the previous session left.
    *sh.controls.lock() = AgentControls::default();
    let (eq_tx, mut eq_rx) = mpsc::unbounded_channel();
    let (caps_tx, mut caps_rx) = mpsc::unbounded_channel();
    let (notecard_copy_tx, mut notecard_copy_rx) = mpsc::unbounded_channel();
    let (inventory_move_tx, mut inventory_move_rx) = mpsc::unbounded_channel();
    let (object_cache, mut object_cache_rx) = object_cache::ObjectCaches::new(sh.object_cache_dir.lock().clone());
    let mut s = Session {
        sh,
        sock: sock.clone(),
        login: login.clone(),
        sims: HashMap::new(),
        main: None,
        eq_tx,
        caps_tx,
        agent_update: AgentUpdateRule::new(Instant::now()),
        control_flags: 0,
        one_shot: 0,
        cam_warned: false,
        far: 128.0,
        logout_started: None,
        finished: false,
        agent_moved: false,
        cam_requested_at: None,
        agent_parcel: None,
        pending_named_tp: None,
        object_cache,
        throttle: Throttle::new(sh.bandwidth_kbps.load(Ordering::Relaxed) as f32, Instant::now()).counting_from(packets_counted(&sh.stats)),
        social: Default::default(),
        tasks: Default::default(),
        land: Default::default(),
        decode_failures: HashMap::new(),
        inventory_uploads: Default::default(),
        asset_imports: Default::default(),
        inventory_previews: Default::default(),
        script_transfers: Default::default(),
        inventory_callbacks: HashMap::new(),
        inventory_copies: inventory_copies::Pending::default(),
        notecard_copy_tx,
        inventory_move_tx,
        notecard_copies: HashMap::new(),
        inventory_merchant_pending: false,
        next_inventory_callback: 0,
    };

    log::info!(
        "simulator bandwidth: setting {:.0} kb/s, {:.0} asked at first ({:.0} for the objects)",
        s.throttle.setting(),
        s.throttle.asked(),
        throttle::group(s.throttle.asked())[4]
    );
    let addr = SocketAddr::V4(SocketAddrV4::new(login.sim_ip, login.sim_port));
    s.add_sim(addr, login.region_handle(), (login.region_size_x, login.region_size_y), true);
    s.set_main(addr, Some(login.seed_capability.clone()));
    s.complete_agent_movement(addr);

    let mut buf = vec![0u8; 16384];
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while !s.finished {
        // What woke the loop is handled before anything else is read: the
        // longest of these is how long the circuits went unserved
        // (`NetStats::loop_busy`, in the "net summary" log line).
        let woke;
        tokio::select! {
            r = sock.recv_from(&mut buf) => {
                woke = Instant::now();
                match r {
                    Ok((n, from)) => s.on_datagram(&buf[..n], from),
                    Err(e) => {
                        // Windows reports ICMP port unreachable as recv errors.
                        log::debug!("udp recv error: {e}");
                    }
                }
            }
            _ = tick.tick() => {
                woke = Instant::now();
                s.tick();
            }
            c = cmd_rx.recv() => {
                woke = Instant::now();
                match c {
                    Some(c) => s.on_command(c),
                    None => break,
                }
            }
            Some(m) = eq_rx.recv() => {
                woke = Instant::now();
                match m {
                    EqMessage::Event(e) => s.on_eq_event(e),
                    EqMessage::Lost(sim) => s.on_eq_lost(sim),
                }
            }
            Some((addr, seed, caps)) = caps_rx.recv() => {
                woke = Instant::now();
                s.on_caps(addr, &seed, caps);
            }
            Some((callback, result)) = notecard_copy_rx.recv() => {
                woke = Instant::now();
                s.notecard_copy_http(callback, result);
            }
            Some(request) = inventory_move_rx.recv() => {
                woke = Instant::now();
                s.inventory_move(request);
            }
            Some(file) = object_cache_rx.recv() => {
                woke = Instant::now();
                s.on_object_cache_loaded(file);
            }
        }
        sh.stats.loop_busy(woke);
    }
    s.object_cache.save();
    // Close every circuit politely.
    for (addr, sim) in s.sims.iter_mut() {
        let m = CloseCircuit::default();
        let _ = sim.circuit.send(&s.sock, &s.sh.stats, &m, false);
        log::debug!("closed circuit {addr}");
    }
    sh.stats.sim_count.store(0, Ordering::Relaxed);
    *sh.stats.simulator_motion.lock() = Default::default();
    sh.stats.simulators.lock().clear();
    // The next session reads the files this one leaves.
    s.object_cache.flush().await;
}

/// Buffers Firestorm asks for its socket (`RECEIVE_BUFFER_SIZE` and
/// `SEND_BUFFER_SIZE`, llmessage/net.cpp).
const RECEIVE_BUFFER_SIZE: usize = 800_000;
const SEND_BUFFER_SIZE: usize = 200_000;

/// Sizes the buffers of the circuits' socket like Firestorm's `start_net`.
/// The system's default (64 KiB on Windows) holds a few dozen datagrams of
/// object updates: on arrival the region and its neighbours send theirs at
/// once, and what does not fit is lost — resent a second later when it was
/// reliable, gone when it was a position update. Returns (receive, send) as
/// obtained.
fn size_socket_buffers(sock: &UdpSocket) -> (usize, usize) {
    let sock = socket2::SockRef::from(sock);
    let sizes = |sock: &socket2::SockRef| (sock.recv_buffer_size().unwrap_or(0), sock.send_buffer_size().unwrap_or(0));
    let before = sizes(&sock);
    if let Err(e) = sock.set_recv_buffer_size(RECEIVE_BUFFER_SIZE) {
        log::warn!("udp socket: receive buffer not resized ({e})");
    }
    if let Err(e) = sock.set_send_buffer_size(SEND_BUFFER_SIZE) {
        log::warn!("udp socket: send buffer not resized ({e})");
    }
    let after = sizes(&sock);
    log::info!(
        "udp socket buffers: receive {} bytes (system default {}), send {} bytes (system default {})",
        after.0,
        before.0,
        after.1,
        before.1
    );
    after
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

    /// `login`: the first region of the session, whose UseCircuitCode has
    /// llstartup.cpp's own resend policy.
    fn add_sim(&mut self, addr: SocketAddr, handle: RegionHandle, size: (u32, u32), login: bool) {
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
            let policy = if login { Resend::LOGIN_USE_CIRCUIT_CODE } else { Resend::DEFAULT };
            let seq = sim.circuit.send_reliable_with(&self.sock, &self.sh.stats, &m, policy);
            sim.ucc_seq = Some(seq);
            sim.ucc_sent_at = Instant::now();
            sim.ucc_login = login;
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
                self.update_agent_language();
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

        self.send_throttle(addr);

        let mut hw = AgentHeightWidth::default();
        hw.agent_data.agent_id = self.agent_id();
        hw.agent_data.session_id = self.session_id();
        hw.agent_data.circuit_code = self.login.circuit_code;
        hw.height_width_block.height = 1080;
        hw.height_width_block.width = 1920;
        self.send(addr, &hw, true);
    }

    /// AgentThrottle: what this simulator may send per second on each
    /// channel (LLViewerThrottleGroup::sendToSim).
    fn send_throttle(&mut self, addr: SocketAddr) {
        let mut t = AgentThrottle::default();
        t.agent_data.agent_id = self.agent_id();
        t.agent_data.session_id = self.session_id();
        t.agent_data.circuit_code = self.login.circuit_code;
        t.throttle.gen_counter = 0;
        t.throttle.throttles = self.throttle.bits_per_second().iter().flat_map(|f| f.to_le_bytes()).collect();
        self.send(addr, &t, true);
    }

    /// The bandwidth asked follows the setting, and the link every five
    /// seconds (LLViewerThrottle::updateDynamicThrottle); a change goes to
    /// the agent's simulator, as Firestorm's does.
    fn update_throttle(&mut self, now: Instant) {
        let setting = self.sh.bandwidth_kbps.load(Ordering::Relaxed) as f32;
        let mut changed = false;
        if setting != self.throttle.setting() {
            self.throttle = Throttle::new(setting, now).counting_from(packets_counted(&self.sh.stats));
            log::info!("simulator bandwidth: setting {setting:.0} kb/s, {:.0} asked", self.throttle.asked());
            changed = true;
        }
        self.throttle.note_backlog(self.sh.events.waiting());
        if let Some(lost) = self.throttle.update_from(now, packets_counted(&self.sh.stats)) {
            log::info!(
                "simulator bandwidth: {:.0} kb/s asked ({lost:.1} % of the packets lost in the last 5 s, {} events waiting)",
                self.throttle.asked(),
                self.sh.events.waiting()
            );
            changed = true;
        }
        if changed && let Some(main) = self.main {
            self.send_throttle(main);
        }
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
        // (one request per simulator: a new seed takes the place of the old)
        if let Some(old) = sim.caps_task.take() {
            old.abort();
        }
        sim.caps_task = Some(tokio::spawn(async move {
            let alive = tx.clone();
            let r = caps::resolve_seed(&http, &seed, caps::SeedRetry::FIRESTORM, move || !alive.is_closed())
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send((addr, seed, r));
        }));
    }

    fn on_caps(&mut self, addr: SocketAddr, seed: &str, r: Result<HashMap<String, String>, String>) {
        // the region has another seed since: the answer of a stale request
        // (mHttpResponderID in LLViewerRegionImpl::requestBaseCapabilitiesCoro)
        if self.sims.get(&addr).is_some_and(|sim| sim.seed.as_deref() != Some(seed)) {
            log::info!("capabilities for {addr}: answer to a previous seed ignored");
            return;
        }
        let caps = match r {
            Ok(c) => c,
            Err(e) => {
                log::warn!("capabilities for {addr} failed for good: {e}");
                // asked again when the region is entered again
                // (set_main), not before: every attempt was made
                if let Some(sim) = self.sims.get_mut(&addr) {
                    sim.caps_requested = false;
                }
                if self.main == Some(addr) {
                    emit(
                        self.sh,
                        NetEvent::Alert {
                            message: aurora_i18n::tr!(
                                "session-region-capabilities-are-unavailable-textures-inventory-and-teleports-will",
                                e = e.to_string()
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
                self.sh.stats.clone(),
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
            self.update_agent_language();
            // LLEnvironment::onRegionChange -> requestRegion once caps arrive
            self.request_environment(None);
            if self.inventory_merchant_pending {
                self.inventory_merchant_pending = false;
                self.on_command(NetCommand::RequestInventoryMerchant);
            }
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
                            avatar_hover_height: v["AvatarHoverHeightEnabled"].as_bool(),
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

    fn remove_sim(&mut self, addr: SocketAddr) {
        self.object_cache.forget(addr);
        if let Some(sim) = self.sims.remove(&addr) {
            emit(self.sh, NetEvent::RegionRemoved { handle: sim.handle });
        }
        if self.main == Some(addr) {
            self.main = None;
        }
        self.sh.stats.sim_count.store(self.sims.len() as u32, Ordering::Relaxed);
    }

    fn publish_motion(&self) {
        {
            let mut all = self.sh.stats.simulators.lock();
            all.clear();
            all.extend(self.sims.values().map(|sim| crate::stats::SimulatorMotion {
                handle: Some(sim.handle),
                time_dilation: sim.time_dilation,
                last_packet: Some(sim.circuit.last_recv),
            }));
        }
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
        self.expire_script_transfers();
        self.expire_inventory_uploads();
        self.expire_asset_imports();
        self.expire_inventory_previews();
        for completion in self.inventory_copies.expire(Instant::now()) {
            self.finish_inventory_copies(completion);
        }
        let before = self.inventory_callbacks.len();
        self.inventory_callbacks
            .retain(|_, (sent, _)| sent.elapsed() < Duration::from_secs(60));
        for (_, (editor, notecard, open)) in self
            .notecard_copies
            .extract_if(|callback, _| !self.inventory_callbacks.contains_key(callback))
        {
            emit(
                self.sh,
                NetEvent::NotecardInventoryCopied {
                    editor,
                    notecard,
                    open,
                    result: Err(aurora_i18n::tr!("session-a-creation-or-copy-was-not-confirmed-reload-the").into()),
                },
            );
        }
        if before != self.inventory_callbacks.len() {
            emit(
                self.sh,
                NetEvent::InventoryOperationFailed(aurora_i18n::tr!("session-a-creation-or-copy-was-not-confirmed-reload-the").into()),
            );
        }
        let now = Instant::now();
        self.publish_motion();
        self.object_cache.save_when_due(now);
        self.update_throttle(now);
        let stats = self.sh.stats.clone();
        // Deferred CompleteAgentMovement once UseCircuitCode is acked. At
        // login it waits for the ack itself, as llstartup.cpp STATE_WORLD_WAIT
        // does (gGotUseCircuitCodeAck); elsewhere 2 s are enough.
        let mut ready = Vec::new();
        let mut ucc_failed = false;
        for (addr, s) in self.sims.iter_mut() {
            let dropped = s.circuit.take_dropped();
            if !s.pending_cam {
                continue;
            }
            let ucc_dropped = s.ucc_seq.is_some_and(|q| dropped.contains(&q));
            if s.ucc_login && ucc_dropped {
                ucc_failed = true;
            } else if s.ucc_seq.is_none_or(|q| !s.circuit.is_unacked(q) && !ucc_dropped)
                || (!s.ucc_login && s.ucc_sent_at.elapsed() > Duration::from_secs(2))
            {
                ready.push(*addr);
            }
        }
        if ucc_failed {
            // use_circuit_callback: the region never acknowledged our circuit
            // (notification LoginPacketNeverReceived), back to the login screen
            log::warn!("UseCircuitCode never acknowledged by the login region: login abandoned");
            emit(
                self.sh,
                NetEvent::Disconnected {
                    reason: aurora_i18n::tr!("session-we-are-having-trouble-connecting-you-there-may-be").into(),
                },
            );
            self.finished = true;
            return;
        }
        for addr in ready {
            if let Some(sim) = self.sims.get_mut(&addr) {
                sim.pending_cam = false;
            }
            self.send_complete_agent_movement(addr);
        }
        self.check_region_handshake();
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
                        reason: aurora_i18n::tr!("session-the-region-did-not-respond-in-time-agentmovementcomplete").into(),
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
                        reason: aurora_i18n::tr!("session-connection-to-the-region-was-lost").into(),
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
        } else {
            // (a key pressed during a teleport is not an input delay)
            self.sh.stats.forget_control_change();
        }
    }

    /// Warn once when the region the agent arrived in has not sent its
    /// RegionHandshake: nothing else would show why terrain and objects
    /// never arrive. Firestorm has no recovery here (there is no message to
    /// request a handshake again): the simulator resends it until acked.
    fn check_region_handshake(&mut self) {
        let Some(sim) = self.main.and_then(|a| self.sims.get_mut(&a)) else {
            return;
        };
        if sim.handshake_seen || sim.handshake_warned || sim.arrived_at.is_none_or(|t| t.elapsed() < HANDSHAKE_WARN_DELAY) {
            return;
        }
        sim.handshake_warned = true;
        let stats = &self.sh.stats;
        log::warn!(
            "no RegionHandshake from {} {} s after AgentMovementComplete: no terrain or objects will arrive \
             (from this region: {} packets, {} decode failures; session: {} duplicates, {} reliable packets given up)",
            sim.circuit.addr,
            HANDSHAKE_WARN_DELAY.as_secs(),
            sim.packets_in,
            sim.decode_failures,
            stats.duplicates.load(Ordering::Relaxed),
            stats.dropped_reliable.load(Ordering::Relaxed),
        );
    }

    /// The tick's look at the controls: the keep-alive, the follow-up of a
    /// flag change, and what moved without being handed over. The flags are
    /// those of the last command, so that they only ever change in the
    /// order the viewer changed them.
    fn maybe_send_agent_update(&mut self, now: Instant) {
        let mut c = *self.sh.controls.lock();
        c.control_flags = self.control_flags;
        self.send_agent_update_if_due(c, now);
    }

    /// Controls handed over by the viewer because they changed enough to
    /// leave now (`NetCommand::AgentControls`).
    fn on_agent_controls(&mut self, c: AgentControls) {
        self.control_flags = c.control_flags;
        if self.agent_moved && self.logout_started.is_none() {
            self.send_agent_update_if_due(c, Instant::now());
        }
    }

    fn send_agent_update_if_due(&mut self, mut c: AgentControls, now: Instant) {
        c.far = self.far;
        c.control_flags |= self.one_shot;
        let flags_changed = c.control_flags != self.agent_update.sent_flags();
        if self.agent_update.should_send(&c, now) {
            self.sh.stats.agent_update_sent(flags_changed);
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
            self.one_shot = 0;
        }
    }

    // -------------------------------------------------------------- commands

    fn on_command(&mut self, c: NetCommand) {
        if self.on_object_action_command(&c) || self.on_social_command(&c) || self.on_offline_command(&c) || self.on_search_command(&c) {
            return;
        }
        match c {
            NetCommand::UpdateAgentLanguage => self.update_agent_language(),
            NetCommand::SetHoverHeight { region, request, value } => self.set_hover_height(region, request, value),
            NetCommand::SetMaturityPreference(maturity) => self.set_maturity_preference(maturity),
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
            NetCommand::AcceptLure { lure_id, godlike } => {
                let mut m = TeleportLureRequest::default();
                m.info.agent_id = self.agent_id();
                m.info.session_id = self.session_id();
                m.info.lure_id = lure_id;
                // TELEPORT_FLAGS_VIA_GODLIKE_LURE | DISABLE_CANCEL, else VIA_LURE
                m.info.teleport_flags = if godlike { (1 << 8) | (1 << 11) } else { 1 << 2 };
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
            NetCommand::RemoteParcelRequest {
                handle,
                position,
                region_id,
            } => self.remote_parcel_request(handle, position, region_id),
            NetCommand::RegionHandleRequest(region_id) => {
                let mut m = RegionHandleRequest::default();
                m.request_block.region_id = region_id;
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
                m.target_data
                    .extend(to.into_iter().map(|target_id| start_lure::TargetData { target_id }));
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
            NetCommand::EditInventory { request, change } => {
                if change.operations.is_empty() && !change.copies.is_empty() && change.copies.len() <= 2048 && !change.refresh.is_empty() {
                    emit(
                        self.sh,
                        NetEvent::InventoryEdited {
                            request,
                            result: Ok(crate::inventory::operations::Reply {
                                contents: Vec::new(),
                                folders: Vec::new(),
                                moves_queued: false,
                                refresh: change.refresh,
                                copies: change.copies,
                                removed: change.removed,
                                created: HashMap::new(),
                            }),
                        },
                    );
                    return;
                }
                if change.move_only() {
                    let result = change
                        .operations
                        .iter()
                        .try_for_each(|op| {
                            let crate::inventory::operations::Operation::Patch { id, folder, body } = op else {
                                return Ok(());
                            };
                            let parent = body["parent_id"].as_uuid();
                            if id.is_nil() || parent.is_nil() {
                                return Err(aurora_i18n::tr!("inventory-move-invalid-target").into());
                            }
                            self.queue_inventory_move(crate::inventory::operations::InventoryMove {
                                id: *id,
                                parent,
                                folder: *folder,
                            })
                        })
                        .map(|()| crate::inventory::operations::Reply {
                            contents: Vec::new(),
                            folders: Vec::new(),
                            moves_queued: true,
                            refresh: change.refresh,
                            copies: Vec::new(),
                            removed: change.removed,
                            created: HashMap::new(),
                        });
                    emit(self.sh, NetEvent::InventoryEdited { request, result });
                    return;
                }
                let folder_creation = (change.defer_folder_contents || !change.copies.is_empty())
                    && change
                        .operations
                        .iter()
                        .all(|op| matches!(op, crate::inventory::operations::Operation::Folder { .. }));
                let fetch = if folder_creation {
                    Some(String::new())
                } else {
                    self.main_cap("FetchInventoryDescendents2")
                };
                let Some((cap, fetch_cap)) = self.main_cap("InventoryAPIv3").zip(fetch) else {
                    emit(
                        self.sh,
                        NetEvent::InventoryEdited {
                            request,
                            result: Err(aurora_i18n::tr!("session-the-inventory-server-is-not-available-yet").into()),
                        },
                    );
                    return;
                };
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                let owner = self.agent_id();
                let moves = self.inventory_move_tx.clone();
                tokio::spawn(async move {
                    let result = crate::inventory::operations::mutate_with_progress(
                        &http,
                        &cap,
                        &fetch_cap,
                        owner,
                        change,
                        |movement| inventory_moves::submit(moves.clone(), movement),
                        |progress| {
                            let _ = events.send(NetEvent::InventoryProgress { request, progress });
                        },
                    )
                    .await;
                    let _ = events.send(NetEvent::InventoryEdited { request, result });
                });
            }
            NetCommand::UploadInventoryThumbnail {
                request,
                item,
                folder,
                parent,
                data,
            } => {
                let caps = self
                    .main_cap("InventoryThumbnailUpload")
                    .zip(self.main_cap("InventoryAPIv3"))
                    .zip(self.main_cap("FetchInventoryDescendents2"));
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                let owner = self.agent_id();
                tokio::spawn(async move {
                    let result = async {
                        let ((upload, ais), fetch) = caps
                            .ok_or_else(|| aurora_i18n::tr!("session-the-server-does-not-support-uploading-inventory-images").to_owned())?;
                        let asset = crate::inventory::thumbnail::upload(&http, &upload, item, folder, data).await?;
                        crate::inventory::operations::mutate(
                            &http,
                            &ais,
                            &fetch,
                            owner,
                            crate::inventory::thumbnail::patch(item, folder, parent, asset),
                        )
                        .await
                    }
                    .await;
                    let _ = events.send(NetEvent::InventoryEdited { request, result });
                });
            }
            NetCommand::UploadAsset { request, upload } => self.import_asset(request, upload),
            NetCommand::QuoteMesh { request, upload } => self.quote_mesh(request, upload),
            NetCommand::UploadMesh { request, token } => self.upload_mesh(request, token),
            NetCommand::DiscardMeshQuote(token) => self.discard_mesh_quote(token),
            NetCommand::RequestBalance => {
                let mut message = MoneyBalanceRequest::default();
                message.agent_data.agent_id = self.agent_id();
                message.agent_data.session_id = self.session_id();
                self.send_main(&message, true);
            }
            NetCommand::CurrencyQuote { helper, amount } => self.currency_quote(helper, amount),
            NetCommand::BuyCurrency { helper, quote, password } => self.buy_currency(helper, quote, password),
            NetCommand::FetchInventoryThumbnail { request, asset, resize } => {
                let cap = self.main_cap("ViewerAsset");
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                tokio::spawn(async move {
                    let result = if let Some(cap) = cap {
                        crate::inventory::operations::preview(&http, &cap, "texture_id", asset).await
                    } else {
                        Err(aurora_i18n::tr!("session-this-region-does-not-support-uploading-this-image").into())
                    };
                    let _ = events.send(NetEvent::InventoryThumbnailSource {
                        request,
                        asset,
                        resize,
                        result,
                    });
                });
            }
            NetCommand::RequestInventoryMerchant => {
                if self.main.and_then(|a| self.sims.get(&a)).is_none_or(|s| s.caps.is_empty()) {
                    self.inventory_merchant_pending = true;
                    return;
                }
                let events = self.sh.events.clone();
                let http = self.sh.caps_http.clone();
                let cap = self.main_cap("DirectDelivery");
                tokio::spawn(async move {
                    let result = if let Some(cap) = cap {
                        match http
                            .get(format!("{}/merchant", cap.trim_end_matches('/')))
                            .timeout(Duration::from_secs(20))
                            .send()
                            .await
                        {
                            Ok(r) if r.status().is_success() => Ok(true),
                            Ok(r) if matches!(r.status().as_u16(), 404 | 503) => Ok(false),
                            _ => Err(aurora_i18n::tr!("session-the-marketplace-service-is-unavailable").into()),
                        }
                    } else {
                        Ok(false)
                    };
                    let _ = events.send(NetEvent::InventoryMerchant(result));
                });
            }
            NetCommand::PreviewInventoryItem(it) => {
                // Protected inventory assets require the simulator to check
                // the item permissions (LLAssetStorage::getInvItemAsset).
                if matches!(it.asset_type, 7 | 10) {
                    self.preview_inventory_document(it);
                    return;
                }
                let Some(cap) = self.main_cap("ViewerAsset") else {
                    emit(
                        self.sh,
                        NetEvent::InventoryPreview {
                            item: it.id,
                            asset: it.asset_id,
                            result: Err(aurora_i18n::tr!("session-this-region-does-not-support-uploading-this-content").into()),
                        },
                    );
                    return;
                };
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                tokio::spawn(async move {
                    let kind = match it.asset_type {
                        13 => "bodypart_id",
                        5 => "clothing_id",
                        21 => "gesture_id",
                        56 => "settings_id",
                        57 => "material_id",
                        _ => "",
                    };
                    let result = crate::inventory::operations::preview(&http, &cap, kind, it.asset_id).await;
                    let _ = events.send(NetEvent::InventoryPreview {
                        item: it.id,
                        asset: it.asset_id,
                        result,
                    });
                });
            }
            NetCommand::LoadGesture { item, asset } => self.load_gesture(item, asset),
            NetCommand::SetGestureActive { item, asset, active } => self.set_gesture_active(item, asset, active),
            NetCommand::GestureSound { asset, position } => self.gesture_sound(asset, position),
            NetCommand::PreviewTaskNotecard { handle, object, item } => self.preview_task_notecard(handle, object, *item),
            NetCommand::SaveTaskNotecard {
                request,
                handle,
                object,
                item,
                data,
            } => self.save_task_notecard(request, handle, object, item, data),
            NetCommand::LoadScript {
                request,
                target,
                handle,
                owner,
                asset,
            } => self.load_script(request, target, handle, owner, asset),
            NetCommand::LoadScriptExperiences { request, target, handle } => self.load_script_experiences(request, target, handle),
            NetCommand::SaveScript {
                request,
                target,
                handle,
                options,
                source,
            } => self.save_script(request, target, handle, options, source),
            NetCommand::SaveWearable {
                request,
                item,
                data,
                copy_name,
            } => self.save_wearable(request, *item, data, copy_name),
            NetCommand::CreateInventoryWearable { parent, kind, name, data } => self.create_wearable(parent, kind, name, data),
            NetCommand::RestoreInventoryObject(it) => {
                let mut m = RezRestoreToWorld::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.inventory_data.item_id = it.id;
                m.inventory_data.folder_id = it.parent;
                m.inventory_data.creator_id = it.creator;
                m.inventory_data.owner_id = it.owner;
                m.inventory_data.base_mask = it.base_mask;
                m.inventory_data.owner_mask = it.owner_mask;
                m.inventory_data.group_mask = it.group_mask;
                m.inventory_data.everyone_mask = it.everyone_mask;
                m.inventory_data.next_owner_mask = it.next_owner_mask;
                m.inventory_data.type_ = it.asset_type as i8;
                m.inventory_data.inv_type = it.inv_type as i8;
                m.inventory_data.flags = it.flags;
                m.inventory_data.name = str_field(&it.name);
                m.inventory_data.description = str_field(&it.desc);
                m.inventory_data.creation_date = it.created_at as i32;
                m.inventory_data.group_id = it.group_id;
                m.inventory_data.group_owned = it.group_owned;
                m.inventory_data.sale_type = it.sale_type;
                m.inventory_data.sale_price = it.sale_price;
                m.inventory_data.crc = crate::inventory::operations::checksum(&it);
                self.send_main(&m, true);
            }
            NetCommand::SaveInventoryContent {
                request,
                item,
                asset_type,
                data,
            } => {
                let name = match asset_type {
                    7 => "UpdateNotecardAgentInventory",
                    10 => "UpdateScriptAgent",
                    21 => "UpdateGestureAgentInventory",
                    _ => "",
                };
                let Some((cap, fetch)) = self.main_cap(name).zip(self.main_cap("FetchInventory2")) else {
                    emit(
                        self.sh,
                        NetEvent::InventoryContentSaved {
                            request,
                            item,
                            result: Err(aurora_i18n::tr!("session-saving-is-unavailable-in-this-region").into()),
                        },
                    );
                    return;
                };
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                let owner = self.agent_id();
                tokio::spawn(async move {
                    let result = crate::inventory::operations::save_content(&http, &cap, &fetch, owner, item, asset_type, data).await;
                    let _ = events.send(NetEvent::InventoryContentSaved { request, item, result });
                });
            }
            NetCommand::CopyInventoryItems(items) => {
                for it in items {
                    let callback = self.inventory_callback(false);
                    let m = inventory_copies::message(self.agent_id(), self.session_id(), callback, &it);
                    self.send_main(&m, true);
                }
            }
            NetCommand::FinishInventoryCopies { request, reply } => self.start_inventory_copies(request, reply),
            NetCommand::CopyNotecardInventory {
                editor,
                open,
                handle,
                object,
                notecard,
                item,
            } => {
                let cap = handle
                    .and_then(|handle| self.sim_for_handle(handle))
                    .or(self.main)
                    .and_then(|addr| self.sims.get(&addr))
                    .and_then(|sim| sim.caps.get("CopyInventoryFromNotecard"))
                    .cloned();
                let Some(cap) = cap else {
                    emit(
                        self.sh,
                        NetEvent::NotecardCopySubmitted {
                            editor,
                            notecard,
                            result: Err(aurora_i18n::tr!("notecard-copy-unavailable").into()),
                        },
                    );
                    return;
                };
                let callback = self.inventory_callback(false);
                self.notecard_copies.insert(callback, (editor, notecard, open));
                let body = crate::inventory::notecard::copy_body(notecard, object, item.id, callback);
                let http = self.sh.caps_http.clone();
                let replies = self.notecard_copy_tx.clone();
                tokio::spawn(async move {
                    let result = caps::post_llsd(&http, &cap, &body)
                        .await
                        .map(|_| ())
                        .map_err(|_| aurora_i18n::tr!("notecard-copy-failed").to_owned());
                    let _ = replies.send((callback, result));
                });
            }
            NetCommand::CreateInventoryItem { parent, kind, name } => {
                let (asset, inventory, subtype) = kind.types();
                let mut m = CreateInventoryItem::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.inventory_block.folder_id = parent;
                m.inventory_block.callback_id = self.inventory_callback(true);
                m.inventory_block.next_owner_mask = 0x7fffffff;
                m.inventory_block.type_ = asset;
                m.inventory_block.inv_type = inventory;
                m.inventory_block.wearable_type = subtype;
                m.inventory_block.name = str_field(&name);
                self.send_main(&m, true);
            }
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
            NetCommand::DetachAttachments(items) => {
                for item_id in items {
                    let mut m = DetachAttachmentIntoInv::default();
                    m.object_data.agent_id = self.agent_id();
                    m.object_data.item_id = item_id;
                    self.send_main(&m, true);
                }
            }
            NetCommand::SetInventoryFavorite { item, favorite } => {
                let caps = self.main_cap("InventoryAPIv3").zip(self.main_cap("FetchInventory2"));
                let Some((cap, fetch_cap)) = caps else {
                    emit(
                        self.sh,
                        NetEvent::InventoryFavoriteUpdated {
                            item,
                            result: Err(aurora_i18n::tr!("session-the-inventory-server-is-not-available-yet").into()),
                        },
                    );
                    return;
                };
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                let owner = self.agent_id();
                tokio::spawn(async move {
                    let result = crate::outfits::set_favorite(&http, &cap, &fetch_cap, owner, item, favorite).await;
                    let _ = events.send(NetEvent::InventoryFavoriteUpdated { item, result });
                });
            }
            NetCommand::UpdateOutfitCategory { request, change } => {
                let caps = self.main_cap("InventoryAPIv3").zip(self.main_cap("FetchInventoryDescendents2"));
                let Some((cap, fetch_cap)) = caps else {
                    emit(
                        self.sh,
                        NetEvent::OutfitUpdated {
                            request,
                            result: Err(aurora_i18n::tr!("session-the-inventory-server-is-not-available-yet").into()),
                        },
                    );
                    return;
                };
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                let owner = self.agent_id();
                tokio::spawn(async move {
                    let result = crate::outfits::categories::mutate(&http, &cap, &fetch_cap, owner, change).await;
                    let _ = events.send(NetEvent::OutfitUpdated { request, result });
                });
            }
            NetCommand::UpdateOutfit { request, change } => {
                let caps = self.main_cap("InventoryAPIv3").zip(self.main_cap("FetchInventoryDescendents2"));
                let Some((cap, fetch_cap)) = caps else {
                    emit(
                        self.sh,
                        NetEvent::OutfitUpdated {
                            request,
                            result: Err(aurora_i18n::tr!("session-the-inventory-server-is-not-available-yet").into()),
                        },
                    );
                    return;
                };
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                let owner = self.agent_id();
                tokio::spawn(async move {
                    let result = crate::outfits::mutate(&http, &cap, &fetch_cap, owner, change).await;
                    let _ = events.send(NetEvent::OutfitUpdated { request, result });
                });
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
            NetCommand::AgentControls(c) => self.on_agent_controls(c),
            NetCommand::ClearObjectCache => self.object_cache.clear(),
            NetCommand::Build(b) => self.on_build(b),
            // handled by on_social_command
            NetCommand::RequestMuteList { .. }
            | NetCommand::UpdateMute { .. }
            | NetCommand::RemoveMute { .. }
            | NetCommand::RequestGroups
            | NetCommand::ActivateGroup(_)
            | NetCommand::LeaveGroup(_)
            | NetCommand::SetGroupFlags { .. }
            | NetCommand::TerminateFriendship(_)
            | NetCommand::AgentAnimation { .. }
            | NetCommand::ObjectGrab { .. }
            | NetCommand::ObjectGrabUpdate { .. }
            | NetCommand::ObjectRelease { .. }
            | NetCommand::RequestTaskInventory { .. }
            | NetCommand::ChatSession { .. } => {}
            // handled by on_offline_command
            NetCommand::RequestOfflineMessages
            | NetCommand::RequestOfflineMessagesLegacy
            | NetCommand::AnswerOfflineFriendship { .. }
            | NetCommand::AnswerOfflineGroupInvite { .. } => {}
            // handled by on_search_command
            NetCommand::DirQuery { .. }
            | NetCommand::EventInfoRequest(_)
            | NetCommand::GroupProfileRequest(_)
            | NetCommand::ProductInfoRequest => {}
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
                .header("Accept-Language", aurora_i18n::language())
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
                        reason: aurora_i18n::tr!("session-the-region-does-not-provide-the-agentprofile-capability").into(),
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

    /// LLAgentLanguage::update (indra/newview/llagentlanguage.cpp).
    /// Repeat after entering a region and after language/privacy changes.
    fn update_agent_language(&self) {
        let Some(url) = self.main_cap("UpdateAgentLanguage") else { return };
        let (language, public) = self.sh.agent_language.lock().clone();
        let body = caps::agent_language_body(&language, public);
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            if caps::post_llsd(&http, &url, &body).await.is_err() {
                log::warn!("Could not update agent language");
            }
        });
    }

    fn main_cap(&self, name: &str) -> Option<String> {
        self.main.and_then(|a| self.sims.get(&a)).and_then(|s| s.caps.get(name).cloned())
    }

    /// LLAgent::sendMaturityPreferenceToServer: always answered, a missing
    /// capability or a failed request as None (handlePreferredMaturityError).
    fn set_maturity_preference(&self, maturity: u8) {
        let url = self.main_cap("UpdateAgentInformation");
        let body = caps::maturity_preference_body(maturity);
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = match url {
                Some(url) => caps::post_llsd(&http, &url, &body)
                    .await
                    .ok()
                    .map(|reply| caps::maturity_preference_reply(&reply)),
                None => None,
            };
            if result.is_none() {
                log::warn!("Could not update the maturity preference");
            }
            let _ = events.send(NetEvent::MaturityPreference(result));
        });
    }

    /// LLVOAvatarSelf::sendHoverHeight. A delayed command must never write
    /// to a different region after teleporting; never log a capability URL.
    fn set_hover_height(&self, region: RegionHandle, request: u64, value: f32) {
        let url = self
            .main
            .and_then(|address| self.sims.get(&address))
            .filter(|sim| sim.handle == region)
            .and_then(|sim| sim.caps.get("AgentPreferences"))
            .cloned();
        let body = caps::hover_height_body(value);
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let success = if let (Some(url), Some(body)) = (url, body) {
                tokio::time::timeout(std::time::Duration::from_secs(20), caps::post_llsd(&http, &url, &body))
                    .await
                    .is_ok_and(|result| result.is_ok())
            } else {
                false
            };
            if !success {
                log::warn!("Could not update avatar hover height");
            }
            let _ = events.send(NetEvent::HoverHeightUpdated { region, request, success });
        });
    }

    fn notecard_copy_http(&mut self, callback: u32, result: Result<(), String>) {
        let Some((editor, notecard, open)) = self.notecard_copies.get(&callback).copied() else {
            return;
        };
        if let Err(reason) = result {
            self.notecard_copies.remove(&callback);
            self.inventory_callbacks.remove(&callback);
            emit(
                self.sh,
                NetEvent::NotecardInventoryCopied {
                    editor,
                    notecard,
                    open,
                    result: Err(reason),
                },
            );
        } else {
            emit(
                self.sh,
                NetEvent::NotecardCopySubmitted {
                    editor,
                    notecard,
                    result: Ok(()),
                },
            );
        }
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

    /// The event queue of a simulator gave up. Like LLEventPollImpl (the end
    /// of eventPollCoro): without the queue of the agent's region teleports,
    /// IM sessions, parcel events and region crossings all fail and nothing
    /// gets better before a new login, so the session ends now with the
    /// "connection lost" message; a neighbour only loses its own events.
    fn on_eq_lost(&mut self, addr: SocketAddr) {
        // (a queue replaced since, after a new seed capability, is not lost)
        if !self
            .sims
            .get(&addr)
            .is_some_and(|sim| sim.eq_task.as_ref().is_some_and(JoinHandle::is_finished))
        {
            return;
        }
        if self.main != Some(addr) {
            log::warn!("event queue of neighbour {addr} lost: it stays without its events");
            return;
        }
        log::warn!("event queue of the agent's region {addr} lost: disconnecting");
        emit(
            self.sh,
            NetEvent::Disconnected {
                reason: aurora_i18n::tr!("session-connection-to-the-region-was-lost").into(),
            },
        );
        self.finished = true;
    }

    fn on_eq_event(&mut self, e: EqEvent) {
        log::debug!("EQ {} from {}", e.message, e.sim);
        let b = &e.body;
        if self.on_inventory_eq(&e) {
            return;
        }
        if self.on_social_eq(&e.message, b)
            || self.on_land_eq(&e.message, b)
            || self.on_search_eq(&e.message, b)
            || self.on_build_eq(&e.message, b)
        {
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
                    self.add_sim(addr, handle, size, false);
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
        self.add_sim(addr, handle, size, false);
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
            log::debug!("packet from {from}, which is not one of our circuits");
            return;
        };
        sim.packets_in += 1;
        if !sim.circuit.on_receive(pkt.flags, pkt.sequence, &pkt.acks) {
            stats.duplicates.fetch_add(1, Ordering::Relaxed);
            return;
        }
        sim.circuit.note_sequence(pkt.sequence, stats);
        match self.dispatch(from, &pkt) {
            // ack only now (the handler may also have removed the circuit)
            Ok(()) => {
                if let Some(sim) = self.sims.get_mut(&from) {
                    sim.circuit.accept(pkt.flags, pkt.sequence);
                }
            }
            Err(e) => self.on_decode_failure(from, &pkt, &e),
        }
    }

    /// A known message that does not decode is not acked (the simulator
    /// resends it, as for LL's message system) and is logged: a lost
    /// RegionHandshake, for one, leaves the region without terrain or
    /// objects. Warnings are rate-limited per message.
    fn on_decode_failure(&mut self, from: SocketAddr, pkt: &IncomingPacket, e: &aurora_msg::DecodeError) {
        self.sh.stats.decode_errors.fetch_add(1, Ordering::Relaxed);
        if let Some(sim) = self.sims.get_mut(&from) {
            sim.decode_failures += 1;
        }
        let name = aurora_msg::message_info(pkt.id).map(|i| i.name).unwrap_or("?");
        let n = self.decode_failures.entry(name).or_default();
        *n += 1;
        if *n <= 3 || n.is_multiple_of(100) {
            log::warn!(
                "failed to decode {name} from {from} (#{n}, seq {}{}{}, {} body bytes): {e}",
                pkt.sequence,
                if pkt.reliable() {
                    ", reliable: not acked, will be resent"
                } else {
                    ""
                },
                if pkt.resent() { ", resent" } else { "" },
                pkt.body.len(),
            );
        }
    }

    fn handle_of(&self, addr: SocketAddr) -> RegionHandle {
        self.sims.get(&addr).map(|s| s.handle).unwrap_or(0)
    }

    fn dispatch(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<(), aurora_msg::DecodeError> {
        let id = pkt.id;
        if self.dispatch_inventory_upload(from, pkt)?
            || self.dispatch_inventory_preview(from, pkt)?
            || self.dispatch_script_transfer(from, pkt)?
            || self.dispatch_object_actions(from, pkt)?
            || self.dispatch_social(from, pkt)?
            || self.dispatch_land(from, pkt)?
            || self.dispatch_search(from, pkt)?
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
                self.sh
                    .stats
                    .ping_avg_ms
                    .store(sim.circuit.rtt.as_millis() as u32, Ordering::Relaxed);
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
            let mut cache = self.object_cache.get_mut(from);
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
                let cache = self.object_cache.get(from);
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
                        if let Some(c) = self.object_cache.get_mut(from) {
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
            if let Some(grid) = terrain::decode_wind_layer(m.layer_id.type_, &m.layer_data.data) {
                emit(
                    self.sh,
                    NetEvent::Wind {
                        handle: self.handle_of(from),
                        grid: Box::new(grid),
                    },
                );
            }
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
        } else if id == SetFollowCamProperties::ID || id == ClearFollowCamProperties::ID {
            if let Some(event) = follow_cam_event(pkt, self.handle_of(from))? {
                emit(self.sh, event);
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
        } else if id == RegionIDAndHandleReply::ID {
            let m: RegionIDAndHandleReply = pkt.decode()?;
            let b = &m.reply_block;
            emit(
                self.sh,
                NetEvent::RegionIdHandle {
                    region_id: b.region_id,
                    handle: b.region_handle,
                },
            );
        } else if id == ParcelInfoReply::ID {
            let m: ParcelInfoReply = pkt.decode()?;
            let d = &m.data;
            // LLRemoteParcelInfoProcessor::processParcelInfoReply
            let p = crate::ParcelSummary {
                id: d.parcel_id,
                owner: d.owner_id,
                name: field_str(&d.name),
                desc: field_str(&d.desc),
                actual_area: d.actual_area,
                billable_area: d.billable_area,
                flags: d.flags,
                sim_name: field_str(&d.sim_name),
                global: glam::DVec3::new(d.global_x as f64, d.global_y as f64, d.global_z as f64),
                snapshot: d.snapshot_id,
                dwell: d.dwell,
                sale_price: d.sale_price,
                auction_id: d.auction_id,
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
                sim.arrived_at.get_or_insert_with(Instant::now);
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
            self.send(from, &EconomyDataRequest::default(), true);
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
                        timestamp: b.timestamp,
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
                    attachments: m
                        .attachment_block
                        .iter()
                        .filter(|a| !a.id.is_nil())
                        .map(|a| a.id)
                        .collect::<std::collections::HashSet<_>>()
                        .len() as u32,
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
                        reason: aurora_i18n::tr!("session-the-region-closed-the-connection").into(),
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
                                reason: aurora_i18n::tr!("session-region-not-found", want = want.to_string()),
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
        } else if id == GenericMessage::ID {
            let m: GenericMessage = pkt.decode()?;
            if let Some(parameter) = m.param_list.first() {
                self.on_inventory_generic(m.agent_data.agent_id, &m.method_data.method, &parameter.parameter);
            }
        } else if id == LargeGenericMessage::ID {
            let m: LargeGenericMessage = pkt.decode()?;
            if let Some(parameter) = m.param_list.first() {
                self.on_inventory_generic(m.agent_data.agent_id, &m.method_data.method, &parameter.parameter);
            }
        } else if id == UpdateCreateInventoryItem::ID {
            let m: UpdateCreateInventoryItem = pkt.decode()?;
            if m.agent_data.agent_id == self.agent_id() {
                let mut created = Vec::new();
                let items = m
                    .inventory_data
                    .iter()
                    .map(|it| {
                        let item = crate::inventory::operations::item_from_udp(it);
                        self.asset_created(it.callback_id, &item);
                        self.accept_inventory_copy(it.callback_id, &item);
                        if let Some((editor, notecard, open)) = self.notecard_copies.remove(&it.callback_id) {
                            emit(
                                self.sh,
                                NetEvent::NotecardInventoryCopied {
                                    editor,
                                    notecard,
                                    open,
                                    result: Ok(item.clone()),
                                },
                            );
                        }
                        if self.inventory_callbacks.remove(&it.callback_id).is_some_and(|(_, new)| new) {
                            created.push(item.clone());
                        }
                        item
                    })
                    .collect();
                emit(
                    self.sh,
                    NetEvent::InventoryUpdated {
                        folders: Vec::new(),
                        items,
                    },
                );
                for item in created {
                    emit(self.sh, NetEvent::InventoryCreated(item));
                }
            }
        } else if id == BulkUpdateInventory::ID {
            let m: BulkUpdateInventory = pkt.decode()?;
            if m.agent_data.agent_id == self.agent_id() {
                let mut created = Vec::new();
                let items = m
                    .item_data
                    .iter()
                    .map(|it| {
                        let new = self.inventory_callbacks.remove(&it.callback_id).is_some_and(|(_, new)| new);

                        let item = crate::inventory::InvItem {
                            id: it.item_id,
                            parent: it.folder_id,
                            asset_id: it.asset_id,
                            name: field_str(&it.name),
                            desc: field_str(&it.description),
                            asset_type: i32::from(it.type_),
                            inv_type: i32::from(it.inv_type),
                            flags: it.flags,
                            creator: it.creator_id,
                            owner: it.owner_id,
                            base_mask: it.base_mask,
                            owner_mask: it.owner_mask,
                            group_id: it.group_id,
                            group_owned: it.group_owned,
                            group_mask: it.group_mask,
                            everyone_mask: it.everyone_mask,
                            next_owner_mask: it.next_owner_mask,
                            sale_type: it.sale_type,
                            sale_price: it.sale_price,
                            created_at: i64::from(it.creation_date),
                            ..Default::default()
                        };
                        self.accept_inventory_copy(it.callback_id, &item);
                        if new {
                            created.push(item.clone());
                        }
                        self.asset_created(it.callback_id, &item);
                        if let Some((editor, notecard, open)) = self.notecard_copies.remove(&it.callback_id) {
                            emit(
                                self.sh,
                                NetEvent::NotecardInventoryCopied {
                                    editor,
                                    notecard,
                                    open,
                                    result: Ok(item.clone()),
                                },
                            );
                        }
                        item
                    })
                    .collect();
                let folders = m
                    .folder_data
                    .iter()
                    .filter(|f| !f.folder_id.is_nil())
                    .map(|f| crate::inventory::InvFolder {
                        id: f.folder_id,
                        parent: f.parent_id,
                        name: field_str(&f.name),
                        type_default: i32::from(f.type_),
                        version: -1,
                        ..Default::default()
                    })
                    .collect();
                emit(self.sh, NetEvent::InventoryUpdated { folders, items });
                for item in created {
                    emit(self.sh, NetEvent::InventoryCreated(item));
                }
            }
        } else if id == AvatarAnimation::ID {
            let m: AvatarAnimation = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::AvatarAnimations {
                    avatar: m.sender.id,
                    anims: m.animation_list.iter().map(|a| (a.anim_id, a.anim_sequence_id)).collect(),
                    sources: m.animation_source_list.iter().map(|s| s.object_id).collect(),
                },
            );
        } else if id == ObjectAnimation::ID {
            let m: ObjectAnimation = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::AvatarAnimations {
                    avatar: m.sender.id,
                    anims: m.animation_list.iter().map(|a| (a.anim_id, a.anim_sequence_id)).collect(),
                    sources: Vec::new(),
                },
            );
        } else if id == EconomyData::ID {
            let message: EconomyData = pkt.decode()?;
            if message.info.price_upload >= 0 {
                emit(self.sh, NetEvent::UploadCost(message.info.price_upload));
            }
        } else if id == MoneyBalanceReply::ID {
            let m: MoneyBalanceReply = pkt.decode()?;
            self.asset_imports.balance = Some(m.money_data.money_balance);
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
            self.agent_update.force();
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
                sim.handshake_seen = true;
                (sim.handle, sim.size)
            }
            None => return,
        };
        log::info!(
            "region handshake from {from}: {}{}",
            field_str(&r.sim_name),
            if is_main { " (main)" } else { "" }
        );
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
                    message: aurora_i18n::tr!("session-joined-region", value0 = field_str(&r.sim_name).to_string()),
                    fraction: 0.6,
                },
            );
        }
        // The region's object cache is loaded before the simulator starts
        // sending (LLViewerRegion::loadObjectCache): the reply tells it
        // whether there is one.
        self.answer_region_handshake(from, m.region_info2.region_id);
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

/// Object-sourced messages from established circuits, as in
/// process_set/clear_follow_cam_properties. An old region can still clear
/// its source after the agent crosses into a neighbour.
fn follow_cam_event(pkt: &IncomingPacket, handle: RegionHandle) -> Result<Option<NetEvent>, aurora_msg::DecodeError> {
    if pkt.id == SetFollowCamProperties::ID {
        let m: SetFollowCamProperties = pkt.decode()?;
        Ok(Some(NetEvent::SetFollowCam {
            handle,
            object: m.object_data.object_id,
            properties: m.camera_property.into_iter().map(|p| (p.type_, p.value)).collect(),
        }))
    } else if pkt.id == ClearFollowCamProperties::ID {
        let m: ClearFollowCamProperties = pkt.decode()?;
        Ok(Some(NetEvent::ClearFollowCam {
            object: m.object_data.object_id,
        }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod follow_cam_tests {
    use super::*;

    fn packet(message: &impl Msg) -> IncomingPacket {
        let mut bytes = Vec::new();
        aurora_msg::build_packet(message, 1, true, false, &[], &mut bytes);
        parse_packet(&bytes).expect("packet")
    }

    #[test]
    fn simulator_camera_packets_decode_with_source_region_and_order() {
        let object = uuid::Uuid::from_u128(123);
        let mut message = SetFollowCamProperties::default();
        message.object_data.object_id = object;
        message.camera_property = vec![
            msgs::set_follow_cam_properties::CameraProperty { type_: 12, value: 1.0 },
            msgs::set_follow_cam_properties::CameraProperty { type_: 7, value: 6.0 },
            msgs::set_follow_cam_properties::CameraProperty { type_: 14, value: 128.0 },
        ];
        let set = packet(&message);
        assert!(matches!(
            follow_cam_event(&set, 20),
            Ok(Some(NetEvent::SetFollowCam { handle: 20, .. }))
        ));
        match follow_cam_event(&set, 10).expect("decode").expect("event") {
            NetEvent::SetFollowCam {
                handle,
                object: source,
                properties,
            } => {
                assert_eq!((handle, source), (10, object));
                assert_eq!(properties, vec![(12, 1.0), (7, 6.0), (14, 128.0)]);
            }
            event => panic!("unexpected {event:?}"),
        }
        let mut clear = ClearFollowCamProperties::default();
        clear.object_data.object_id = object;
        let clear = packet(&clear);
        assert!(matches!(follow_cam_event(&clear, 20), Ok(Some(NetEvent::ClearFollowCam { object: source })) if source == object));
    }

    #[test]
    fn truncated_camera_packets_fail_without_emitting_an_event() {
        for id in [SetFollowCamProperties::ID, ClearFollowCamProperties::ID] {
            let packet = IncomingPacket {
                flags: 0,
                sequence: 1,
                id,
                body: vec![0; 8],
                acks: vec![],
            };
            assert!(follow_cam_event(&packet, 10).is_err());
        }
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

impl Session<'_> {
    fn inventory_callback(&mut self, created: bool) -> u32 {
        loop {
            self.next_inventory_callback = self.next_inventory_callback.wrapping_add(1).max(1);
            if let std::collections::hash_map::Entry::Vacant(entry) = self.inventory_callbacks.entry(self.next_inventory_callback) {
                entry.insert((Instant::now(), created));
                return self.next_inventory_callback;
            }
        }
    }
}

#[cfg(test)]
mod socket_tests {
    use super::*;

    #[tokio::test]
    async fn the_socket_gets_the_buffers_asked_for() {
        let sock = UdpSocket::bind("127.0.0.1:0").await.expect("socket");
        let system = socket2::SockRef::from(&sock).recv_buffer_size().expect("size");
        let (receive, send) = size_socket_buffers(&sock);
        // A system may cap what it grants (Linux: net.core.rmem_max) or
        // round it up, but never gives less than before.
        assert!(receive >= system.min(RECEIVE_BUFFER_SIZE), "{receive} < {system}");
        #[cfg(windows)]
        assert_eq!((receive, send), (RECEIVE_BUFFER_SIZE, SEND_BUFFER_SIZE));
        #[cfg(not(windows))]
        assert!(send > 0);
    }
}

#[cfg(test)]
mod event_queue_tests {
    use super::*;

    async fn finished_task() -> JoinHandle<()> {
        let task = tokio::spawn(async {});
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
        task
    }

    #[tokio::test]
    async fn a_lost_event_queue_ends_the_session_only_for_the_agents_region() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main_addr: SocketAddr = "127.0.0.1:13001".parse().expect("address");
        let neighbour_addr: SocketAddr = "127.0.0.1:13002".parse().expect("address");
        let (events, received) = crossbeam_channel::unbounded();
        let sh = Shared {
            http: reqwest::Client::new(),
            caps_http: reqwest::Client::new(),
            events: crate::EventSender(events),
            controls: Arc::new(Mutex::new(AgentControls::default())),
            stats: Arc::new(NetStats::default()),
            object_cache_dir: Arc::new(Mutex::new(None)),
            agent_language: Arc::new(Mutex::new(("en".into(), false))),
            bandwidth_kbps: Default::default(),
        };
        let login = Arc::new(LoginResponse {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            secure_session_id: Uuid::nil(),
            first_name: String::new(),
            last_name: String::new(),
            sim_ip: Ipv4Addr::LOCALHOST,
            sim_port: main_addr.port(),
            circuit_code: 0,
            region_x: 0,
            region_y: 0,
            region_size_x: 256,
            region_size_y: 256,
            seed_capability: String::new(),
            look_at: Vec3::ZERO,
            message: String::new(),
            agent_appearance_service: String::new(),
            inventory_root: Uuid::nil(),
            mfa_hash: None,
            raw: Llsd::Undef,
        });
        let (eq_tx, _) = mpsc::unbounded_channel();
        let (caps_tx, _) = mpsc::unbounded_channel();
        let (notecard_copy_tx, _) = mpsc::unbounded_channel();
        let (inventory_move_tx, _) = mpsc::unbounded_channel();
        let mut session = Session {
            sh: &sh,
            sock,
            login,
            sims: [(main_addr, Sim::new(main_addr, 10)), (neighbour_addr, Sim::new(neighbour_addr, 20))].into(),
            main: Some(main_addr),
            eq_tx,
            caps_tx,
            notecard_copy_tx,
            inventory_move_tx,
            notecard_copies: HashMap::new(),
            agent_update: AgentUpdateRule::new(Instant::now()),
            control_flags: 0,
            one_shot: 0,
            cam_warned: false,
            far: 128.0,
            logout_started: None,
            finished: false,
            agent_moved: false,
            cam_requested_at: None,
            agent_parcel: None,
            pending_named_tp: None,
            object_cache: object_cache::ObjectCaches::new(None).0,
            throttle: crate::throttle::Throttle::new(crate::throttle::DEFAULT_KBPS, Instant::now()),
            social: Default::default(),
            tasks: Default::default(),
            land: Default::default(),
            decode_failures: HashMap::new(),
            inventory_uploads: Default::default(),
            asset_imports: Default::default(),
            inventory_previews: Default::default(),
            script_transfers: Default::default(),
            inventory_callbacks: HashMap::new(),
            inventory_copies: inventory_copies::Pending::default(),
            inventory_merchant_pending: false,
            next_inventory_callback: 0,
        };
        let set_queue = |session: &mut Session, addr: SocketAddr, task: JoinHandle<()>| {
            let sim = session.sims.get_mut(&addr).expect("simulator");
            if let Some(old) = sim.eq_task.replace(task) {
                old.abort();
            }
        };

        // a neighbour that lost its queue only loses its own events
        set_queue(&mut session, neighbour_addr, finished_task().await);
        session.on_eq_lost(neighbour_addr);
        assert!(!session.finished);
        assert!(received.is_empty());

        // the agent's region asked for a new queue since (new seed
        // capability): the old one giving up is not a loss
        set_queue(&mut session, main_addr, tokio::spawn(std::future::pending()));
        session.on_eq_lost(main_addr);
        assert!(!session.finished);
        assert!(received.is_empty());

        // the seed capability: the answer to a seed the region no longer
        // has is ignored; a seed given up is asked again when the region is
        // entered again, and only the agent's region tells the user
        for addr in [main_addr, neighbour_addr] {
            let sim = session.sims.get_mut(&addr).expect("simulator");
            sim.seed = Some("https://sim.example/new".into());
            sim.caps_requested = true;
        }
        let caps = HashMap::from([("ViewerAsset".to_owned(), "https://sim.example/asset".to_owned())]);
        session.on_caps(main_addr, "https://sim.example/old", Ok(caps));
        assert!(session.sims[&main_addr].caps.is_empty());
        assert!(received.is_empty());
        session.on_caps(neighbour_addr, "https://sim.example/new", Err("status 503".into()));
        assert!(!session.sims[&neighbour_addr].caps_requested);
        assert!(received.is_empty());
        session.on_caps(main_addr, "https://sim.example/new", Err("status 503".into()));
        assert!(!session.sims[&main_addr].caps_requested);
        let (_, event) = received.try_recv().expect("alert");
        assert!(matches!(event, NetEvent::Alert { .. }), "{event:?}");

        // its queue really stopped: the session ends, as in Firestorm
        set_queue(&mut session, main_addr, finished_task().await);
        session.on_eq_lost(main_addr);
        assert!(session.finished);
        let (_, event) = received.try_recv().expect("disconnection");
        assert!(matches!(event, NetEvent::Disconnected { .. }), "{event:?}");
    }
}
