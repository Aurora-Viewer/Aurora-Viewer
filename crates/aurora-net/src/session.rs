//! The session task: owns the UDP socket and all simulator circuits,
//! drives login, capabilities, the event queue and agent updates.
//!
//! Message flow follows `newview/llstartup.cpp` and `llviewermessage.cpp`.

mod abuse_reports;
mod account;
mod asset_imports;
mod attach_queue;
mod build_cmds;
mod currency;
mod estate;
mod experience_events;
mod gestures;
mod inventory_copies;
mod inventory_moves;
mod inventory_preview;
mod inventory_updates;
mod inventory_upload;
mod object_actions;
mod object_cache;
mod scripts;
mod sim_requests;
mod slm_net;
mod task_notecard;
mod terrain_raw;
mod wearable_save;

use crate::agent_update::AgentUpdateRule;
use crate::caps::{self, EqEvent, EqMessage};
use crate::circuit::{Circuit, Resend};
use crate::login::{self, LoginError, LoginRequest, LoginResponse};
use crate::objects;
use crate::stats::NetStats;
use crate::terrain;
use crate::throttle::{self, Throttle};
use crate::transition;
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

/// The members, roles, titles and notices of a group (groups.rs).
#[path = "session_groups.rs"]
mod groups_net;
use crate::build::BUILD_PARCEL_SEQ;

/// A simulator silent this long is dead: the Legacy Viewer's circuit timeout
/// (llstartup.cpp, circuit_timeout = 100 s, LLCircuitData::checkPeriodTime).
/// 60 s cut sessions a laggy region or a short network outage would survive.
const SIM_TIMEOUT: Duration = Duration::from_secs(100);
/// DEFAULT_FIELD_OF_VIEW (60°): the camera's angle until the viewer gives its own.
const DEFAULT_FOV: f32 = std::f32::consts::FRAC_PI_3;
const PING_INTERVAL: Duration = Duration::from_secs(5);
/// Diagnostic: the RegionHandshake of a region the agent arrived in should
/// come along with AgentMovementComplete; without it the simulator never
/// sends terrain or objects (it waits for RegionHandshakeReply).
const HANDSHAKE_WARN_DELAY: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
struct ParcelVoice {
    local_id: i32,
    allowed: bool,
    channel_region: Option<Uuid>,
}

impl ParcelVoice {
    /// LLWebRTCVoiceClient::setSpatialChannel (indra/newview/llvoicewebrtc.cpp).
    fn apply(self, region_id: Uuid, parcel: &mut ParcelInfo) {
        if parcel.local_id != self.local_id {
            return;
        }
        parcel.flags &= !parcel_flags::ALLOW_VOICE_CHAT;
        if self.allowed {
            parcel.flags |= parcel_flags::ALLOW_VOICE_CHAT;
        }
        if !region_id.is_nil() {
            parcel.flags &= !parcel_flags::USE_ESTATE_VOICE_CHAN;
            if self.channel_region == Some(region_id) {
                parcel.flags |= parcel_flags::USE_ESTATE_VOICE_CHAN;
            }
        }
    }
}

struct Sim {
    circuit: Circuit,
    handle: RegionHandle,
    region_id: Uuid,
    /// Voice permission arriving before the initial agent-parcel properties.
    pending_voice: Option<ParcelVoice>,
    name: String,
    size: (u32, u32),
    seed: Option<String>,
    caps_requested: bool,
    caps: Arc<HashMap<String, String>>,
    eq_task: Option<JoinHandle<()>>,
    /// The request of the seed capability, while it is asked for.
    caps_task: Option<JoinHandle<()>>,
    /// The same seed asked again (the region sent it twice), while asked.
    caps_again_task: Option<JoinHandle<()>>,
    /// Region features belong to this circuit, never to the next account.
    features_task: Option<JoinHandle<()>>,
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
    /// The UseCircuitCode was acknowledged or given up (`region link`).
    link_up: bool,
    /// When the seed capability was last asked for (timelines).
    caps_asked_at: Option<Instant>,
}

impl Sim {
    fn new(addr: SocketAddr, handle: RegionHandle) -> Sim {
        Sim {
            circuit: Circuit::new(addr),
            handle,
            region_id: Uuid::nil(),
            pending_voice: None,
            name: String::new(),
            size: (256, 256),
            seed: None,
            caps_requested: false,
            caps: Arc::new(HashMap::new()),
            eq_task: None,
            caps_task: None,
            caps_again_task: None,
            features_task: None,
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
            link_up: false,
            caps_asked_at: None,
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
        if let Some(t) = self.caps_again_task.take() {
            t.abort();
        }
        if let Some(t) = self.features_task.take() {
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
    /// The viewer's part of the viewer statistics (`NetClient::set_viewer_stats`).
    pub viewer_stats: Arc<Mutex<crate::viewer_stats::ViewerSide>>,
    /// Reports being posted (`NetClient::wait_viewer_stats`).
    pub viewer_stats_posts: Arc<crate::viewer_stats::PostsInFlight>,
}

struct Session<'a> {
    sh: &'a Shared,
    sock: Arc<UdpSocket>,
    login: Arc<LoginResponse>,
    sims: HashMap<SocketAddr, Sim>,
    main: Option<SocketAddr>,
    eq_tx: mpsc::UnboundedSender<EqMessage>,
    caps_tx: CapsSender,
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
    /// FSDisableNeighbourRegionConnections, while the agent stands inside its
    /// region: EnableSimulator for another region is ignored.
    refuse_neighbours: bool,
    /// The camera's vertical viewing angle, last told to the simulator.
    fov: f32,
    logout_started: Option<Instant>,
    finished: bool,
    agent_moved: bool,
    /// When Linden Lab's viewer statistics leave (viewer_stats.rs), and the
    /// regions the agent arrived in (LLAgent::mRegionsVisited).
    viewer_stats: crate::viewer_stats::Schedule,
    regions_visited: std::collections::HashSet<u64>,
    cam_requested_at: Option<Instant>,
    /// Last confirmed agent parcel in the main region, including its geometry.
    agent_parcel: Option<Arc<ParcelInfo>>,
    /// Teleport by region name waiting for its MapBlockReply (lowercase name, position).
    pending_named_tp: Option<(String, Uuid)>,
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
    /// Invoice of the last EstateOwnerMessage `getinfo` (session/estate.rs).
    estate_invoice: Uuid,
    /// The RAW terrain file going by Xfer (session/terrain_raw.rs).
    terrain_raw: terrain_raw::RawXfers,
    /// Decode failures per message name, to rate-limit their warnings.
    decode_failures: HashMap<&'static str, u64>,
    inventory_uploads: inventory_upload::Uploads,
    asset_imports: asset_imports::Imports,
    inventory_previews: inventory_preview::Previews,
    script_transfers: scripts::ScriptTransfers,
    inventory_callbacks: HashMap<u32, (Instant, bool)>,
    inventory_copies: inventory_copies::Pending,
    attach_queue: attach_queue::AttachQueue,
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

/// Items read, items the server says do not exist, and failed reads.
fn send_item_reads(events: &crate::EventSender, reads: crate::inventory::ais::ItemReads, attachment: bool) {
    if !reads.items.is_empty() {
        let _ = events.send(if attachment {
            NetEvent::AttachmentInventoryItems(reads.items)
        } else {
            NetEvent::InventoryItems(reads.items)
        });
    }
    if !reads.missing.is_empty() {
        let _ = events.send(NetEvent::InventoryItemsMissing { items: reads.missing });
    }
    if !reads.failed.is_empty() {
        log::warn!("{} inventory item reads failed", reads.failed.len());
        let _ = events.send(NetEvent::InventoryItemsFailed { items: reads.failed });
    }
}

/// (simulator, seed asked, its capabilities or why they did not come)
type CapsSender = mpsc::UnboundedSender<(SocketAddr, String, Result<HashMap<String, String>, String>)>;

/// One request of a seed capability, answered on the session loop (on_caps).
async fn resolve_seed_task(http: reqwest::Client, tx: CapsSender, addr: SocketAddr, seed: String) {
    let alive = tx.clone();
    let r = caps::resolve_seed(&http, &seed, caps::SeedRetry::FIRESTORM, move || !alive.is_closed())
        .await
        .map_err(|e| e.to_string());
    let _ = tx.send((addr, seed, r));
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
            let previous_session = e.previous_session();
            // The grid's own words (LLLoginInstance shows the message as it
            // comes), without the "login refused (presence)" prefix.
            let error = match &e {
                LoginError::Refused { message, .. } if previous_session && !message.trim().is_empty() => message.trim().to_owned(),
                _ => e.to_string(),
            };
            emit(
                sh,
                NetEvent::LoginFailed {
                    error,
                    mfa_required: mfa,
                    tos_required: tos,
                    bad_credentials,
                    previous_session,
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
                    previous_session: false,
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
        refuse_neighbours: false,
        fov: DEFAULT_FOV,
        logout_started: None,
        finished: false,
        agent_moved: false,
        viewer_stats: Default::default(),
        regions_visited: Default::default(),
        cam_requested_at: None,
        agent_parcel: None,
        pending_named_tp: None,
        object_cache,
        throttle: Throttle::new(sh.bandwidth_kbps.load(Ordering::Relaxed) as f32, Instant::now()).counting_from(packets_counted(&sh.stats)),
        social: Default::default(),
        tasks: Default::default(),
        land: Default::default(),
        estate_invoice: Uuid::nil(),
        terrain_raw: Default::default(),
        decode_failures: HashMap::new(),
        inventory_uploads: Default::default(),
        asset_imports: Default::default(),
        inventory_previews: Default::default(),
        script_transfers: Default::default(),
        inventory_callbacks: HashMap::new(),
        inventory_copies: inventory_copies::Pending::default(),
        attach_queue: Default::default(),
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
    s.set_main(addr, login.seed_capability.clone());
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

    /// AgentFOV with the camera's real vertical angle
    /// (LLViewerCamera::setView: GenCounter 0, reliable).
    fn send_fov(&mut self, to: SocketAddr) {
        let fov = agent_fov(self.agent_id(), self.session_id(), self.login.circuit_code, self.fov);
        self.send(to, &fov, true);
    }

    fn send_main<M: Msg>(&mut self, msg: &M, reliable: bool) {
        if let Some(a) = self.main {
            self.send(a, msg, reliable);
        }
    }

    /// LLAppViewer::sendLogoutRequest: the region ends this session.
    fn send_logout_request(&mut self) {
        let mut m = LogoutRequest::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        self.send_main(&m, true);
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
        transition::link(|| format!("UseCircuitCode sent to region {} ({addr})", transition::grid(handle)));
        self.sh.stats.sim_count.store(self.sims.len() as u32, Ordering::Relaxed);
    }

    fn set_main(&mut self, addr: SocketAddr, seed: String) {
        let changed = self.main != Some(addr);
        self.main = Some(addr);
        self.set_seed(addr, seed);
        if changed {
            self.agent_parcel = None;
            if let Some(sim) = self.sims.get_mut(&addr) {
                sim.pending_voice = None;
            }
            // capabilities of a neighbour we already know: no new caps event
            if self.sims.get(&addr).is_some_and(|s| !s.caps.is_empty()) {
                self.update_agent_language();
                self.request_environment(None);
            }
        }
        let handle = self.sims.get(&addr).map_or(0, |sim| sim.handle);
        self.publish_motion();
        emit(self.sh, NetEvent::MainRegionChanged { handle });
    }

    /// LLViewerRegion::setSeedCapability. A new seed drops the capabilities
    /// until its own arrive: a request sent meanwhile on the old ones could
    /// fail, inventory and outfit changes first. The same seed again is
    /// asked a second time (requestBaseCapabilitiesCompleteCoro): the first
    /// answer of a neighbour lacks the inventory capabilities, and the
    /// crossing or EstablishAgentCommunication that makes it the agent's
    /// region repeats its seed (#750).
    fn set_seed(&mut self, addr: SocketAddr, seed: String) {
        let Some(sim) = self.sims.get_mut(&addr) else {
            return;
        };
        if sim.seed.as_deref() == Some(seed.as_str()) {
            if sim.caps_requested {
                self.request_caps_again(addr);
            } else {
                // given up before: asked again from the start
                self.request_caps(addr);
            }
            return;
        }
        sim.seed = Some(seed);
        sim.caps_requested = false;
        sim.caps = Arc::default();
        if let Some(old) = sim.caps_again_task.take() {
            old.abort();
        }
        self.request_caps(addr);
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
        transition::step(|| format!("CompleteAgentMovement sent to region {}", transition::grid(self.handle_of(addr))));
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
        self.sh
            .stats
            .throttle_asked_kbps
            .store(self.throttle.asked() as u32, Ordering::Relaxed);
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
        sim.caps_asked_at = Some(Instant::now());
        transition::step(|| format!("seed capability asked of region {}", transition::grid(sim.handle)));
        let http = self.sh.caps_http.clone();
        let tx = self.caps_tx.clone();
        // (one request per simulator: a new seed takes the place of the old)
        if let Some(old) = sim.caps_task.take() {
            old.abort();
        }
        if let Some(old) = sim.features_task.take() {
            old.abort();
        }
        sim.caps_task = Some(tokio::spawn(resolve_seed_task(http, tx, addr, seed)));
    }

    /// The seed the region already had, asked again beside the first
    /// request (which keeps going): on_caps keeps the larger answer.
    fn request_caps_again(&mut self, addr: SocketAddr) {
        let Some(sim) = self.sims.get_mut(&addr) else {
            return;
        };
        let Some(seed) = sim.seed.clone().filter(|s| !s.is_empty()) else {
            return;
        };
        transition::step(|| format!("seed capability asked again of region {} (same seed)", transition::grid(sim.handle)));
        let http = self.sh.caps_http.clone();
        let tx = self.caps_tx.clone();
        if let Some(old) = sim.caps_again_task.take() {
            old.abort();
        }
        sim.caps_again_task = Some(tokio::spawn(resolve_seed_task(http, tx, addr, seed)));
    }

    fn on_caps(&mut self, addr: SocketAddr, seed: &str, r: Result<HashMap<String, String>, String>) {
        // the region has another seed since: the answer of a stale request
        // (mHttpResponderID in LLViewerRegionImpl::requestBaseCapabilitiesCoro)
        if self.sims.get(&addr).is_some_and(|sim| sim.seed.as_deref() != Some(seed)) {
            log::info!("capabilities for {addr}: answer to a previous seed ignored");
            return;
        }
        // a second answer to the seed: it replaces the capabilities only
        // when it grants more (requestBaseCapabilitiesCompleteCoro); the
        // event queue and the region features keep going
        if let Some(sim) = self.sims.get_mut(&addr)
            && !sim.caps.is_empty()
        {
            match r {
                Ok(caps) if caps.len() > sim.caps.len() => {
                    log::info!(
                        "{} capabilities for {} ({}) asked again, {} before",
                        caps.len(),
                        addr,
                        sim.name,
                        sim.caps.len()
                    );
                    transition::step(|| {
                        format!(
                            "seed capability of region {} asked again: {} capabilities, {} before",
                            transition::grid(sim.handle),
                            caps.len(),
                            sim.caps.len()
                        )
                    });
                    // ModifyRegion granted only now: the handshake could not ask it
                    let modify_new = !sim.caps.contains_key("ModifyRegion") && caps.contains_key("ModifyRegion");
                    let caps = Arc::new(caps);
                    sim.caps = caps.clone();
                    emit(self.sh, NetEvent::Capabilities { handle: sim.handle, caps });
                    let terrain_handle = (sim.handshake_seen && modify_new).then_some(sim.handle);
                    if let Some(handle) = terrain_handle {
                        self.request_terrain_transforms(handle);
                    }
                }
                Ok(caps) => log::info!("{} capabilities for {addr} asked again, {} kept", caps.len(), sim.caps.len()),
                Err(e) => log::warn!("capabilities for {addr} asked again failed, {} kept: {e}", sim.caps.len()),
            }
            return;
        }
        let caps = match r {
            Ok(c) => c,
            Err(e) => {
                log::warn!("capabilities for {addr} failed for good: {e}");
                transition::step(|| format!("seed capability of region {} failed: {e}", transition::grid(self.handle_of(addr))));
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
        transition::step(|| {
            format!(
                "seed capability of region {}: {} capabilities in {:.0} ms",
                transition::grid(sim.handle),
                caps.len(),
                sim.caps_asked_at.map_or(0.0, |t| t.elapsed().as_secs_f32() * 1000.0)
            )
        });
        if let Some(eq) = caps.get("EventQueueGet").cloned() {
            transition::link(|| format!("event queue of region {} started", transition::grid(sim.handle)));
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
        let terrain_handle = sim.handshake_seen.then_some(sim.handle);
        if let Some(handle) = terrain_handle {
            self.request_terrain_transforms(handle);
        }
        if let Some((handle, url)) = features {
            self.request_features(addr, handle, url);
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
    /// server type, hover support and RenderMaterials limits.
    fn request_features(&mut self, addr: SocketAddr, handle: RegionHandle, url: String) {
        let Some(sim) = self.sims.get_mut(&addr) else { return };
        if let Some(old) = sim.features_task.take() {
            old.abort();
        }
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        sim.features_task = Some(tokio::spawn(async move {
            match caps::simulator_features(&http, &url, caps::SeedRetry::SIMULATOR_FEATURES).await {
                Ok(v) => {
                    let voice_server_type = v["VoiceServerType"].to_string_value();
                    let rate = &v["RenderMaterialsCapability"];
                    let materials_rate = (!rate.is_undef()).then(|| rate.as_f32());
                    let max = &v["MaxMaterialsPerTransaction"];
                    let materials_max = (!max.is_undef()).then(|| max.as_u32());
                    let avatar_hover_height = v["AvatarHoverHeightEnabled"].as_bool();
                    let lsl_syntax_id = v["LSLSyntaxId"].as_uuid();
                    // an OpenSim grid's status feed and page
                    // (LFSimFeatureHandler::setSupportedFeatures)
                    let extras = &v["OpenSimExtras"];
                    let grid_status_feed = extras["GridStatusRSS"].to_string_value();
                    let grid_status_page = extras["GridStatus"].to_string_value();
                    log::info!("SimulatorFeatures received: avatar hover height {avatar_hover_height}");
                    let _ = events.send(NetEvent::SimulatorFeatures {
                        handle,
                        voice_server_type,
                        avatar_hover_height,
                        materials_rate,
                        materials_max,
                        lsl_syntax_id,
                        grid_status_feed,
                        grid_status_page,
                    });
                }
                Err(_) => log::warn!("SimulatorFeatures unavailable after retries"),
            }
        }));
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
        if b["ParcelData"][0]["RequestResult"].as_i32() == crate::land::PARCEL_RESULT_NO_DATA {
            return;
        }
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
        if matches!(
            info.sequence_id,
            crate::land::SELECTED_PARCEL_SEQ_ID | crate::land::HOVERED_PARCEL_SEQ_ID
        ) || info.sequence_id >= BUILD_PARCEL_SEQ
        {
            self.refresh_agent_parcel(sim, &info);
        }
        if info.sequence_id == crate::land::SELECTED_PARCEL_SEQ_ID {
            // the About Land selection, in whichever region it lies
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
            return;
        }
        if info.sequence_id == crate::land::HOVERED_PARCEL_SEQ_ID {
            // the hover tips' parcel (LLViewerParcelMgr::processParcelProperties
            // keeps it apart from the agent's and the selection's)
            let handle = self.handle_of(sim);
            emit(
                self.sh,
                NetEvent::HoverParcel {
                    handle,
                    parcel: Arc::new(info),
                },
            );
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
            if info.sequence_id >= 0 {
                log::info!(
                    "agent parcel notification from region {:x} ignored: not the agent's region (local id {})",
                    self.handle_of(sim),
                    info.local_id
                );
            }
            return;
        }
        // LLViewerParcelMgr::processParcelProperties accepts sequence zero
        // unconditionally, otherwise only a newer agent-parcel notification.
        if info.sequence_id != 0 && self.agent_parcel.as_ref().is_some_and(|p| info.sequence_id <= p.sequence_id) {
            log::info!(
                "agent parcel notification ignored: sequence {} after {} (local id {})",
                info.sequence_id,
                self.agent_parcel.as_ref().map_or(0, |p| p.sequence_id),
                info.local_id
            );
            return;
        }
        self.publish_agent_parcel(Arc::new(info));
    }

    /// LLViewerParcelMgr::processParcelProperties also unpacks a selected or
    /// hovered parcel into the agent parcel when both region and local id match.
    /// Keep the agent's bitmap, bounds and notification sequence: a selection
    /// can describe a rectangle instead of the parcel occupied by the agent.
    fn refresh_agent_parcel(&mut self, sim: SocketAddr, info: &ParcelInfo) {
        if self.main != Some(sim) {
            return;
        }
        let Some(current) = self.agent_parcel.as_ref().filter(|p| p.local_id == info.local_id) else {
            return;
        };
        let mut updated = info.clone();
        updated.sequence_id = current.sequence_id;
        updated.request_result = current.request_result;
        updated.bitmap.clone_from(&current.bitmap);
        updated.aabb_min = current.aabb_min;
        updated.aabb_max = current.aabb_max;
        // Keep the shared snapshot on identical refreshes: its identity is
        // used by the viewer's parcel visibility cache.
        if &updated == current.as_ref() {
            return;
        }
        self.publish_agent_parcel(Arc::new(updated));
    }

    fn publish_agent_parcel(&mut self, mut info: Arc<ParcelInfo>) {
        if let Some(sim) = self.main.and_then(|addr| self.sims.get_mut(&addr))
            && let Some(voice) = sim.pending_voice.take()
        {
            voice.apply(sim.region_id, Arc::make_mut(&mut info));
        }
        let (local_id, version) = (info.local_id, info.env_version);
        let environment_changed = self.agent_parcel.as_ref().map(|p| (p.local_id, p.env_version)) != Some((local_id, version));
        self.agent_parcel = Some(info.clone());
        emit(self.sh, NetEvent::AgentParcel(info));
        if environment_changed {
            self.request_environment(Some(local_id));
        }
    }

    /// LLViewerParcelVoiceInfo::post (indra/newview/llvoiceclient.cpp): the
    /// simulator also pushes voice changes without a ParcelProperties reply.
    /// Keep only the spatial permission/channel kind, never call credentials.
    fn on_parcel_voice_info(&mut self, sim: SocketAddr, body: &Llsd) {
        if self.main != Some(sim) {
            return;
        }
        let Llsd::Integer(local_id) = body["parcel_local_id"] else {
            return;
        };
        let (allowed, channel_region) = match &body["voice_credentials"]["channel_uri"] {
            Llsd::String(uri) | Llsd::Uri(uri) => (!uri.is_empty(), Uuid::parse_str(uri).ok()),
            Llsd::Uuid(id) => (true, Some(*id)),
            _ => return,
        };
        let voice = ParcelVoice {
            local_id,
            allowed,
            channel_region,
        };
        let Some(current) = self.agent_parcel.as_ref() else {
            if let Some(sim) = self.sims.get_mut(&sim) {
                sim.pending_voice = Some(voice);
            }
            return;
        };
        if current.local_id != local_id {
            return;
        }
        let mut updated = (**current).clone();
        let region_id = self.sims.get(&sim).map_or(Uuid::nil(), |sim| sim.region_id);
        voice.apply(region_id, &mut updated);
        if updated.flags != current.flags {
            self.agent_parcel = Some(Arc::new(updated));
            emit(
                self.sh,
                NetEvent::ParcelVoiceInfo {
                    handle: self.handle_of(sim),
                    local_id,
                    allowed,
                    use_estate: (!region_id.is_nil()).then_some(channel_region == Some(region_id)),
                },
            );
        }
    }

    fn remove_sim(&mut self, addr: SocketAddr) {
        self.object_cache.forget(addr);
        if let Some(sim) = self.sims.remove(&addr) {
            transition::link(|| {
                format!(
                    "circuit to region {} ({addr}) closed after {:.0} s",
                    transition::grid(sim.handle),
                    sim.ucc_sent_at.elapsed().as_secs_f32()
                )
            });
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
        if self.logout_started.is_none() && self.viewer_stats.due(Instant::now(), self.agent_moved) {
            self.send_viewer_stats();
        }
        self.expire_task_inventory();
        self.expire_script_transfers();
        self.expire_inventory_uploads();
        self.expire_terrain_raw();
        self.expire_asset_imports();
        self.expire_inventory_previews();
        for completion in self.inventory_copies.expire(Instant::now()) {
            self.finish_inventory_copies(completion);
        }
        for request in self.inventory_copies.expire_singles(Instant::now()) {
            emit(
                self.sh,
                NetEvent::InventoryItemCopied {
                    request,
                    result: Err(aurora_i18n::tr!("inventory-copy-unconfirmed").into()),
                },
            );
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
        self.send_queued_attachment(now);
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
            if !s.link_up
                && let Some(q) = s.ucc_seq
            {
                let given_up = dropped.contains(&q);
                if given_up || !s.circuit.is_unacked(q) {
                    s.link_up = true;
                    let ms = s.ucc_sent_at.elapsed().as_secs_f32() * 1000.0;
                    transition::link(|| {
                        let region = transition::grid(s.handle);
                        if given_up {
                            format!("circuit to region {region} ({addr}) not acknowledged: UseCircuitCode given up after {ms:.0} ms")
                        } else {
                            format!("circuit to region {region} ({addr}) up: UseCircuitCode acknowledged after {ms:.0} ms")
                        }
                    });
                }
            }
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
                    lost: true,
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
        transition::tick(&self.sh.stats);
        if let Some(t) = self.cam_requested_at {
            // diagnostic: no AgentUpdate is sent until AgentMovementComplete
            if !self.agent_moved && !self.cam_warned && t.elapsed() > Duration::from_secs(5) {
                self.cam_warned = true;
                log::warn!("no AgentMovementComplete 5 s after CompleteAgentMovement: agent updates (movement) paused");
            }
            if !self.agent_moved && t.elapsed() > Duration::from_secs(90) {
                // STATE_AGENT_WAIT's timeout (llstartup.cpp): "Session was
                // created, don't just hang up on server, send a logout
                // request", or the grid keeps this session and the next
                // login finds it still there.
                self.send_logout_request();
                emit(
                    self.sh,
                    NetEvent::Disconnected {
                        reason: aurora_i18n::tr!("session-the-region-did-not-respond-in-time-agentmovementcomplete").into(),
                        lost: true,
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
                        lost: true,
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
        c.control_flags = crate::script_controls::mask_left_button(c.control_flags | self.one_shot, c.alt_or_ctrl);
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
        if self.on_object_action_command(&c)
            || self.on_social_command(&c)
            || self.on_offline_command(&c)
            || self.on_search_command(&c)
            || self.on_groups_command(&c)
        {
            return;
        }
        match c {
            NetCommand::UpdateAgentLanguage => self.update_agent_language(),
            NetCommand::SetHoverHeight { region, request, value } => self.set_hover_height(region, request, value),
            NetCommand::SetMaturityPreference(maturity) => self.set_maturity_preference(maturity),
            NetCommand::SetHomeLocation { position, look_at } => self.set_home_location(position, look_at),
            NetCommand::RequestUserInfo => self.request_user_info(),
            NetCommand::UpdateUserInfo {
                directory_visibility,
                im_via_email,
            } => self.update_user_info(directory_visibility, im_via_email),
            NetCommand::ServerReleaseNotes => self.fetch_server_release_notes(),
            NetCommand::FetchPathfinding { request, region, kind } => self.fetch_pathfinding(request, region, kind),
            NetCommand::SimConsole(command) => self.send_sim_console(command),
            NetCommand::ModerateVoice { target, mute } => self.moderate_voice(target, mute),
            NetCommand::FetchLslSyntax { id, cache_dir } => self.fetch_lsl_syntax(id, cache_dir),
            NetCommand::Login(_) => log::warn!("already logged in"),
            NetCommand::Logout => {
                if self.logout_started.is_none() {
                    // LLAppViewer::requestQuit: a last report, then the logout
                    if self.agent_moved {
                        self.send_viewer_stats();
                    }
                    self.send_logout_request();
                    self.logout_started = Some(Instant::now());
                }
            }
            NetCommand::Chat {
                message,
                channel,
                chat_type,
            } => {
                // SL limits chat to 1023 bytes; split longer messages. The
                // typing notices are empty lines, sent whole
                // (send_chat_from_viewer: "Don't break null messages").
                let mut rest = message.as_str();
                let mut notice = matches!(chat_type, ChatType::StartTyping | ChatType::StopTyping);
                while !rest.is_empty() || std::mem::take(&mut notice) {
                    let mut cut = rest.len().min(1000);
                    while !rest.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    let (part, r) = rest.split_at(cut);
                    rest = r;
                    // really_send_chat_from_viewer (fsnearbychathub.cpp): an
                    // avatar cannot speak on a negative channel, the
                    // simulator drops such a ChatFromViewer. The line goes
                    // as a ScriptDialogReply from the agent itself, its
                    // text as the button label (a 255-byte field), which
                    // llListen hears on that channel like a dialog answer.
                    if channel < 0 {
                        let mut end = part.len().min(254);
                        while !part.is_char_boundary(end) {
                            end -= 1;
                        }
                        let mut m = ScriptDialogReply::default();
                        m.agent_data.agent_id = self.agent_id();
                        m.agent_data.session_id = self.session_id();
                        m.data.object_id = self.agent_id();
                        m.data.chat_channel = channel;
                        m.data.button_index = 0;
                        m.data.button_label = str_field(&part[..end]);
                        self.send_main(&m, true);
                        continue;
                    }
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
            NetCommand::ChatReply { message, channel } => {
                if let Some(packet) = crate::chat_reply::packet(self.agent_id(), self.session_id(), channel, &message) {
                    self.send_main(&packet, true);
                }
            }
            NetCommand::TeleportHome => {
                let mut m = TeleportLandmarkRequest::default();
                m.info.agent_id = self.agent_id();
                m.info.session_id = self.session_id();
                m.info.landmark_id = uuid::Uuid::nil();
                self.send_main(&m, true);
                transition::begin(
                    transition::Kind::Teleport,
                    || "TeleportLandmarkRequest (home) sent".to_string(),
                    &self.sh.stats,
                );
            }
            NetCommand::CancelRegionRestart => {
                // LLPanelRegionInfo::sendEstateOwnerMessage("restart", {"-1"}),
                // to the agent's region, with the invoice of the Region /
                // Estate window's last `getinfo` (LLFloaterRegionInfo::
                // getLastInvoice: nil when the window was never opened)
                let mut m = EstateOwnerMessage::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.agent_data.transaction_id = uuid::Uuid::nil();
                m.method_data.method = str_field("restart");
                m.method_data.invoice = self.estate_invoice;
                m.param_list = vec![msgs::estate_owner_message::ParamList {
                    parameter: str_field("-1"),
                }];
                log::info!("Sending estate request 'restart' (-1: cancel)");
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
                transition::begin(
                    transition::Kind::Teleport,
                    || format!("TeleportLocationRequest sent (region {})", transition::grid(handle)),
                    &self.sh.stats,
                );
            }
            NetCommand::TeleportToRegion { request, name, .. } => {
                // LLWorldMapMessage::sendNamedRegionRequest: the reply gives the grid position
                self.pending_named_tp = Some((name.trim().to_lowercase(), request));
                let mut m = MapNameRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.agent_data.flags = 2; // LAYER_FLAG
                m.name_data.name = str_field(name.trim());
                self.send_main(&m, true);
            }
            NetCommand::SetDrawDistance(d) => self.far = d.clamp(32.0, 1024.0),
            NetCommand::RefuseNeighbourRegions(on) => self.refuse_neighbours = on,
            NetCommand::SetFov(angle) => {
                if angle.is_finite() && angle > 0.0 && angle != self.fov {
                    self.fov = angle;
                    if let Some(main) = self.main {
                        self.send_fov(main);
                    }
                }
            }
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
                transition::begin(
                    transition::Kind::Teleport,
                    || "TeleportLureRequest sent".to_string(),
                    &self.sh.stats,
                );
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
                sender,
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
                // callback_script_dialog (indra/newview/llviewermessage.cpp)
                // answers the exact sender, even when it is a neighbour or
                // the agent has changed regions while the menu stayed open.
                if let Some(to) = sender.or(self.main) {
                    if self.sims.contains_key(&to) {
                        if sender.is_some() {
                            log::info!(
                                "script dialog reply: channel={channel}, button={index}, label_bytes={}, sender_is_main={}",
                                label.len(),
                                self.main == Some(to),
                            );
                        }
                        self.send(to, &m, true);
                    } else {
                        log::warn!("script dialog reply not sent: source circuit is closed (channel={channel}, button={index})");
                    }
                }
            }
            NetCommand::ScriptAnswer {
                task_id,
                item_id,
                questions,
                region,
            } => {
                let mut m = ScriptAnswerYes::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.data.task_id = task_id;
                m.data.item_id = item_id;
                m.data.questions = questions;
                // script_question_cb answers the simulator that asked (the
                // object's, a neighbour's for an object across the border)
                match self.sim_for_handle(region) {
                    Some(addr) => self.send(addr, &m, true),
                    None => self.send_main(&m, true),
                }
            }
            NetCommand::ExperienceInfo(ids) => self.request_experiences(ids),
            NetCommand::GroupExperiences(group) => self.request_group_experiences(group),
            NetCommand::ExperiencePermission { experience, change } => self.experience_permission(experience, change),
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
            NetCommand::RequestReportObjectProperties { handle, object } => self.request_report_object(handle, object),
            NetCommand::RequestAbuseCategories { language } => self.request_abuse_categories(&language),
            NetCommand::SendAbuseReport(report) => self.send_abuse_report(*report),
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
            NetCommand::PayResident {
                to,
                group,
                amount,
                message,
            } => {
                // Port of give_money (llviewermessage.cpp): TRANS_GIFT, the
                // destination-is-a-group bit of pack_transaction_flags, and
                // the payment message as the description.
                if amount > 0
                    && !to.is_nil()
                    && let Some(addr) = self.main
                {
                    let mut m = MoneyTransferRequest::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.money_data.source_id = self.agent_id();
                    m.money_data.dest_id = to;
                    m.money_data.flags = if group { 2 } else { 0 };
                    m.money_data.amount = amount;
                    m.money_data.transaction_type = 5001;
                    m.money_data.description = str_field(&message);
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
            NetCommand::ClassifiedUpdate(c) => {
                // LLAvatarPropertiesProcessor::sendClassifiedInfoUpdate; the
                // simulator fills ParcelID and ParentEstate on the way
                let mut m = ClassifiedInfoUpdate::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                let d = &mut m.data;
                d.classified_id = c.id;
                d.category = c.category;
                d.name = str_field(&c.name);
                d.desc = str_field(&c.desc);
                d.parcel_id = c.parcel;
                d.parent_estate = 0;
                d.snapshot_id = c.snapshot;
                d.pos_global = c.pos_global;
                d.classified_flags = c.flags;
                d.price_for_listing = c.price;
                self.send_main(&m, true);
            }
            NetCommand::ClassifiedStatsRequest(id) => {
                // LLPanelProfileClassified::onOpen: the stats of the new table
                // when the capability exists
                let Some(url) = self.main_cap("SearchStatRequest") else {
                    return;
                };
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                tokio::spawn(async move {
                    let body = aurora_llsd::llsd_map! { "classified_id" => id };
                    match caps::post_llsd(&http, &url, &body).await {
                        Ok(reply) => {
                            let stats = crate::profile::ClassifiedStats::from_llsd(&reply);
                            let _ = events.send(NetEvent::ClassifiedStats { id, stats });
                        }
                        Err(e) => log::warn!("SearchStatRequest {id}: {e}"),
                    }
                });
            }
            NetCommand::ClassifiedClick { kind, classified } => {
                let Some(url) = self.main_cap("SearchStatTracking") else {
                    return;
                };
                let http = self.sh.caps_http.clone();
                let body = crate::profile::click_body(kind, &classified);
                tokio::spawn(async move {
                    if let Err(e) = caps::post_llsd(&http, &url, &body).await {
                        log::info!("SearchStatTracking click report not sent: {e}");
                    }
                });
            }
            NetCommand::Land(l) => self.on_land_command(l),
            NetCommand::Estate(e) => self.on_estate_command(e),
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
            NetCommand::PointAt {
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
                    .push(pointat_effect(self.agent_id(), effect, target, offset, kind, duration));
                self.send_main(&m, false);
            }
            NetCommand::ResetSkeleton {
                effect,
                target,
                animations,
            } => {
                let mut m = ViewerEffect::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.effect.push(reset_skeleton_effect(self.agent_id(), effect, target, animations));
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
            NetCommand::ImReply {
                to,
                message,
                busy,
                offline,
            } => {
                let name = format!("{} {}", self.login.first_name, self.login.last_name);
                if let Some(packet) = crate::im_reply::packet(self.agent_id(), self.session_id(), &name, to, &message, busy, offline) {
                    self.send_main(&packet, true);
                }
            }
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
            NetCommand::TrackAgent(prey) => {
                let mut m = TrackAgent::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.target_data.prey_id = prey;
                self.send_main(&m, true);
            }
            NetCommand::FindAgent(prey) => {
                let mut m = FindAgent::default();
                m.agent_block.hunter = self.agent_id();
                m.agent_block.prey = prey;
                // SpaceIP is filled in by the simulator
                m.location_block.push(find_agent::LocationBlock {
                    global_x: 0.0,
                    global_y: 0.0,
                });
                self.send_main(&m, true);
            }
            NetCommand::TeleportLandmark(asset) => {
                let mut m = TeleportLandmarkRequest::default();
                m.info.agent_id = self.agent_id();
                m.info.session_id = self.session_id();
                m.info.landmark_id = asset;
                self.send_main(&m, true);
                transition::begin(
                    transition::Kind::Teleport,
                    || "TeleportLandmarkRequest sent".to_string(),
                    &self.sh.stats,
                );
            }
            NetCommand::FetchInventory { folders, owner, library } => self.fetch_inventory(folders, owner, library),
            NetCommand::FetchInventoryTree {
                request,
                parent,
                children,
                depth,
                owner,
                library,
            } => self.fetch_inventory_tree(request, parent, children, depth, owner, library),
            NetCommand::FetchItems { items, owner } => self.fetch_items(items, owner, false),
            NetCommand::FetchAttachmentItems { items } => self.fetch_items(items, self.agent_id(), true),
            NetCommand::FetchInventoryOrphans { request } => {
                let ais = self.main_cap("InventoryAPIv3");
                let legacy = self.main_cap("FetchInventoryDescendents2");
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                let owner = self.agent_id();
                tokio::spawn(async move {
                    let result = crate::inventory::orphans::fetch(&http, ais.as_deref(), legacy.as_deref(), owner).await;
                    let _ = events.send(NetEvent::InventoryOrphansFetched {
                        request,
                        owner,
                        result: result.map_err(str::to_owned),
                    });
                });
            }
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
            NetCommand::LandToolPreflight {
                helper,
                billable_area,
                currency_buy,
            } => self.land_tool_preflight(helper, billable_area, currency_buy),
            NetCommand::LandToolBuy { helper, request } => self.land_tool_buy(helper, request),
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
            NetCommand::RequestMerchantStatus => {
                let events = self.sh.events.clone();
                let http = self.sh.caps_http.clone();
                let cap = self.main_cap("DirectDelivery");
                tokio::spawn(async move {
                    // getMerchantStatusCoro: no capability is a connection failure
                    let status = match cap {
                        None => crate::slm::MerchantStatus::ConnectionFailure(String::new()),
                        Some(cap) => match http
                            .get(format!("{}/merchant", cap.trim_end_matches('/')))
                            .timeout(Duration::from_secs(20))
                            .send()
                            .await
                        {
                            Ok(r) => {
                                let reason = r.status().canonical_reason().unwrap_or("").to_owned();
                                crate::slm::merchant_status(r.status().as_u16(), &reason)
                            }
                            Err(e) => crate::slm::MerchantStatus::ConnectionFailure(e.to_string()),
                        },
                    };
                    let _ = events.send(NetEvent::MerchantStatus(status));
                });
            }
            NetCommand::Slm(request) => {
                let events = self.sh.events.clone();
                let http = self.sh.caps_http.clone();
                let cap = self.main_cap("DirectDelivery");
                tokio::spawn(async move {
                    let reply = slm_net::slm_request(&http, cap.as_deref(), &request).await;
                    let _ = events.send(NetEvent::Slm { request, reply });
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
            NetCommand::Bridge { request, command } => {
                let events = self.sh.events.clone();
                tokio::spawn(async move {
                    let result = crate::bridge::send(command).await;
                    let _ = events.send(NetEvent::BridgeReply { request, result });
                });
            }
            NetCommand::BridgeQuery { request, endpoint, query } => {
                let events = self.sh.events.clone();
                tokio::spawn(async move {
                    let result = crate::bridge::query::send(endpoint, query).await;
                    let _ = events.send(NetEvent::BridgeQueryReply { request, result });
                });
            }
            NetCommand::SaveWearable {
                request,
                item,
                data,
                copy_name,
            } => self.save_wearable(request, *item, data, copy_name),
            NetCommand::CreateInventoryWearable {
                parent,
                kind,
                name,
                data,
                permissions,
            } => self.create_wearable(parent, kind, name, data, permissions),
            NetCommand::CreateSettingsItem {
                request,
                parent,
                name,
                subtype,
                next_owner_mask,
                data,
            } => self.create_settings_item(request, parent, name, subtype, next_owner_mask, data),
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
                    56 => "UpdateSettingsAgentInventory",
                    57 => "UpdateMaterialAgentInventory",
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
            NetCommand::CopyInventoryItem { request, copy } => self.copy_inventory_item(request, copy),
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
            NetCommand::CreateNotecardItem {
                request,
                parent,
                name,
                data,
                permissions,
            } => {
                self.create_notecard_item(request, parent, name, data, permissions);
            }
            NetCommand::CreateInventoryItem {
                parent,
                kind,
                name,
                permissions,
            } => {
                let (asset, inventory, subtype) = kind.types();
                let mut m = CreateInventoryItem::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.inventory_block.folder_id = parent;
                m.inventory_block.callback_id = self.inventory_callback(true);
                m.inventory_block.next_owner_mask = permissions.map_or(0x7fff_ffff, |rights| rights.next_owner);
                if let Some(rights) = permissions {
                    self.remember_creation_permissions(m.inventory_block.callback_id, parent, i32::from(asset), rights);
                }
                m.inventory_block.type_ = asset;
                m.inventory_block.inv_type = inventory;
                m.inventory_block.wearable_type = subtype;
                m.inventory_block.name = str_field(&name);
                self.send_main(&m, true);
            }
            NetCommand::RezAttachments(list) => {
                // one object per message, one message per tick
                // (LLAttachmentsMgr::requestAttachments)
                if list.len() > 1 {
                    log::info!(
                        "attachments: {} requests queued, sent one at a time",
                        list.len() + self.attach_queue.len()
                    );
                }
                self.attach_queue.push(list, Instant::now());
                self.send_queued_attachment(Instant::now());
            }
            NetCommand::DetachObject { handle, local_id } => {
                if let Some(addr) = self.sim_for_handle(handle) {
                    self.send(
                        addr,
                        &crate::attachment_detach::packet(self.agent_id(), self.session_id(), local_id),
                        true,
                    );
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
                let moves = self.inventory_move_tx.clone();
                tokio::spawn(async move {
                    let result = crate::outfits::categories::mutate(&http, &cap, &fetch_cap, owner, change, |movement| {
                        inventory_moves::submit(moves, movement)
                    })
                    .await;
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
            NetCommand::IncrementCofVersion => self.increment_cof_version(),
            NetCommand::AvatarTexturesRequest(id) => self.send_generic("avatartexturesrequest", &[id.to_string()]),
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
            | NetCommand::OfferCallingCard { .. }
            | NetCommand::AnswerCallingCard { .. }
            | NetCommand::AgentAnimation { .. }
            | NetCommand::AgentAnimations { .. }
            | NetCommand::AgentAnimationChanges { .. }
            | NetCommand::AnimationStateReset
            | NetCommand::RevokePermissions { .. }
            | NetCommand::ObjectGrab { .. }
            | NetCommand::ObjectGrabUpdate { .. }
            | NetCommand::ObjectRelease { .. }
            | NetCommand::RequestTaskInventory { .. }
            | NetCommand::ChatSession { .. }
            | NetCommand::Call { .. } => {}
            // handled by on_offline_command
            NetCommand::RequestOfflineMessages
            | NetCommand::RequestOfflineMessagesLegacy
            | NetCommand::AnswerOfflineFriendship { .. }
            | NetCommand::AnswerOfflineGroupInvite { .. } => {}
            // handled by on_groups_command
            NetCommand::GroupMembersRequest { .. }
            | NetCommand::GroupRoleDataRequest { .. }
            | NetCommand::GroupRoleMembersRequest { .. }
            | NetCommand::GroupTitlesRequest { .. }
            | NetCommand::GroupTitleUpdate { .. }
            | NetCommand::GroupNoticesListRequest(_)
            | NetCommand::GroupNoticeRequest(_)
            | NetCommand::EjectGroupMembers { .. }
            | NetCommand::GroupRoleChanges { .. }
            | NetCommand::GroupBans { .. }
            | NetCommand::GroupRoleUpdate { .. }
            | NetCommand::UpdateGroupInfo { .. }
            | NetCommand::CreateGroup { .. }
            | NetCommand::JoinGroup(_)
            | NetCommand::InviteToGroup { .. }
            | NetCommand::GroupLandQuery { .. }
            | NetCommand::SetGroupContribution { .. }
            | NetCommand::GroupAccount(_) => {}
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
        if !library && let Some(cap) = self.main_cap("InventoryAPIv3") {
            let http = self.sh.caps_http.clone();
            let events = self.sh.events.clone();
            tokio::spawn(async move {
                for chunk in folders.chunks(16) {
                    match crate::inventory::ais::fetch_folders(&http, &cap, owner, chunk).await {
                        Ok(contents) => {
                            let _ = events.send(NetEvent::InventoryContents(contents));
                        }
                        Err(error) => {
                            log::warn!("inventory fetch failed: {error}");
                            let _ = events.send(NetEvent::InventoryFetchFailed { folders: chunk.to_vec() });
                        }
                    }
                }
            });
            return;
        }
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
                match caps::post_llsd_as(&http, &url, &body, crate::probe::HttpKind::Inventory).await {
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

    /// LLInventoryModelBackgroundFetch::bulkFetchViaAis: AIS reads whole
    /// subtrees (the library through LibraryAPIv3). Without AIS, bulkFetch
    /// asks for the folders alone and the walk goes down one level at a time.
    fn fetch_inventory_tree(
        &mut self,
        request: u64,
        parent: uuid::Uuid,
        children: Vec<uuid::Uuid>,
        depth: i32,
        owner: uuid::Uuid,
        library: bool,
    ) {
        let ais = self.main_cap(if library { "LibraryAPIv3" } else { "InventoryAPIv3" });
        let legacy = self.main_cap(if library {
            "FetchLibDescendents2"
        } else {
            "FetchInventoryDescendents2"
        });
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = if let Some(cap) = ais {
                crate::inventory::ais::fetch_tree(&http, &cap, owner, parent, &children, depth).await
            } else if let Some(url) = legacy {
                let folders = if children.is_empty() { vec![parent] } else { children };
                let body = crate::inventory::fetch_request_body(&folders, owner);
                caps::post_llsd_as(&http, &url, &body, crate::probe::HttpKind::Inventory)
                    .await
                    .map(|v| crate::inventory::parse_fetch_response(&v))
                    .map_err(|_| "inventory read failed")
            } else {
                Err("no inventory capability")
            };
            if let Err(error) = result {
                log::debug!("background inventory fetch of {parent} failed: {error}");
            }
            let _ = events.send(NetEvent::InventoryTreeFetched {
                request,
                result: result.map_err(str::to_owned),
            });
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

    /// send_viewer_stats (indra/newview/llviewerstats.cpp): Linden Lab's
    /// viewer statistics to the agent region's `ViewerStats` capability
    /// (viewer_stats.rs). The schedule moves on even when the capability is
    /// missing, like the Legacy Viewer's timer.
    fn send_viewer_stats(&mut self) {
        let side = self.sh.viewer_stats.lock().clone();
        let sim_fps = self.viewer_stats.sent(Instant::now(), side.frames, side.render_time);
        let Some(url) = self.main_cap("ViewerStats") else {
            log::debug!("no ViewerStats capability: viewer statistics not sent");
            return;
        };
        let counters = crate::viewer_stats::NetCounters::read(&self.sh.stats);
        let unix_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let mut body = crate::viewer_stats::body(&side, &counters, sim_fps, unix_now, self.regions_visited.len());
        log::debug!(
            "sending viewer statistics: {}",
            aurora_llsd::to_xml_string(&crate::viewer_stats::loggable(&body))
        );
        // (added after the log line: the session id never goes to the log)
        if let Llsd::Map(m) = &mut body {
            m.insert("session_id".into(), Llsd::Uuid(self.login.session_id));
        }
        let http = self.sh.caps_http.clone();
        let posts = self.sh.viewer_stats_posts.clone();
        posts.begin();
        tokio::spawn(async move {
            crate::viewer_stats::post(&http, &url, &body).await;
            posts.end();
        });
    }

    /// The capability of the agent's region (cloned for a request task).
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

    /// LLExperienceCache::requestExperiences: the details of experiences, on
    /// a task; the answer always names every one asked for.
    fn request_experiences(&self, ids: Vec<Uuid>) {
        let base = self.main_cap("GetExperienceInfo");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let details = crate::experience::fetch_info(&http, base.as_deref(), &ids).await;
            let _ = events.send(NetEvent::ExperienceInfo(details));
        });
    }

    /// LLExperienceCache::getGroupExperiences, on a task.
    fn request_group_experiences(&self, group: Uuid) {
        let cap = self.main_cap("GroupExperiences");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let ids = crate::experience::fetch_group_experiences(&http, cap.as_deref(), group).await;
            let _ = events.send(NetEvent::GroupExperiences { group, ids });
        });
    }

    /// LLExperienceCache::get / set / forgetExperiencePermission.
    fn experience_permission(&self, experience: Uuid, change: crate::experience::ExperiencePermissionChange) {
        let cap = self.main_cap("ExperiencePreferences");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = crate::experience::permission(&http, cap.as_deref(), experience, change).await;
            if let Err(e) = &result {
                log::warn!("experience {experience} permission ({change:?}): {e}");
            }
            let _ = events.send(NetEvent::ExperiencePermission {
                experience,
                change,
                result,
            });
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

    fn fetch_items(&mut self, items: Vec<uuid::Uuid>, owner: uuid::Uuid, attachment: bool) {
        if owner == self.agent_id()
            && let Some(cap) = self.main_cap("InventoryAPIv3")
        {
            let http = self.sh.caps_http.clone();
            let events = self.sh.events.clone();
            tokio::spawn(async move {
                for chunk in items.chunks(16) {
                    let reads = crate::inventory::ais::read_items(&http, &cap, owner, chunk).await;
                    send_item_reads(&events, reads, attachment);
                }
            });
            return;
        }
        let Some(url) = self.main_cap("FetchInventory2") else {
            emit(self.sh, NetEvent::InventoryItemsFailed { items });
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            for chunk in items.chunks(50) {
                let body = crate::inventory::fetch_items_body(chunk, owner);
                let reads = match caps::post_llsd(&http, &url, &body).await {
                    Ok(v) => crate::inventory::item_reads(&v, chunk),
                    Err(e) => {
                        log::warn!("item fetch failed: {e}");
                        crate::inventory::ais::ItemReads {
                            failed: chunk.to_vec(),
                            ..Default::default()
                        }
                    }
                };
                send_item_reads(&events, reads, attachment);
            }
        });
    }

    /// LLAppearanceMgr::serverAppearanceUpdateCoro: POST the COF version;
    /// the server bakes the outfit and sends a new AvatarAppearance.
    fn request_server_appearance(&mut self, cof_version: i32) {
        transition::step(|| format!("appearance asked (UpdateAvatarAppearance, outfit version {cof_version})"));
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

    /// LLAppearanceMgr::syncCofVersionAndRefreshCoro: GET the capability,
    /// which steps the Current Outfit folder's version on the server and
    /// answers it ({"version": n}); retried as Firestorm does (BAKE_RETRY_*:
    /// five more times, 1, 3, 7, 15 then 31 s apart).
    fn increment_cof_version(&mut self) {
        transition::step(|| "outfit version increment asked (IncrementCOFVersion)".to_string());
        /// BAKE_RETRY_MAX_COUNT (llappearancemgr.cpp).
        const BAKE_RETRY_MAX: u32 = 5;
        let Some(url) = self.main_cap("IncrementCOFVersion") else {
            log::warn!("no IncrementCOFVersion capability: COF version not stepped");
            emit(self.sh, NetEvent::CofVersionIncremented(None));
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let mut retries = 0u32;
            let version = loop {
                let result = http.get(&url).header("Accept", "application/llsd+xml").send().await;
                let reply = match result {
                    Ok(resp) if resp.status().is_success() => {
                        let doc = resp.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok());
                        match doc {
                            Some(d) if d.has("version") => Ok(d["version"].as_i32()),
                            _ => Err("malformed response".to_string()),
                        }
                    }
                    Ok(resp) => Err(format!("HTTP {}", resp.status().as_u16())),
                    Err(e) => Err(e.to_string()),
                };
                match reply {
                    Ok(v) => {
                        log::info!("Current Outfit folder version stepped to {v}");
                        break Some(v);
                    }
                    Err(e) if retries < BAKE_RETRY_MAX => {
                        retries += 1;
                        let wait = 2u64.pow(retries) - 1;
                        log::warn!("COF version increment failed: {e}; retry {retries} in {wait} s");
                        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                    }
                    Err(e) => {
                        log::warn!("COF version increment failed: {e}; giving up");
                        break None;
                    }
                }
            };
            let _ = events.send(NetEvent::CofVersionIncremented(version));
        });
    }

    fn request_objects(&mut self, addr: SocketAddr, ids: &[u32]) {
        crate::probe::objects(crate::probe::ObjectKind::Requested, ids.len());
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
        transition::link(|| format!("event queue of region {} gave up", transition::grid(self.handle_of(addr))));
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
                lost: true,
            },
        );
        self.finished = true;
    }

    fn on_eq_event(&mut self, e: EqEvent) {
        log::debug!("EQ {} from {}", e.message, e.sim);
        if transition::running() && self.main == Some(e.sim) {
            transition::step_once("first event", || format!("first event queue event of the region ({})", e.message));
        }
        let b = &e.body;
        if self.on_inventory_eq(&e) || self.on_experience_eq(&e) {
            return;
        }
        if self.on_social_eq(&e.message, b)
            || self.on_land_eq(&e.message, b)
            || self.on_search_eq(&e.message, b)
            || self.on_groups_eq(&e.message, b)
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
                    // process_enable_simulator (llworld.cpp): no circuit to a
                    // neighbour; the simulator keeps its child agent
                    if self.refuse_neighbours && self.main.is_some_and(|main| main != addr) {
                        transition::link(|| {
                            format!(
                                "EnableSimulator region {} ({addr}) ignored: neighbours off",
                                transition::grid(handle)
                            )
                        });
                        continue;
                    }
                    transition::link(|| {
                        let known = if self.sims.contains_key(&addr) {
                            ", circuit already open"
                        } else {
                            ""
                        };
                        format!("EnableSimulator region {} ({addr}){known}", transition::grid(handle))
                    });
                    self.add_sim(addr, handle, size, false);
                }
            }
            "EstablishAgentCommunication" => {
                let ipport = b["sim-ip-and-port"].as_str();
                transition::link(|| format!("EstablishAgentCommunication for {ipport}"));
                let seed = b["seed-capability"].as_str().to_owned();
                if let Ok(addr) = ipport.parse::<SocketAddr>() {
                    self.set_seed(addr, seed);
                }
            }
            "TeleportFinish" => {
                let info = &b["Info"][0];
                transition::step(|| {
                    format!(
                        "TeleportFinish to region {}, flags {:#x}",
                        transition::grid(info["RegionHandle"].as_u64()),
                        info["TeleportFlags"].as_u64()
                    )
                });
                emit(
                    self.sh,
                    NetEvent::TeleportFinished {
                        handle: info["RegionHandle"].as_u64(),
                        flying: info["TeleportFlags"].as_u64() & (1 << 13) != 0,
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
                transition::begin(
                    transition::Kind::Crossing,
                    || format!("CrossedRegion to region {}", transition::grid(rd["RegionHandle"].as_u64())),
                    &self.sh.stats,
                );
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
            "ParcelVoiceInfo" => self.on_parcel_voice_info(e.sim, b),
            "LandStatReply" => {
                let sim = e.sim;
                self.on_land_stat_reply(sim, b);
            }
            "TeleportFailed" => {
                let reason = b["Info"][0]["Reason"].as_str().to_owned();
                transition::end(
                    Some(transition::Kind::Teleport),
                    &format!("failed (TeleportFailed: {reason})"),
                    &self.sh.stats,
                );
                emit(self.sh, NetEvent::TeleportFailed { reason });
            }
            "DisableSimulator" => {
                transition::link(|| {
                    format!(
                        "DisableSimulator from region {} (event queue)",
                        transition::grid(self.handle_of(e.sim))
                    )
                });
                if self.main != Some(e.sim) {
                    self.remove_sim(e.sim);
                }
            }
            // the region console's output (ConsoleResponseNode,
            // llfloaterregiondebugconsole.cpp)
            "SimConsoleResponse" => {
                emit(self.sh, NetEvent::SimConsoleReply(Ok(crate::sim_console::response_text(b))));
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
        if let Some(sim) = self.sims.get(&addr) {
            transition::step(|| {
                format!(
                    "circuit to region {} already open for {:.1} s (UseCircuitCode {}), capabilities {}",
                    transition::grid(handle),
                    sim.ucc_sent_at.elapsed().as_secs_f32(),
                    if sim.link_up { "acknowledged" } else { "not acknowledged yet" },
                    if sim.caps.is_empty() { "not known yet" } else { "known" }
                )
            });
        }
        let previous = self.main.filter(|m| *m != addr);
        self.add_sim(addr, handle, size, false);
        if let Some(sim) = self.sims.get_mut(&addr) {
            sim.handle = handle;
        }
        self.agent_moved = false;
        self.set_main(addr, seed);
        if let Some(previous) = previous.filter(|p| self.sims.contains_key(p)) {
            transition::step(|| format!("previous region {} kept as a neighbour", transition::grid(self.handle_of(previous))));
        }
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
        if pkt.flags & aurora_msg::PacketFlags::ZEROCODED != 0 {
            stats.zero_coded_in.fetch_add(1, Ordering::Relaxed);
            stats.zero_coded_bytes_in.fetch_add(data.len() as u64, Ordering::Relaxed);
            stats.zero_decoded_bytes_in.fetch_add(pkt.expanded_len as u64, Ordering::Relaxed);
        }
        let Some(sim) = self.sims.get_mut(&from) else {
            log::debug!("packet from {from}, which is not one of our circuits");
            stats.off_circuit.fetch_add(1, Ordering::Relaxed);
            return;
        };
        sim.packets_in += 1;
        crate::probe::message_in(pkt.id, data.len());
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
            || self.dispatch_terrain_raw(from, pkt)?
            || self.dispatch_social(from, pkt)?
            || self.dispatch_estate(from, pkt)?
            || self.dispatch_land(from, pkt)?
            || self.dispatch_search(from, pkt)?
            || self.dispatch_groups(pkt)?
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
            if transition::running() && self.main == Some(from) {
                transition::step_once("first objects", || "first object update of the region (ObjectUpdate)".to_string());
            }
            let objects: Vec<_> = m.object_data.iter().filter_map(objects::parse_full).collect();
            self.attach_queue
                .observe(objects.iter().map(|o| o.name_values.as_str()), Instant::now());
            self.sh.stats.objects_full.fetch_add(objects.len() as u64, Ordering::Relaxed);
            crate::probe::objects(crate::probe::ObjectKind::Full, objects.len());
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
            if transition::running() && self.main == Some(from) {
                transition::step_once("first objects", || {
                    "first object update of the region (ObjectUpdateCompressed)".to_string()
                });
            }
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
            self.attach_queue
                .observe(objects.iter().map(|o| o.name_values.as_str()), Instant::now());
            self.sh.stats.objects_full.fetch_add(objects.len() as u64, Ordering::Relaxed);
            crate::probe::objects(crate::probe::ObjectKind::Compressed, objects.len());
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
            crate::probe::objects(crate::probe::ObjectKind::Terse, updates.len());
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
            if transition::running() && self.main == Some(from) {
                transition::step_once("first objects", || {
                    "first object update of the region (ObjectUpdateCached)".to_string()
                });
            }
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
            crate::probe::objects(crate::probe::ObjectKind::Cached, m.object_data.len());
            crate::probe::objects(crate::probe::ObjectKind::CacheHit, hits.len());
            self.attach_queue
                .observe(hits.iter().map(|o| o.name_values.as_str()), Instant::now());
            crate::probe::objects(crate::probe::ObjectKind::CacheMiss, misses.len());
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
            crate::probe::objects(crate::probe::ObjectKind::Killed, m.object_data.len());
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
            log::info!(
                "script dialog received: channel={}, buttons={}, sender_is_main={}",
                d.chat_channel,
                m.buttons.len(),
                self.main == Some(from),
            );
            let owner = format!("{} {}", field_str(&d.first_name), field_str(&d.last_name));
            emit(
                self.sh,
                NetEvent::ScriptDialog {
                    sender: from,
                    object_id: d.object_id,
                    object_name: field_str(&d.object_name),
                    owner_name: owner.trim().to_owned(),
                    message: field_str(&d.message),
                    channel: d.chat_channel,
                    buttons: m.buttons.iter().map(|b| field_str(&b.button_label)).collect(),
                },
            );
        } else if id == SaveAssetIntoInventory::ID {
            let message: SaveAssetIntoInventory = pkt.decode()?;
            if message.agent_data.agent_id == self.agent_id() {
                emit(self.sh, NetEvent::AttachmentAssetSaved(message.inventory_data.item_id));
            }
        } else if id == ScriptQuestion::ID {
            let m: ScriptQuestion = pkt.decode()?;
            let d = &m.data;
            // the Experience block is missing from older simulators: read
            // as zeros, a nil experience (process_script_question, msg->has)
            emit(
                self.sh,
                NetEvent::ScriptQuestion {
                    task_id: d.task_id,
                    item_id: d.item_id,
                    object_name: field_str(&d.object_name),
                    owner_name: field_str(&d.object_owner),
                    questions: d.questions,
                    experience: m.experience.experience_id,
                    region: self.handle_of(from),
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
        } else if id == ScriptTeleportRequest::ID {
            let m: ScriptTeleportRequest = pkt.decode()?;
            emit(self.sh, script_teleport_request(&m));
        } else if id == MeanCollisionAlert::ID {
            // the Victim of each block is us (the viewer reads gAgentID)
            let m: MeanCollisionAlert = pkt.decode()?;
            let list = m
                .mean_collision
                .iter()
                .map(|b| MeanCollision {
                    perp: b.perp,
                    time: b.time,
                    magnitude: b.mag,
                    kind: b.type_,
                })
                .collect();
            emit(self.sh, NetEvent::MeanCollisions(list));
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
                let info = crate::estate::region_details(&m);
                emit(
                    self.sh,
                    NetEvent::Estate(crate::estate::EstateEvent::RegionInfo {
                        handle,
                        info: Arc::new(info),
                    }),
                );
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
                // the Region / Estate window reads the agent's region's
                if self.main == Some(from) {
                    let stats = crate::estate::SimStats {
                        values: m.stat.iter().map(|s| (s.stat_id, s.stat_value)).collect(),
                        object_capacity: max_tasks,
                    };
                    emit(
                        self.sh,
                        NetEvent::Estate(crate::estate::EstateEvent::Stats {
                            handle,
                            stats: Arc::new(stats),
                        }),
                    );
                }
            }
        } else if id == HealthMessage::ID {
            let m: HealthMessage = pkt.decode()?;
            emit(self.sh, NetEvent::Health(m.health_data.health));
        } else if id == ViewerFrozenMessage::ID {
            // process_frozen_message (freeze.rs)
            let m: ViewerFrozenMessage = pkt.decode()?;
            emit(self.sh, NetEvent::Frozen(m.frozen_data.data));
        } else if id == RegionHandshake::ID {
            let m: RegionHandshake = pkt.decode()?;
            self.on_region_handshake(from, m);
            if self.main == Some(from) {
                transition::step(|| format!("RegionHandshake from region {}, reply sent", transition::grid(self.handle_of(from))));
            }
        } else if id == AgentMovementComplete::ID {
            let m: AgentMovementComplete = pkt.decode()?;
            transition::step(|| format!("AgentMovementComplete from region {}", transition::grid(m.data.region_handle)));
            // (a crossing settles in a few seconds; a teleport ends when the
            // viewer hides its screen, this is the longest it waits)
            transition::arrived(match transition::current() {
                Some(transition::Kind::Crossing) => Duration::from_secs(5),
                _ => Duration::from_secs(20),
            });
            self.agent_moved = true;
            self.regions_visited.insert(m.data.region_handle);
            self.cam_requested_at = None;
            // gLastVersionChannel: the server version the Region / Estate
            // window shows
            emit(
                self.sh,
                NetEvent::Estate(crate::estate::EstateEvent::SimVersion {
                    handle: m.data.region_handle,
                    version: aurora_msg::field_str(&m.sim_data.channel_version),
                }),
            );
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
            self.send_fov(from);
            let mut mb = MoneyBalanceRequest::default();
            mb.agent_data.agent_id = self.agent_id();
            mb.agent_data.session_id = self.session_id();
            self.send(from, &mb, true);
            self.send(from, &EconomyDataRequest::default(), true);
            self.maybe_send_agent_update(Instant::now());
        } else if id == ChatFromSimulator::ID {
            let m: ChatFromSimulator = pkt.decode()?;
            let c = &m.chat_data;
            // typing start/stop are no lines: the typing state of the
            // avatar (name tag, radar, nearby chat), as LLVOAvatar::mTyping
            // CHAT_AUDIBLE_FULLY == 1 (faint = 0, inaudible = -1 → 255)
            let chat_type = ChatType::from_u8(c.chat_type);
            let notice = matches!(chat_type, ChatType::StartTyping | ChatType::StopTyping);
            if notice && c.audible == 1 && c.source_type == 1 {
                emit(
                    self.sh,
                    NetEvent::NearbyTyping {
                        avatar: c.source_id,
                        typing: chat_type == ChatType::StartTyping,
                    },
                );
            }
            if !notice && c.audible == 1 {
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
            // every dialog, IM_TYPING_START / STOP included: the world turns
            // those into "is typing", never into a line
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
        } else if id == msgs::AvatarAppearance::ID {
            let m: msgs::AvatarAppearance = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::Appearance(crate::types::AvatarAppearance {
                    avatar_id: m.sender.id,
                    texture_entry: aurora_prim::parse_texture_entry(&m.object_data.texture_entry).map(Arc::new),
                    visual_params: m.visual_param.iter().map(|p| p.param_value).collect(),
                    cof_version: m.appearance_data.first().map(|a| a.cof_version).unwrap_or(0),
                    appearance_version: m.appearance_data.first().map(|a| a.appearance_version).unwrap_or(0),
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
            self.attach_queue.on_alert(&msg, Instant::now());
            // attempt_standard_notification: a named notification (AlertInfo)
            // is the viewer's to word; then process_alert_core's system
            // messages (OpenSim's restart notices, the cancellation)
            use crate::region_restart::{self, Restart};
            let named = m
                .alert_info
                .first()
                .and_then(|info| region_restart::from_alert_info(&field_str(&info.message), &info.extra_params));
            // the home location's answers (process_special_alert_messages,
            // the HomePositionSet notifications)
            let home = m
                .alert_info
                .first()
                .and_then(|info| crate::home::from_alert_info(&field_str(&info.message)))
                .or_else(|| crate::home::from_text(&msg));
            // who froze us and for how long (AvatarFrozen…, freeze.rs)
            let freeze = m
                .alert_info
                .first()
                .and_then(|info| crate::freeze::from_alert_info(&field_str(&info.message), &info.extra_params));
            if let Some(alert) = freeze {
                emit(self.sh, NetEvent::FreezeAlert(alert));
            } else if let Some(alert) = home {
                emit(self.sh, NetEvent::HomeAlert(alert));
            } else {
                match named.map(Restart::Notice).or_else(|| region_restart::from_text(&msg)) {
                    Some(Restart::Notice(notice)) => emit(self.sh, NetEvent::RegionRestart(notice)),
                    Some(Restart::Cancelled) => emit(self.sh, NetEvent::RegionRestartCancelled),
                    None if !msg.is_empty() => emit(self.sh, NetEvent::Alert { message: msg }),
                    None => {}
                }
            }
        } else if id == AgentAlertMessage::ID {
            let m: AgentAlertMessage = pkt.decode()?;
            let message = field_str(&m.alert_data.message);
            self.attach_queue.on_alert(&message, Instant::now());
            match crate::home::from_text(&message) {
                Some(alert) => emit(self.sh, NetEvent::HomeAlert(alert)),
                None => emit(self.sh, NetEvent::Alert { message }),
            }
        } else if id == UserInfoReply::ID {
            // process_user_info_reply: the UDP answer to UserInfoRequest
            let m: UserInfoReply = pkt.decode()?;
            emit(self.sh, NetEvent::UserInfo(Ok(crate::user_info::from_udp(&m))));
        } else if id == ScriptControlChange::ID {
            // LLAgent::processScriptControlChange: the counts are the viewer's
            let m: ScriptControlChange = pkt.decode()?;
            let blocks = m.data.iter().map(|d| (d.take_controls, d.controls, d.pass_to_agent)).collect();
            emit(self.sh, NetEvent::ScriptControlChange(blocks));
        } else if id == KickUser::ID {
            let m: KickUser = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::Disconnected {
                    reason: field_str(&m.user_info.reason),
                    lost: false,
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
                        lost: true,
                    },
                );
                self.finished = true;
            } else {
                transition::link(|| format!("DisableSimulator from region {}", transition::grid(self.handle_of(from))));
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
            if let Some((want, request)) = self.pending_named_tp.clone() {
                // MapSearchModule::AddFinalBlock repeats the requested name
                // with access 255: that terminator is never a destination.
                let found = m.data.iter().find(|d| d.access != 255 && field_str(&d.name).to_lowercase() == want);
                match found {
                    Some(d) => {
                        self.pending_named_tp = None;
                        let handle = ((d.x as u64 * 256) << 32) | (d.y as u64 * 256);
                        emit(
                            self.sh,
                            NetEvent::TeleportRegionResolved {
                                request,
                                result: Ok(handle),
                            },
                        );
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
                            NetEvent::TeleportRegionResolved {
                                request,
                                result: Err(aurora_i18n::tr!("session-region-not-found", want = want.to_string())),
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
        } else if id == FindAgent::ID {
            // LLAvatarTracker::processAgentFound: the answer reuses the request
            let m: FindAgent = pkt.decode()?;
            if let Some(l) = m.location_block.first() {
                emit(
                    self.sh,
                    NetEvent::AgentFound {
                        prey: m.agent_block.prey,
                        x: l.global_x,
                        y: l.global_y,
                    },
                );
            }
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
            self.on_experience_generic(
                m.agent_data.agent_id,
                &m.method_data.method,
                m.method_data.invoice,
                &m.param_list
                    .iter()
                    .take(3)
                    .map(|parameter| parameter.parameter.as_slice())
                    .collect::<Vec<_>>(),
            );
            if let Some(parameter) = m.param_list.first() {
                self.on_inventory_generic(m.agent_data.agent_id, &m.method_data.method, &parameter.parameter);
            }
        } else if id == LargeGenericMessage::ID {
            let m: LargeGenericMessage = pkt.decode()?;
            self.on_experience_generic(
                m.agent_data.agent_id,
                &m.method_data.method,
                m.method_data.invoice,
                &m.param_list
                    .iter()
                    .take(3)
                    .map(|parameter| parameter.parameter.as_slice())
                    .collect::<Vec<_>>(),
            );
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
                let folders: Vec<_> = m
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
                let delivered = folders.iter().map(|folder: &crate::inventory::InvFolder| folder.id).collect();
                emit(self.sh, NetEvent::InventoryUpdated { folders, items });
                if !m.agent_data.transaction_id.is_nil() {
                    emit(
                        self.sh,
                        NetEvent::InventoryDelivery {
                            transaction: m.agent_data.transaction_id,
                            folders: delivered,
                        },
                    );
                }
                for item in created {
                    emit(self.sh, NetEvent::InventoryCreated(item));
                }
            }
        } else if id == AvatarAnimation::ID {
            let m: AvatarAnimation = pkt.decode()?;
            // Our old simulator can still send reliable animation packets
            // after a teleport. Only the current agent circuit owns our state.
            if m.sender.id == self.agent_id() && self.main != Some(from) {
                return Ok(());
            }
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
            if m.transaction_info.transaction_type != 0 {
                emit(
                    self.sh,
                    NetEvent::MoneyTransaction {
                        id: m.money_data.transaction_id,
                        success: m.money_data.transaction_success,
                        kind: m.transaction_info.transaction_type,
                        source: m.transaction_info.source_id,
                        source_group: m.transaction_info.is_source_group,
                        dest: m.transaction_info.dest_id,
                        dest_group: m.transaction_info.is_dest_group,
                        amount: m.transaction_info.amount,
                        item: field_str(&m.transaction_info.item_description),
                        description: field_str(&m.money_data.description),
                    },
                );
            }
            emit(self.sh, NetEvent::Balance(m.money_data.money_balance));
            emit(
                self.sh,
                NetEvent::LandHoldings {
                    credit: m.money_data.square_meters_credit,
                    committed: m.money_data.square_meters_committed,
                },
            );
        } else if id == TeleportStart::ID {
            if transition::current() == Some(transition::Kind::Teleport) {
                transition::step(|| "TeleportStart".to_string());
            } else {
                transition::begin(
                    transition::Kind::Teleport,
                    || "TeleportStart (begun by the simulator)".to_string(),
                    &self.sh.stats,
                );
            }
            emit(self.sh, NetEvent::TeleportStarted);
        } else if id == TeleportProgress::ID {
            let m: TeleportProgress = pkt.decode()?;
            transition::step(|| format!("TeleportProgress '{}'", field_str(&m.info.message)));
            emit(
                self.sh,
                NetEvent::TeleportProgress {
                    message: field_str(&m.info.message),
                },
            );
        } else if id == TeleportFailed::ID {
            let m: TeleportFailed = pkt.decode()?;
            transition::end(
                Some(transition::Kind::Teleport),
                &format!("failed (TeleportFailed: {})", field_str(&m.info.reason)),
                &self.sh.stats,
            );
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
                } else if let Some((source, target, offset, kind)) = parse_pointat(e) {
                    emit(
                        self.sh,
                        NetEvent::PointAt {
                            effect: e.id,
                            source,
                            target,
                            offset,
                            kind,
                            duration: e.duration,
                        },
                    );
                } else if let Some((target, animations)) = parse_reset_skeleton(e) {
                    emit(
                        self.sh,
                        NetEvent::ResetSkeleton {
                            source: e.agent_id,
                            target,
                            animations,
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
            emit(
                self.sh,
                NetEvent::PreloadSounds(
                    m.data_block
                        .iter()
                        .map(|b| SoundPreload {
                            sound: b.sound_id,
                            object: b.object_id,
                            owner: b.owner_id,
                        })
                        .collect(),
                ),
            );
        } else if id == TeleportLocal::ID {
            let m: TeleportLocal = pkt.decode()?;
            transition::step(|| "TeleportLocal (inside the region)".to_string());
            transition::arrived(Duration::from_secs(2));
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
                sim.region_id = m.region_info2.region_id;
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
            billable_factor: r.billable_factor,
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
        self.request_terrain_transforms(handle);
    }
}

/// LLHUDObject::LL_HUD_EFFECT_LOOKAT.
const HUD_EFFECT_LOOKAT: u8 = 14;
/// LLHUDObject::LL_HUD_EFFECT_POINTAT; same binary layout as look-at.
const HUD_EFFECT_POINTAT: u8 = 15;
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
                expanded_len: 0,
            };
            assert!(follow_cam_event(&packet, 10).is_err());
        }
    }
}

/// llMapDestination, as process_script_teleport_request reads it
/// (indra/newview/llviewermessage.cpp, originally LGPL 2.1): without an
/// Options block the map opens and centres on the place.
fn script_teleport_request(m: &ScriptTeleportRequest) -> NetEvent {
    NetEvent::ScriptTeleportRequest {
        object_name: field_str(&m.data.object_name),
        region: field_str(&m.data.sim_name),
        position: m.data.sim_position,
        flags: m.options.first().map_or(beacon::SHOW_MAP | beacon::FOCUS_MAP, |o| o.flags),
    }
}

#[cfg(test)]
mod script_teleport_tests {
    use super::*;

    fn request(options: Vec<u32>) -> ScriptTeleportRequest {
        let mut m = ScriptTeleportRequest::default();
        m.data.object_name = str_field("Club door");
        m.data.sim_name = str_field("YIFF");
        m.data.sim_position = Vec3::new(168.4, 149.6, 3010.2);
        m.options = options
            .into_iter()
            .map(|flags| msgs::script_teleport_request::Options { flags })
            .collect();
        m
    }

    fn decoded(m: &ScriptTeleportRequest) -> ScriptTeleportRequest {
        let mut bytes = Vec::new();
        aurora_msg::build_packet(m, 1, true, false, &[], &mut bytes);
        parse_packet(&bytes).expect("packet").decode().expect("decode")
    }

    #[test]
    fn map_destination_reads_the_place_and_opens_the_map_by_default() {
        match script_teleport_request(&decoded(&request(vec![]))) {
            NetEvent::ScriptTeleportRequest {
                object_name,
                region,
                position,
                flags,
            } => {
                assert_eq!((object_name.as_str(), region.as_str()), ("Club door", "YIFF"));
                assert_eq!(position, Vec3::new(168.4, 149.6, 3010.2));
                assert_eq!(flags, beacon::SHOW_MAP | beacon::FOCUS_MAP);
            }
            event => panic!("unexpected {event:?}"),
        }
    }

    #[test]
    fn map_destination_keeps_the_options_the_script_sent() {
        for flags in [0, beacon::SHOW_MAP, beacon::FOCUS_MAP] {
            assert!(matches!(
                script_teleport_request(&decoded(&request(vec![flags]))),
                NetEvent::ScriptTeleportRequest { flags: f, .. } if f == flags
            ));
        }
    }
}

fn parse_lookat(e: &msgs::viewer_effect::Effect) -> Option<(uuid::Uuid, uuid::Uuid, [f64; 3], u8)> {
    parse_attention(e, HUD_EFFECT_LOOKAT)
}

fn pointat_effect(
    agent: uuid::Uuid,
    effect: uuid::Uuid,
    target: uuid::Uuid,
    offset: [f64; 3],
    kind: u8,
    duration: f32,
) -> msgs::viewer_effect::Effect {
    let mut e = lookat_effect(agent, effect, target, offset, kind, duration);
    e.type_ = HUD_EFFECT_POINTAT;
    e
}

fn parse_pointat(e: &msgs::viewer_effect::Effect) -> Option<(uuid::Uuid, uuid::Uuid, [f64; 3], u8)> {
    parse_attention(e, HUD_EFFECT_POINTAT)
}

fn parse_attention(e: &msgs::viewer_effect::Effect, effect_type: u8) -> Option<(uuid::Uuid, uuid::Uuid, [f64; 3], u8)> {
    if e.type_ != effect_type || e.type_data.len() != LOOKAT_PKT_SIZE {
        return None;
    }
    let d = &e.type_data;
    let source = uuid::Uuid::from_slice(&d[0..16]).ok()?;
    let target = uuid::Uuid::from_slice(&d[16..32]).ok()?;
    let mut offset = [0.0f64; 3];
    for (i, v) in offset.iter_mut().enumerate() {
        let b: [u8; 8] = d[32 + i * 8..40 + i * 8].try_into().ok()?;
        *v = f64::from_le_bytes(b);
        if !v.is_finite() {
            return None;
        }
    }
    Some((source, target, offset, d[56]))
}

/// LLHUDObject::LL_HUD_EFFECT_RESET_SKELETON.
const HUD_EFFECT_RESET_SKELETON: u8 = 19;
/// LLHUDEffectResetSkeleton type data: target object (16), then a byte of
/// which only bit 0 is read (reset the animations too).
const RESET_SKELETON_PKT_SIZE: usize = 17;

#[cfg(test)]
mod pointat_tests {
    use super::*;

    #[test]
    fn pointat_roundtrip_matches_the_legacy_packet_and_rejects_bad_data() {
        let source = uuid::Uuid::new_v4();
        let target = uuid::Uuid::new_v4();
        let effect = uuid::Uuid::new_v4();
        let mut packet = ViewerEffect::default();
        packet
            .effect
            .push(pointat_effect(source, effect, target, [0.125, -2.5, 300.0], 1, f32::MAX / 4.0));
        let mut data = aurora_msg::Writer::new();
        packet.encode_body(&mut data);
        let decoded = ViewerEffect::decode(&data.buf).expect("decode effect");
        let e = &decoded.effect[0];
        assert_eq!(e.type_, 15);
        assert_eq!(e.id, effect);
        assert_eq!(e.type_data.len(), 57);
        assert_eq!(&e.type_data[32..40], &0.125f64.to_le_bytes());
        assert_eq!(parse_pointat(e), Some((source, target, [0.125, -2.5, 300.0], 1)));
        assert!(parse_lookat(e).is_none());
        let mut invalid = e.clone();
        invalid.type_data[32..40].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(parse_pointat(&invalid).is_none());
        invalid.type_data.pop();
        assert!(parse_pointat(&invalid).is_none());
    }
}

/// LLHUDEffectResetSkeleton::packData, with LLHUDEffect's defaults: a 1 s
/// duration and an unused colour (LLColor4U(): opaque black).
fn reset_skeleton_effect(agent: uuid::Uuid, effect: uuid::Uuid, target: uuid::Uuid, animations: bool) -> msgs::viewer_effect::Effect {
    let mut d = Vec::with_capacity(RESET_SKELETON_PKT_SIZE);
    d.extend_from_slice(target.as_bytes());
    d.push(u8::from(animations));
    msgs::viewer_effect::Effect {
        id: effect,
        agent_id: agent,
        type_: HUD_EFFECT_RESET_SKELETON,
        duration: 1.0,
        color: vec![0, 0, 0, 255],
        type_data: d,
    }
}

/// LLHUDEffectResetSkeleton::unpackData: (target, reset the animations); a
/// nil target is the sender itself, a packet of another size is dropped.
fn parse_reset_skeleton(e: &msgs::viewer_effect::Effect) -> Option<(uuid::Uuid, bool)> {
    if e.type_ != HUD_EFFECT_RESET_SKELETON || e.type_data.len() != RESET_SKELETON_PKT_SIZE {
        return None;
    }
    let target = uuid::Uuid::from_slice(&e.type_data[0..16]).ok()?;
    let target = if target.is_nil() { e.agent_id } else { target };
    Some((target, e.type_data[16] & 1 != 0))
}

/// AgentFOV as LLViewerCamera::setView fills it: the generation counter
/// stays 0.
fn agent_fov(agent_id: Uuid, session_id: Uuid, circuit_code: u32, vertical_angle: f32) -> AgentFOV {
    let mut fov = AgentFOV::default();
    fov.agent_data.agent_id = agent_id;
    fov.agent_data.session_id = session_id;
    fov.agent_data.circuit_code = circuit_code;
    fov.fov_block.gen_counter = 0;
    fov.fov_block.vertical_angle = vertical_angle;
    fov
}

#[cfg(test)]
mod agent_fov_tests {
    use super::*;

    #[test]
    fn agent_fov_carries_the_real_angle() {
        let m = agent_fov(Uuid::from_u128(1), Uuid::from_u128(2), 77, 0.75);
        assert_eq!(m.agent_data.agent_id, Uuid::from_u128(1));
        assert_eq!(m.agent_data.session_id, Uuid::from_u128(2));
        assert_eq!(m.agent_data.circuit_code, 77);
        assert_eq!(m.fov_block.gen_counter, 0);
        assert_eq!(m.fov_block.vertical_angle, 0.75);
        // until the viewer gives its own: 60°, not the 1.0 rad sent before
        assert!((DEFAULT_FOV - 60f32.to_radians()).abs() < 1e-6);
    }
}

#[cfg(test)]
mod reset_skeleton_tests {
    use super::*;

    #[test]
    fn reset_skeleton_effect_round_trips_like_firestorm() {
        let (me, effect) = (uuid::Uuid::from_u128(7), uuid::Uuid::from_u128(9));
        let e = reset_skeleton_effect(me, effect, me, true);
        assert_eq!(e.type_, 19);
        assert_eq!(e.type_data.len(), 17);
        assert_eq!(&e.type_data[..16], me.as_bytes());
        assert_eq!(e.type_data[16], 1);
        assert_eq!(parse_reset_skeleton(&e), Some((me, true)));
        let e = reset_skeleton_effect(me, effect, me, false);
        assert_eq!(parse_reset_skeleton(&e), Some((me, false)));
    }

    #[test]
    fn a_nil_target_is_the_sender_and_only_bit_0_counts() {
        let sender = uuid::Uuid::from_u128(5);
        let mut e = reset_skeleton_effect(sender, uuid::Uuid::from_u128(1), uuid::Uuid::nil(), false);
        e.type_data[16] = 0b10;
        assert_eq!(parse_reset_skeleton(&e), Some((sender, false)));
        // wrong size or type: not a reset
        e.type_data.push(0);
        assert_eq!(parse_reset_skeleton(&e), None);
        let look = lookat_effect(sender, uuid::Uuid::from_u128(1), sender, [0.0; 3], 0, 1.0);
        assert_eq!(parse_reset_skeleton(&look), None);
    }
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

    fn parcel_event(sim: SocketAddr, sequence: i32, local_id: i32, flags: u32) -> EqEvent {
        EqEvent {
            sim,
            message: "ParcelProperties".into(),
            body: aurora_llsd::llsd_map! {
                "ParcelData" => Llsd::Array(vec![aurora_llsd::llsd_map! {
                    "RequestResult" => crate::land::PARCEL_RESULT_SUCCESS,
                    "SequenceID" => sequence,
                    "LocalID" => local_id,
                    "ParcelFlags" => Llsd::Binary(flags.to_be_bytes().to_vec()),
                    "Name" => "Updated parcel",
                    "Bitmap" => Llsd::Binary(vec![0x42; 512]),
                    "AABBMin" => Llsd::Array(vec![0.into(), 0.into(), 0.into()]),
                    "AABBMax" => Llsd::Array(vec![128.into(), 128.into(), 0.into()]),
                    "SeeAVs" => flags != 0,
                    "AnyAVSounds" => flags != 0,
                    "GroupAVSounds" => flags != 0,
                    "MusicURL" => "https://music.invalid/live",
                    "MediaURL" => "https://media.invalid/page"
                }]),
                "ParcelEnvironmentBlock" => Llsd::Array(vec![aurora_llsd::llsd_map! {
                    "ParcelEnvironmentVersion" => 3
                }]),
                "ParcelExtendedFlags" => Llsd::Array(vec![aurora_llsd::llsd_map! {
                    "Flags" => Llsd::Binary((if flags != 0 { crate::land::PEF_OBSCURE_MOAP } else { 0 }).to_be_bytes().to_vec())
                }])
            },
        }
    }

    fn parcel_block<'a>(event: &'a mut EqEvent, name: &str) -> &'a mut Llsd {
        let Llsd::Map(blocks) = &mut event.body else { panic!("event map") };
        let Some(Llsd::Array(rows)) = blocks.get_mut(name) else {
            panic!("parcel block")
        };
        rows.first_mut().expect("parcel row")
    }

    fn voice_event(sim: SocketAddr, local_id: i32, uri: Llsd) -> EqEvent {
        EqEvent {
            sim,
            message: "ParcelVoiceInfo".into(),
            body: aurora_llsd::llsd_map! {
                "parcel_local_id" => local_id,
                "voice_credentials" => aurora_llsd::llsd_map! { "channel_uri" => uri }
            },
        }
    }

    #[tokio::test]
    async fn parcel_voice_notifications_disable_reenable_and_switch_spatial_channels() {
        use crate::parcel_flags as pf;
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = "127.0.0.1:13001".parse().unwrap();
        let (events, received) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock, main, "127.0.0.1:13002".parse().unwrap());
        let region = Uuid::from_u128(1247);
        let mut handshake = RegionHandshake::default();
        handshake.region_info2.region_id = region;
        session.on_region_handshake(main, handshake);
        session.on_eq_event(parcel_event(
            main,
            7,
            42,
            pf::SOUND_LOCAL | pf::ALLOW_VOICE_CHAT | pf::USE_ESTATE_VOICE_CHAN,
        ));
        let original = session.agent_parcel.clone().unwrap();
        received.try_iter().for_each(drop);
        for (uri, expected) in [
            (Llsd::from(""), pf::SOUND_LOCAL),
            (
                Llsd::from(region.to_string()),
                pf::SOUND_LOCAL | pf::ALLOW_VOICE_CHAT | pf::USE_ESTATE_VOICE_CHAN,
            ),
            (Llsd::from(format!("{region}-42")), pf::SOUND_LOCAL | pf::ALLOW_VOICE_CHAT),
            (Llsd::from(""), pf::SOUND_LOCAL),
            (
                Llsd::Uuid(region),
                pf::SOUND_LOCAL | pf::ALLOW_VOICE_CHAT | pf::USE_ESTATE_VOICE_CHAN,
            ),
        ] {
            session.on_eq_event(voice_event(main, 42, uri.clone()));
            assert!(received.try_iter().any(|(_, e)| matches!(e, NetEvent::ParcelVoiceInfo {
                handle: 10, local_id: 42, allowed, use_estate: Some(estate)
            } if allowed == (expected & pf::ALLOW_VOICE_CHAT != 0) && estate == (expected & pf::USE_ESTATE_VOICE_CHAN != 0))));
            let updated = session.agent_parcel.clone().unwrap();
            assert_eq!(updated.flags, expected);
            assert_eq!(updated.bitmap, original.bitmap);
            assert_eq!(updated.music_url, original.music_url);
            session.on_eq_event(voice_event(main, 42, uri));
            assert!(received.is_empty(), "identical voice notification preserves live caches");
            assert!(Arc::ptr_eq(session.agent_parcel.as_ref().unwrap(), &updated));
        }
    }

    #[tokio::test]
    async fn parcel_voice_notifications_reject_other_parcels_regions_and_malformed_data() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = "127.0.0.1:13001".parse().unwrap();
        let other = "127.0.0.1:13002".parse().unwrap();
        let (events, received) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock, main, other);
        session.on_eq_event(parcel_event(main, 7, 0, crate::parcel_flags::ALLOW_VOICE_CHAT));
        let original = session.agent_parcel.clone().unwrap();
        received.try_iter().for_each(drop);
        let mut missing_id = voice_event(main, 0, "".into());
        if let Llsd::Map(body) = &mut missing_id.body {
            body.remove("parcel_local_id");
        }
        for event in [
            voice_event(other, 0, "".into()),
            voice_event(main, 42, "".into()),
            voice_event(main, 0, Llsd::Undef),
            missing_id,
        ] {
            session.on_eq_event(event);
            assert!(received.is_empty());
            assert!(Arc::ptr_eq(session.agent_parcel.as_ref().unwrap(), &original));
        }
    }

    #[tokio::test]
    async fn early_parcel_voice_notifications_wait_for_the_matching_agent_parcel() {
        for local_id in [42, 43] {
            let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
            let main = "127.0.0.1:13001".parse().unwrap();
            let (events, received) = crossbeam_channel::unbounded();
            let sh = shared(crate::EventSender(events));
            let mut session = two_regions(&sh, sock, main, "127.0.0.1:13002".parse().unwrap());
            session.on_eq_event(voice_event(main, 42, "".into()));
            assert!(received.is_empty());
            session.on_eq_event(parcel_event(main, 7, local_id, crate::parcel_flags::ALLOW_VOICE_CHAT));
            let current = session.agent_parcel.as_ref().unwrap();
            assert_eq!(current.flags & crate::parcel_flags::ALLOW_VOICE_CHAT != 0, local_id != 42);
            assert!(session.sims[&main].pending_voice.is_none());
            // A later confirmed properties update is authoritative again.
            session.on_eq_event(parcel_event(main, 8, local_id, crate::parcel_flags::ALLOW_VOICE_CHAT));
            assert_ne!(
                session.agent_parcel.as_ref().unwrap().flags & crate::parcel_flags::ALLOW_VOICE_CHAT,
                0
            );
        }
    }

    #[tokio::test]
    async fn parcel_selection_refreshes_live_permissions_without_replacing_agent_geometry() {
        use crate::{land, parcel_flags as pf};
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = "127.0.0.1:13001".parse().unwrap();
        let (events, received) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock, main, "127.0.0.1:13002".parse().unwrap());
        let mut initial = parcel_event(main, 7, 42, 0);
        parcel_block(&mut initial, "ParcelData").insert("MusicURL", "https://music.invalid/old");
        parcel_block(&mut initial, "ParcelData").insert("MediaURL", "https://media.invalid/old");
        session.on_eq_event(initial);
        let original = session.agent_parcel.clone().expect("agent parcel");
        received.try_iter().for_each(drop);

        let flags = pf::ALLOW_VOICE_CHAT
            | pf::USE_ESTATE_VOICE_CHAN
            | pf::SOUND_LOCAL
            | pf::ALLOW_FLY
            | pf::CREATE_OBJECTS
            | pf::ALLOW_OTHER_SCRIPTS
            | pf::RESTRICT_PUSHOBJECT;
        for sequence in [land::SELECTED_PARCEL_SEQ_ID, land::HOVERED_PARCEL_SEQ_ID, BUILD_PARCEL_SEQ + 1] {
            for flags in [flags, 0] {
                let mut event = parcel_event(main, sequence, 42, flags);
                // A selection rectangle/hover must not become the agent's parcel mask.
                parcel_block(&mut event, "ParcelData").insert("Bitmap", Llsd::Binary(Vec::new()));
                parcel_block(&mut event, "ParcelData").insert("AABBMax", Llsd::Array(vec![4.into(), 4.into(), 0.into()]));
                session.on_eq_event(event);
                let events: Vec<_> = received.try_iter().map(|(_, event)| event).collect();
                let NetEvent::AgentParcel(parcel) = &events[0] else {
                    panic!("live permissions before selection observers")
                };
                assert_eq!(parcel.flags, flags);
                assert_eq!(parcel.see_avatars, flags != 0);
                assert_eq!(parcel.any_av_sounds, flags != 0);
                assert_eq!(parcel.group_av_sounds, flags != 0);
                assert_eq!(parcel.media.obscure_moap, flags != 0);
                assert_eq!(parcel.music_url, "https://music.invalid/live");
                assert_eq!(parcel.media_url, "https://media.invalid/page");
                assert_eq!(parcel.bitmap, original.bitmap);
                assert_eq!(parcel.aabb_max, original.aabb_max);
                assert_eq!(parcel.sequence_id, original.sequence_id);
                assert_eq!(events.len(), 2);
                assert!(match &events[1] {
                    NetEvent::Land(land::LandEvent::Selected { .. }) => sequence == land::SELECTED_PARCEL_SEQ_ID,
                    NetEvent::HoverParcel { .. } => sequence == land::HOVERED_PARCEL_SEQ_ID,
                    NetEvent::SelectedParcel { .. } => sequence == BUILD_PARCEL_SEQ + 1,
                    _ => false,
                });
                let current = session.agent_parcel.clone().unwrap();
                session.on_eq_event(parcel_event(main, sequence, 42, flags));
                assert!(!received.try_iter().any(|(_, e)| matches!(e, NetEvent::AgentParcel(_))));
                assert!(
                    Arc::ptr_eq(session.agent_parcel.as_ref().unwrap(), &current),
                    "identical refresh preserves live caches"
                );
            }
        }
    }

    #[tokio::test]
    async fn parcel_replies_for_other_land_or_regions_cannot_change_live_permissions() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = "127.0.0.1:13001".parse().unwrap();
        let other = "127.0.0.1:13002".parse().unwrap();
        let (events, received) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock, main, other);
        session.on_eq_event(parcel_event(main, crate::land::SELECTED_PARCEL_SEQ_ID, 42, 0));
        assert!(session.agent_parcel.is_none(), "a selection alone cannot locate the agent");
        assert!(!received.try_iter().any(|(_, e)| matches!(e, NetEvent::AgentParcel(_))));
        session.on_eq_event(parcel_event(main, 7, 42, 0));
        let original = session.agent_parcel.clone().unwrap();
        received.try_iter().for_each(drop);
        for sequence in [
            crate::land::SELECTED_PARCEL_SEQ_ID,
            crate::land::HOVERED_PARCEL_SEQ_ID,
            BUILD_PARCEL_SEQ + 1,
        ] {
            for (sim, local_id) in [(main, 43), (other, 42)] {
                session.on_eq_event(parcel_event(sim, sequence, local_id, crate::parcel_flags::ALLOW_VOICE_CHAT));
                assert!(!received.try_iter().any(|(_, e)| matches!(e, NetEvent::AgentParcel(_))));
                assert!(Arc::ptr_eq(session.agent_parcel.as_ref().unwrap(), &original));
            }
        }
    }

    #[tokio::test]
    async fn parcel_notifications_reject_no_data_and_old_sequences_but_accept_zero() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = "127.0.0.1:13001".parse().unwrap();
        let (events, received) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock, main, "127.0.0.1:13002".parse().unwrap());
        session.on_eq_event(parcel_event(main, 7, 42, 0));
        let original = session.agent_parcel.clone().unwrap();
        received.try_iter().for_each(drop);
        for sequence in [
            0,
            8,
            crate::land::SELECTED_PARCEL_SEQ_ID,
            crate::land::HOVERED_PARCEL_SEQ_ID,
            BUILD_PARCEL_SEQ + 1,
        ] {
            let mut event = parcel_event(main, sequence, 42, crate::parcel_flags::ALLOW_VOICE_CHAT);
            parcel_block(&mut event, "ParcelData").insert("RequestResult", crate::land::PARCEL_RESULT_NO_DATA);
            session.on_eq_event(event);
            assert!(received.is_empty());
            assert!(Arc::ptr_eq(session.agent_parcel.as_ref().unwrap(), &original));
        }
        for sequence in [6, 7] {
            session.on_eq_event(parcel_event(main, sequence, 43, crate::parcel_flags::ALLOW_VOICE_CHAT));
            assert!(received.is_empty());
            assert!(Arc::ptr_eq(session.agent_parcel.as_ref().unwrap(), &original));
        }
        for (sequence, local_id) in [(8, 43), (0, 42)] {
            session.on_eq_event(parcel_event(main, sequence, local_id, crate::parcel_flags::ALLOW_VOICE_CHAT));
            assert!(
                received
                    .try_iter()
                    .any(|(_, e)| matches!(e, NetEvent::AgentParcel(p) if p.local_id == local_id))
            );
        }
    }

    #[tokio::test]
    async fn parcel_selection_reloads_the_live_environment_only_when_its_version_changes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = "127.0.0.1:13001".parse().unwrap();
        let (events, _) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock, main, "127.0.0.1:13002".parse().unwrap());
        session.on_eq_event(parcel_event(main, 7, 42, 0));
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        session.sims.get_mut(&main).unwrap().caps = Arc::new(HashMap::from([(
            "ExtEnvironment".into(),
            format!("http://{}/environment", server.local_addr().unwrap()),
        )]));
        let mut event = parcel_event(main, crate::land::SELECTED_PARCEL_SEQ_ID, 42, 0);
        parcel_block(&mut event, "ParcelEnvironmentBlock").insert("ParcelEnvironmentVersion", 4);
        session.on_eq_event(event);
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), server.accept())
            .await
            .unwrap()
            .unwrap();
        let mut request = vec![0; 4096];
        let size = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut request))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET /environment?parcelid=42 "));
        let body = aurora_llsd::to_xml(&Llsd::new_map());
        stream
            .write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        stream.write_all(&body).await.unwrap();
        crate::test_http::finish(stream);
        let mut event = parcel_event(main, crate::land::SELECTED_PARCEL_SEQ_ID, 42, 0);
        parcel_block(&mut event, "ParcelEnvironmentBlock").insert("ParcelEnvironmentVersion", 4);
        session.on_eq_event(event);
        assert!(tokio::time::timeout(Duration::from_millis(200), server.accept()).await.is_err());
        assert_eq!(session.agent_parcel.as_ref().unwrap().env_version, 4);
    }

    #[tokio::test]
    async fn remote_teleport_flags_reach_the_viewer_and_old_circuits_cannot_replace_own_animations() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main: SocketAddr = "127.0.0.1:13001".parse().unwrap();
        let old: SocketAddr = "127.0.0.1:13002".parse().unwrap();
        let (events, received) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock, main, old);
        for flags in [0, 1 << 13, 1 << 2] {
            session.on_eq_event(EqEvent {
                sim: main,
                message: "TeleportFinish".into(),
                body: aurora_llsd::llsd_map! {
                    "Info" => Llsd::Array(vec![aurora_llsd::llsd_map! {
                        "RegionHandle" => 10,
                        "TeleportFlags" => flags
                    }])
                },
            });
            assert!(received.try_iter().any(|(_, event)| matches!(event,
                NetEvent::TeleportFinished { handle: 10, flying } if flying == (flags & (1 << 13) != 0)
            )));
        }
        let own = session.agent_id();
        for (avatar, source, accepted) in [(own, old, false), (own, main, true), (Uuid::from_u128(33), old, true)] {
            let mut message = AvatarAnimation::default();
            message.sender.id = avatar;
            let mut bytes = Vec::new();
            aurora_msg::build_packet(&message, 1, true, false, &[], &mut bytes);
            session
                .dispatch(source, &parse_packet(&bytes).expect("animation packet"))
                .expect("dispatch");
            assert_eq!(
                received
                    .try_iter()
                    .any(|(_, event)| matches!(event, NetEvent::AvatarAnimations { avatar: id, .. } if id == avatar)),
                accepted
            );
        }
    }

    #[tokio::test]
    async fn dropping_a_circuit_cancels_its_pending_simulator_features() {
        let addr = "127.0.0.1:13001".parse().expect("local address");
        let mut sim = Sim::new(addr, 10);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (finished, canceled) = tokio::sync::oneshot::channel::<()>();
        sim.features_task = Some(tokio::spawn(async move {
            let _ = started.send(());
            std::future::pending::<()>().await;
            let _ = finished.send(());
        }));
        ready.await.expect("feature request started");
        drop(sim);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), canceled)
                .await
                .expect("request canceled")
                .is_err()
        );
    }

    async fn finished_task() -> JoinHandle<()> {
        let task = tokio::spawn(async {});
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
        task
    }

    fn shared(events: crate::EventSender) -> Shared {
        Shared {
            http: reqwest::Client::new(),
            caps_http: reqwest::Client::new(),
            events,
            controls: Arc::new(Mutex::new(AgentControls::default())),
            stats: Arc::new(NetStats::default()),
            object_cache_dir: Arc::new(Mutex::new(None)),
            agent_language: Arc::new(Mutex::new(("en".into(), false))),
            bandwidth_kbps: Default::default(),
            viewer_stats: Default::default(),
            viewer_stats_posts: Default::default(),
        }
    }

    /// The agent in `main_addr` (handle 10), `neighbour_addr` (20) beside.
    fn two_regions(sh: &Shared, sock: Arc<UdpSocket>, main_addr: SocketAddr, neighbour_addr: SocketAddr) -> Session<'_> {
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
        Session {
            sh,
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
            refuse_neighbours: false,
            fov: DEFAULT_FOV,
            logout_started: None,
            finished: false,
            agent_moved: false,
            viewer_stats: Default::default(),
            regions_visited: Default::default(),
            cam_requested_at: None,
            agent_parcel: None,
            pending_named_tp: None,
            object_cache: object_cache::ObjectCaches::new(None).0,
            throttle: crate::throttle::Throttle::new(crate::throttle::DEFAULT_KBPS, Instant::now()),
            social: Default::default(),
            tasks: Default::default(),
            land: Default::default(),
            estate_invoice: Uuid::nil(),
            terrain_raw: Default::default(),
            decode_failures: HashMap::new(),
            inventory_uploads: Default::default(),
            asset_imports: Default::default(),
            inventory_previews: Default::default(),
            script_transfers: Default::default(),
            inventory_callbacks: HashMap::new(),
            inventory_copies: inventory_copies::Pending::default(),
            attach_queue: Default::default(),
            inventory_merchant_pending: false,
            next_inventory_callback: 0,
        }
    }

    #[tokio::test]
    async fn rlva_named_region_resolution_emits_a_ticket_without_sending_a_teleport() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = UdpSocket::bind("127.0.0.1:0").await.expect("offline simulator");
        let addr = main.local_addr().unwrap();
        let (events, received) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock.clone(), addr, "127.0.0.1:13002".parse().unwrap());
        let request = Uuid::from_u128(33);
        session.pending_named_tp = Some(("region".into(), request));
        let mut reply = MapBlockReply::default();
        reply.data.push(msgs::map_block_reply::Data {
            x: 1000,
            y: 2000,
            name: str_field("Region"),
            ..Default::default()
        });
        let mut bytes = Vec::new();
        aurora_msg::build_packet(&reply, 1, true, false, &[], &mut bytes);
        sock.writable().await.unwrap();
        session.dispatch(addr, &parse_packet(&bytes).unwrap()).unwrap();
        assert!(received.try_iter().any(|(_, event)| matches!(event, NetEvent::TeleportRegionResolved { request: id, result: Ok(handle) } if id == request && handle == crate::origin_to_handle(256_000, 512_000))));
        assert!(session.pending_named_tp.is_none());
        // Dispatch must not enqueue a TeleportLocationRequest before the
        // viewer can recheck newly received locks or a cancelled destination.
        let mut buf = [0; 4096];
        assert!(
            tokio::time::timeout(Duration::from_millis(30), main.recv_from(&mut buf))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn hypergrid_named_teleport_ignores_the_map_terminator() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        sock.writable().await.expect("viewer socket ready");
        let simulator = UdpSocket::bind("127.0.0.1:0").await.expect("simulator socket");
        let main = simulator.local_addr().expect("simulator address");
        let (events, received) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock, main, "127.0.0.1:9".parse().expect("neighbour"));
        let destination = "gate.example:8002:My Region";
        let position = Vec3::new(12.0, 34.0, 56.0);
        for found in [false, true] {
            let ticket = Uuid::from_u128(100 + u128::from(found));
            session.on_command(NetCommand::TeleportToRegion {
                request: ticket,
                name: destination.into(),
                position,
                look_at: Vec3::X,
            });
            let mut datagram = [0; 4096];
            let (len, _) = tokio::time::timeout(Duration::from_secs(2), simulator.recv_from(&mut datagram))
                .await
                .expect("map request")
                .expect("packet");
            let request: MapNameRequest = parse_packet(&datagram[..len]).expect("datagram").decode().expect("map request");
            assert_eq!(field_str(&request.name_data.name), destination);
            assert_eq!(request.agent_data.flags, 2);
            let mut reply = MapBlockReply::default();
            reply.agent_data.agent_id = session.agent_id();
            reply.agent_data.flags = 2;
            if found {
                reply.data.push(msgs::map_block_reply::Data {
                    name: str_field(destination),
                    x: 100,
                    y: 200,
                    access: 13,
                    ..Default::default()
                });
            }
            reply.data.push(msgs::map_block_reply::Data {
                name: str_field(destination),
                access: 255,
                ..Default::default()
            });
            let mut bytes = Vec::new();
            aurora_msg::build_packet(&reply, 1, true, false, &[], &mut bytes);
            session.dispatch(main, &parse_packet(&bytes).expect("map reply")).expect("dispatch");
            assert!(session.pending_named_tp.is_none());
            let result = received
                .try_iter()
                .find_map(|(_, event)| match event {
                    NetEvent::TeleportRegionResolved { request, result } if request == ticket => Some(result),
                    _ => None,
                })
                .expect("region resolution ticket");
            assert!(
                simulator.try_recv_from(&mut datagram).is_err(),
                "resolution must preserve the viewer's RLVa approval step"
            );
            if found {
                let handle = result.expect("resolved region");
                assert_eq!(handle, ((100 * 256_u64) << 32) | (200 * 256));
                session.on_command(NetCommand::TeleportTo {
                    handle,
                    position,
                    look_at: Vec3::X,
                });
                let (len, _) = tokio::time::timeout(Duration::from_secs(2), simulator.recv_from(&mut datagram))
                    .await
                    .expect("teleport")
                    .expect("packet");
                let teleport: TeleportLocationRequest = parse_packet(&datagram[..len]).expect("packet").decode().expect("teleport request");
                assert_eq!(teleport.info.region_handle, ((100 * 256_u64) << 32) | (200 * 256));
                assert_eq!(teleport.info.position, position);
            } else {
                assert!(
                    simulator.try_recv_from(&mut datagram).is_err(),
                    "no teleport to the nonexistent handle zero"
                );
                assert!(result.is_err());
            }
        }
    }

    #[tokio::test]
    async fn a_lost_event_queue_ends_the_session_only_for_the_agents_region() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main_addr: SocketAddr = "127.0.0.1:13001".parse().expect("address");
        let neighbour_addr: SocketAddr = "127.0.0.1:13002".parse().expect("address");
        let (events, received) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock, main_addr, neighbour_addr);
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
        // a lost link, not a kick: the viewer offers to reconnect
        assert!(matches!(event, NetEvent::Disconnected { lost: true, .. }), "{event:?}");
    }

    /// #750: the first answer to a neighbour's seed lacks the inventory
    /// capabilities; the crossing repeats that seed, which is asked again,
    /// and the larger answer is kept.
    #[tokio::test]
    async fn a_repeated_seed_is_asked_again_and_the_larger_answer_kept() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main_addr: SocketAddr = "127.0.0.1:13001".parse().expect("address");
        let neighbour_addr: SocketAddr = "127.0.0.1:13002".parse().expect("address");
        let (events, received) = crossbeam_channel::unbounded();
        let sh = shared(crate::EventSender(events));
        let mut session = two_regions(&sh, sock, main_addr, neighbour_addr);
        // (nothing answers there: the requests only have to start)
        let seed = "http://127.0.0.1:9/neighbour-seed".to_owned();
        let caps = |names: &[&str]| -> HashMap<String, String> {
            names.iter().map(|n| (n.to_string(), format!("http://127.0.0.1:9/{n}"))).collect()
        };
        let child = caps(&["GetTexture", "ViewerAsset"]);
        let agent = caps(&["GetTexture", "ViewerAsset", "InventoryAPIv3", "FetchInventoryDescendents2"]); // gitleaks:allow -- public capability names
        let caps_events = |received: &crossbeam_channel::Receiver<_>| {
            received
                .try_iter()
                .filter_map(|(_, event)| match event {
                    NetEvent::Capabilities { handle, caps } => Some((handle, caps.len())),
                    NetEvent::Alert { message } => panic!("alert: {message}"),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        // EstablishAgentCommunication: the neighbour's seed, its first answer
        session.set_seed(neighbour_addr, seed.clone());
        assert!(session.sims[&neighbour_addr].caps_requested);
        session.on_caps(neighbour_addr, &seed, Ok(child.clone()));
        assert_eq!(caps_events(&received), [(20, 2)]);

        // the crossing gives the same seed: its capabilities stay meanwhile
        session.switch_region(20, Some(Ipv4Addr::LOCALHOST), neighbour_addr.port(), seed.clone(), (256, 256));
        assert_eq!(session.main, Some(neighbour_addr));
        assert!(session.sims[&neighbour_addr].caps_again_task.is_some());
        assert_eq!(session.main_cap("ViewerAsset").as_deref(), Some("http://127.0.0.1:9/ViewerAsset"));
        assert_eq!(session.main_cap("InventoryAPIv3"), None);

        // the second answer grants more: it takes the place of the first;
        // a smaller or failed one afterwards changes nothing
        session.on_caps(neighbour_addr, &seed, Ok(agent.clone()));
        assert!(session.main_cap("InventoryAPIv3").is_some());
        session.on_caps(neighbour_addr, &seed, Ok(child));
        session.on_caps(neighbour_addr, &seed, Err("status 503".into()));
        assert_eq!(session.sims[&neighbour_addr].caps.len(), 4);
        assert_eq!(caps_events(&received), [(20, 4)]);

        // a new seed drops them until its own answer comes; the answer to
        // the old seed still on its way is ignored
        let new_seed = "http://127.0.0.1:9/new-seed".to_owned();
        session.set_seed(neighbour_addr, new_seed.clone());
        assert!(session.sims[&neighbour_addr].caps.is_empty());
        assert!(session.sims[&neighbour_addr].caps_again_task.is_none());
        session.on_caps(neighbour_addr, &seed, Ok(agent.clone()));
        assert!(session.sims[&neighbour_addr].caps.is_empty());
        session.on_caps(neighbour_addr, &new_seed, Ok(agent));
        assert!(session.main_cap("InventoryAPIv3").is_some());
        assert_eq!(caps_events(&received), [(20, 4)]);
    }
}

#[cfg(test)]
mod script_dialog_tests {
    use super::*;

    fn test_shared(events: crossbeam_channel::Sender<(Instant, NetEvent)>) -> Shared {
        Shared {
            http: reqwest::Client::new(),
            caps_http: reqwest::Client::new(),
            events: crate::EventSender(events),
            controls: Arc::new(Mutex::new(AgentControls::default())),
            stats: Arc::new(NetStats::default()),
            object_cache_dir: Arc::new(Mutex::new(None)),
            agent_language: Arc::new(Mutex::new(("en".into(), false))),
            bandwidth_kbps: Default::default(),
            viewer_stats: Default::default(),
            viewer_stats_posts: Default::default(),
        }
    }

    fn test_login(main_port: u16) -> Arc<LoginResponse> {
        Arc::new(LoginResponse {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            secure_session_id: Uuid::nil(),
            first_name: String::new(),
            last_name: String::new(),
            sim_ip: Ipv4Addr::LOCALHOST,
            sim_port: main_port,
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
        })
    }

    fn test_session<'a>(sh: &'a Shared, sock: Arc<UdpSocket>, sims: HashMap<SocketAddr, Sim>, main: SocketAddr) -> Session<'a> {
        let (eq_tx, _) = mpsc::unbounded_channel();
        let (caps_tx, _) = mpsc::unbounded_channel();
        let (notecard_copy_tx, _) = mpsc::unbounded_channel();
        let (inventory_move_tx, _) = mpsc::unbounded_channel();
        Session {
            sh,
            sock,
            login: test_login(main.port()),
            sims,
            main: Some(main),
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
            refuse_neighbours: false,
            fov: DEFAULT_FOV,
            logout_started: None,
            finished: false,
            agent_moved: false,
            viewer_stats: Default::default(),
            regions_visited: Default::default(),
            cam_requested_at: None,
            agent_parcel: None,
            pending_named_tp: None,
            object_cache: super::object_cache::ObjectCaches::new(None).0,
            throttle: crate::throttle::Throttle::new(crate::throttle::DEFAULT_KBPS, Instant::now()),
            social: Default::default(),
            tasks: Default::default(),
            land: Default::default(),
            estate_invoice: uuid::Uuid::nil(),
            terrain_raw: Default::default(),
            decode_failures: HashMap::new(),
            inventory_uploads: Default::default(),
            asset_imports: Default::default(),
            inventory_previews: Default::default(),
            script_transfers: Default::default(),
            inventory_callbacks: HashMap::new(),
            inventory_copies: super::inventory_copies::Pending::default(),
            attach_queue: Default::default(),
            inventory_merchant_pending: false,
            next_inventory_callback: 0,
        }
    }

    /// The next LogoutRequest the simulator receives, past the other packets
    /// a tick sends (pings, agent updates).
    async fn logout_request_from(sim: &UdpSocket) -> LogoutRequest {
        let mut datagram = [0; 1500];
        loop {
            let (size, _) = tokio::time::timeout(Duration::from_secs(2), sim.recv_from(&mut datagram))
                .await
                .expect("a LogoutRequest reaches the region")
                .expect("packet");
            let packet = parse_packet(&datagram[..size]).expect("datagram");
            if packet.id == LogoutRequest::ID {
                assert!(packet.reliable());
                return packet.decode().expect("LogoutRequest");
            }
        }
    }

    #[tokio::test]
    async fn a_session_ends_with_a_logout_request_answered_or_timed_out() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = UdpSocket::bind("127.0.0.1:0").await.expect("main simulator");
        let main_addr = main.local_addr().expect("main address");
        let (events, received) = crossbeam_channel::unbounded();
        let sh = test_shared(events);
        let one_sim = || [(main_addr, Sim::new(main_addr, 10))].into();
        let ended = |received: &crossbeam_channel::Receiver<(Instant, NetEvent)>| {
            received
                .try_iter()
                .map(|(_, event)| event)
                .find(|event| matches!(event, NetEvent::LoggedOut | NetEvent::Disconnected { .. }))
        };
        let ago = |seconds| Instant::now().checked_sub(Duration::from_secs(seconds)).expect("uptime");
        sock.writable().await.expect("socket ready");

        // Log out: one LogoutRequest, the session ends on the region's reply.
        let mut session = test_session(&sh, sock.clone(), one_sim(), main_addr);
        session.on_command(NetCommand::Logout);
        let request = logout_request_from(&main).await;
        assert_eq!(
            (request.agent_data.agent_id, request.agent_data.session_id),
            (Uuid::from_u128(1), Uuid::from_u128(2))
        );
        assert!(session.logout_started.is_some() && !session.finished);
        session.on_command(NetCommand::Logout);
        assert!(main.try_recv_from(&mut [0; 1500]).is_err(), "asked once");
        let mut reply = LogoutReply::default();
        reply.agent_data.agent_id = Uuid::from_u128(1);
        reply.agent_data.session_id = Uuid::from_u128(2);
        reply.inventory_data.push(Default::default());
        let mut bytes = Vec::new();
        aurora_msg::build_packet(&reply, 1, true, false, &[], &mut bytes);
        session
            .dispatch(main_addr, &parse_packet(&bytes).expect("reply packet"))
            .expect("dispatch reply");
        assert!(session.finished);
        assert!(matches!(ended(&received), Some(NetEvent::LoggedOut)));

        // No reply: the session ends 6 s later all the same (LOGOUT_REQUEST_TIME).
        let mut session = test_session(&sh, sock.clone(), one_sim(), main_addr);
        session.on_command(NetCommand::Logout);
        logout_request_from(&main).await;
        session.tick();
        assert!(!session.finished, "waits for the reply");
        session.logout_started = Some(ago(7));
        session.tick();
        assert!(session.finished);
        assert!(matches!(ended(&received), Some(NetEvent::LoggedOut)));

        // The region never confirms the arrival (a previous session of the
        // account still there, #956): STATE_AGENT_WAIT's time-out logs the
        // new session out before going back to the login screen.
        let mut session = test_session(&sh, sock.clone(), one_sim(), main_addr);
        session.cam_requested_at = Some(ago(89));
        session.tick();
        assert!(!session.finished);
        session.cam_requested_at = Some(ago(91));
        session.tick();
        logout_request_from(&main).await;
        assert!(session.finished);
        assert!(matches!(ended(&received), Some(NetEvent::Disconnected { lost: true, .. })));
    }

    /// A `ViewerStats` capability: the body of every POST it receives.
    async fn stats_capability() -> (String, mpsc::UnboundedReceiver<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let url = format!("http://{}/stats", listener.local_addr().expect("address"));
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = Vec::new();
                let mut chunk = [0; 8192];
                // the headers, then as much body as Content-Length says
                let body = loop {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break None;
                    }
                    request.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&request).into_owned();
                    let Some((head, body)) = text.split_once("\r\n\r\n") else {
                        continue;
                    };
                    let length = head
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_owned()))
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if body.len() >= length {
                        break Some(body.to_owned());
                    }
                };
                // `Connection: close`: `finish` shuts this connection, so the
                // client must not keep it for the next report (a report sent
                // on it before the client saw the end was drained and lost)
                let reply =
                    "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: 13\r\nConnection: close\r\n\r\n<llsd></llsd>";
                let _ = stream.write_all(reply.as_bytes()).await;
                crate::test_http::finish(stream);
                if let Some(body) = body {
                    let _ = tx.send(body);
                }
            }
        });
        (url, rx)
    }

    #[tokio::test]
    async fn viewer_statistics_leave_on_arrival_every_five_minutes_and_at_logout() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = UdpSocket::bind("127.0.0.1:0").await.expect("main simulator");
        let main_addr = main.local_addr().expect("main address");
        let (events, _received) = crossbeam_channel::unbounded();
        let sh = test_shared(events);
        sh.viewer_stats.lock().version = "Aurora Viewer 0.7.5.0".into();
        let (url, mut posted) = stats_capability().await;
        let mut sim = Sim::new(main_addr, 10);
        sim.caps = Arc::new([("ViewerStats".to_string(), url)].into());
        let mut session = test_session(&sh, sock.clone(), [(main_addr, sim)].into(), main_addr);
        async fn next(posted: &mut mpsc::UnboundedReceiver<String>) -> String {
            tokio::time::timeout(Duration::from_secs(5), posted.recv())
                .await
                .expect("a report reaches the capability")
                .expect("body")
        }
        let none = |posted: &mut mpsc::UnboundedReceiver<String>| posted.try_recv().is_err();

        // not before the agent arrived
        session.tick();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(none(&mut posted));

        // at once on arrival, with the session id and the viewer's values
        session.agent_moved = true;
        session.regions_visited.insert(10);
        session.tick();
        let first = next(&mut posted).await;
        assert!(first.contains("Aurora Viewer 0.7.5.0"), "{first}");
        assert!(first.contains(&Uuid::from_u128(2).to_string()), "the session id: {first}");
        assert!(first.contains("<key>regions_visited</key><integer>1</integer>"), "{first}");

        // then not again before the period is over
        session.tick();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(none(&mut posted));
        session.viewer_stats = {
            let mut s = crate::viewer_stats::Schedule::default();
            let ago = Instant::now().checked_sub(crate::viewer_stats::SEND_PERIOD).expect("uptime");
            s.sent(ago, 0, 0.0);
            s
        };
        session.tick();
        next(&mut posted).await;

        // and a last one at logout, before the LogoutRequest
        session.on_command(NetCommand::Logout);
        next(&mut posted).await;
        logout_request_from(&main).await;
        // none while the logout runs
        session.tick();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(none(&mut posted));
    }

    #[tokio::test]
    async fn script_dialog_replies_reach_the_original_simulator_with_exact_wire_fields() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = UdpSocket::bind("127.0.0.1:0").await.expect("main simulator");
        let neighbour = UdpSocket::bind("127.0.0.1:0").await.expect("object simulator");
        let main_addr = main.local_addr().expect("main address");
        let neighbour_addr = neighbour.local_addr().expect("object address");
        let (events, received) = crossbeam_channel::unbounded();
        let sh = test_shared(events);
        let sims = [(main_addr, Sim::new(main_addr, 10)), (neighbour_addr, Sim::new(neighbour_addr, 20))].into();
        let mut session = test_session(&sh, sock.clone(), sims, main_addr);
        sock.writable().await.expect("socket ready");
        let object = Uuid::from_u128(3);
        // Main-region HUD, neighbouring object's menu and a menu answered
        // after crossing all retain the actual packet sender.
        for (origin, current) in [(&main, main_addr), (&neighbour, main_addr), (&main, neighbour_addr)] {
            session.main = Some(current);
            let origin_addr = origin.local_addr().expect("origin");
            for channel in [i32::MIN, -42, 0, 42, i32::MAX] {
                let mut menu = ScriptDialog::default();
                menu.data.object_id = object;
                menu.data.chat_channel = channel;
                menu.data.object_name = str_field("HUD");
                menu.buttons = ["  Keep spaces  ", "✓ Sélection", " "]
                    .into_iter()
                    .map(|label| msgs::script_dialog::Buttons {
                        button_label: str_field(label),
                    })
                    .collect();
                let mut bytes = Vec::new();
                aurora_msg::build_packet(&menu, 1, true, false, &[], &mut bytes);
                session
                    .dispatch(origin_addr, &parse_packet(&bytes).expect("menu packet"))
                    .expect("dispatch menu");
                let NetEvent::ScriptDialog {
                    sender,
                    object_id,
                    channel: received_channel,
                    buttons,
                    ..
                } = received.try_recv().expect("menu event").1
                else {
                    panic!("script menu")
                };
                assert_eq!(sender, origin_addr);
                assert_eq!((object_id, received_channel), (object, channel));
                for (index, label) in buttons.into_iter().enumerate() {
                    session.on_command(NetCommand::ScriptDialogReply {
                        sender: Some(sender),
                        object_id,
                        channel,
                        index: index as i32,
                        label: label.clone(),
                    });
                    let mut datagram = [0; 1200];
                    let (size, _) = tokio::time::timeout(Duration::from_secs(2), origin.recv_from(&mut datagram))
                        .await
                        .expect("reply reaches menu sender")
                        .expect("reply packet");
                    let packet = parse_packet(&datagram[..size]).expect("reply datagram");
                    assert!(packet.reliable());
                    let reply: ScriptDialogReply = packet.decode().expect("dialog reply");
                    assert_eq!(reply.agent_data.agent_id, Uuid::from_u128(1));
                    assert_eq!(reply.agent_data.session_id, Uuid::from_u128(2));
                    assert_eq!(reply.data.object_id, object);
                    assert_eq!(reply.data.chat_channel, channel);
                    assert_eq!(reply.data.button_index, index as i32);
                    assert_eq!(reply.data.button_label, str_field(&label));
                    assert!(session.sims[&sender].circuit.is_unacked(packet.sequence));
                }
                let other = if origin_addr == main_addr { &neighbour } else { &main };
                assert!(other.try_recv_from(&mut [0; 1200]).is_err(), "no reply to the other simulator");
            }
        }
        // Avatar chat on negative channels (bridge discovery) is still
        // intentionally sent through the current simulator.
        session.main = Some(main_addr);
        session.on_command(NetCommand::ScriptDialogReply {
            sender: None,
            object_id: Uuid::from_u128(1),
            channel: -42,
            index: 0,
            label: "probe".into(),
        });
        let mut datagram = [0; 1200];
        tokio::time::timeout(Duration::from_secs(2), main.recv_from(&mut datagram))
            .await
            .expect("bridge probe reaches main")
            .expect("probe packet");
        // A closed source must never redirect an old menu's answer to main.
        session.sims.remove(&neighbour_addr);
        session.on_command(NetCommand::ScriptDialogReply {
            sender: Some(neighbour_addr),
            object_id: object,
            channel: -42,
            index: 0,
            label: "old menu".into(),
        });
        assert!(main.try_recv_from(&mut datagram).is_err());

        // Chat on a negative channel (a gesture's "/-4242 ping", #952) goes
        // as the agent's own dialog answer, as really_send_chat_from_viewer
        // does; a positive channel stays a ChatFromViewer.
        let long = "é".repeat(200);
        for (channel, message) in [(-4242, "ping"), (-1, long.as_str()), (5, "cmd")] {
            session.on_command(NetCommand::Chat {
                message: message.into(),
                channel,
                chat_type: ChatType::Normal,
            });
            let (size, _) = tokio::time::timeout(Duration::from_secs(2), main.recv_from(&mut datagram))
                .await
                .expect("channel chat reaches main")
                .expect("channel chat packet");
            let packet = parse_packet(&datagram[..size]).expect("channel chat datagram");
            assert!(packet.reliable());
            if channel < 0 {
                assert_eq!(packet.id, ScriptDialogReply::ID);
                let reply: ScriptDialogReply = packet.decode().expect("negative channel as a dialog answer");
                assert_eq!(reply.agent_data.agent_id, Uuid::from_u128(1));
                assert_eq!(reply.data.object_id, Uuid::from_u128(1));
                assert_eq!((reply.data.chat_channel, reply.data.button_index), (channel, 0));
                let label = field_str(&reply.data.button_label);
                assert!(label.len() <= 254 && message.starts_with(&label) && !label.is_empty());
                assert_eq!(reply.data.button_label.len(), label.len() + 1, "kept its NUL");
            } else {
                assert_eq!(packet.id, ChatFromViewer::ID);
                let chat: ChatFromViewer = packet.decode().expect("positive channel chat");
                assert_eq!(
                    (chat.chat_data.channel, field_str(&chat.chat_data.message)),
                    (channel, message.to_owned())
                );
            }
        }
    }
}
