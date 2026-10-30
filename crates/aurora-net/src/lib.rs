//! Aurora Viewer network layer: login, UDP circuits, capabilities, event
//! queue and HTTP asset fetching, running on a dedicated tokio runtime.

pub mod abuse_report;
mod agent_update;
mod attachment_detach;
pub mod bridge;
pub mod build;
pub mod caps;
mod chat_reply;
pub mod circuit;
pub mod currency;
pub mod estate;
pub mod experience;
pub mod fetch;
mod fetch_pace;
pub mod freeze;
pub mod group_money;
pub mod groups;
pub mod home;
mod im_reply;
pub mod inventory;
pub mod land;
pub mod land_tool;
pub mod login;
pub mod lsl_syntax;
pub mod objcache;
pub mod objects;
pub mod offline_im;
pub mod outfits;
pub mod pathfinding;
pub mod postcard;
pub mod probe;
pub mod profile;
pub mod region_restart;
pub mod release_notes;
pub mod script_controls;
pub mod scripts;
pub mod search;
mod session;
pub mod sim_console;
pub mod slm;
pub mod social;
pub mod stats;
pub mod task_inventory;
pub mod terrain;
#[cfg(test)]
mod test_http;
pub mod throttle;
pub mod transition;
pub mod types;
pub mod user_info;
pub mod viewer_stats;
pub mod voice_moderation;
pub mod xmlrpc;

pub use fetch::{FetchRequest, FetchResult, Fetcher, Lane};
pub use login::{LoginRequest, LoginResponse, StartLocation};
pub use profile::{AvatarProfile, ClassifiedInfo, ParcelSummary, PickInfo, ProfileGroup};
pub use social::{CallMethod, GroupMembership, HistoryLine, MuteListSource, SessionAgent, SessionInvite, SessionMethod, VoiceInvite};
pub use stats::{NetStats, NetStatsSnapshot};
pub use types::*;

use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::mpsc;

/// The session's side of the event channel: every event leaves with the
/// time it was sent, so the viewer can tell how long it waited.
#[derive(Clone)]
pub(crate) struct EventSender(crossbeam_channel::Sender<(std::time::Instant, NetEvent)>);

impl EventSender {
    /// False once the viewer is gone.
    pub(crate) fn send(&self, ev: NetEvent) -> bool {
        self.0.send((std::time::Instant::now(), ev)).is_ok()
    }

    /// Events the viewer has not taken yet.
    pub(crate) fn waiting(&self) -> usize {
        self.0.len()
    }
}

/// Handle owned by the viewer: send commands, poll events.
pub struct NetClient {
    rt: tokio::runtime::Runtime,
    cmd_tx: mpsc::UnboundedSender<NetCommand>,
    /// Events of the session, each with the time the network side sent it.
    pub events: crossbeam_channel::Receiver<(std::time::Instant, NetEvent)>,
    pub controls: Arc<Mutex<AgentControls>>,
    /// The controls last handed to the session as a command.
    pushed_controls: Mutex<AgentControls>,
    pub stats: Arc<NetStats>,
    pub fetcher: Fetcher,
    pub fetch_results: crossbeam_channel::Receiver<FetchResult>,
    object_cache_dir: Arc<Mutex<Option<std::path::PathBuf>>>,
    agent_language: Arc<Mutex<(String, bool)>>,
    /// Bandwidth setting, in kb/s (throttle.rs).
    bandwidth_kbps: Arc<std::sync::atomic::AtomicU32>,
    caps_http: reqwest::Client,
    /// The viewer's part of Linden Lab's viewer statistics (viewer_stats.rs).
    viewer_stats: Arc<Mutex<viewer_stats::ViewerSide>>,
    viewer_stats_posts: Arc<viewer_stats::PostsInFlight>,
}

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("http client: {0}")]
    Http(String),
}

/// The certificates a simulator's capability may be signed by: the public
/// roots and Linden Lab's own CA (the simulator hostname is not checked, see
/// NetClient::new).
pub(crate) fn simulator_roots() -> Vec<reqwest::Certificate> {
    let mut roots: Vec<reqwest::Certificate> = webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|c| reqwest::Certificate::from_der(c.as_ref()).ok())
        .collect();
    roots.extend(reqwest::Certificate::from_pem_bundle(include_bytes!("../lindenlab.pem")).unwrap_or_default());
    roots
}

/// How many asset downloads may be under way at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetConnections {
    /// Follow the link (`fetch_pace.rs`): from 24 up to 192 while it keeps
    /// up. Aurora's default.
    Adaptive,
    /// A fixed 24 requests and at most 32 kept connections per host, like
    /// the Legacy Viewer's bounded llcorehttp policy
    /// (`LLAppCoreHttp::init_data`, llappcorehttp.cpp). Some network
    /// adapter drivers lose the link until a reboot under the adaptive
    /// pace's connections (#1545, #1546).
    Limited,
}

/// Requests under way at once in the limited mode.
pub const LIMITED_FETCH_SLOTS: usize = 24;
/// Idle connections kept per host in the limited mode.
const LIMITED_IDLE_PER_HOST: usize = 32;

/// `AURORA_FETCH_SLOTS=<n>` (1 to 256): the requests under way at once
/// fixed for one launch, to compare, whatever the mode.
fn fetch_slots_override() -> Option<usize> {
    match std::env::var("AURORA_FETCH_SLOTS").ok().map(|v| v.trim().parse::<usize>()) {
        Some(Ok(n)) if (1..=256).contains(&n) => {
            log::info!("asset fetch: {n} requests at once (AURORA_FETCH_SLOTS)");
            Some(n)
        }
        Some(_) => {
            log::warn!("AURORA_FETCH_SLOTS: expected 1 to 256, ignored");
            None
        }
        None => None,
    }
}

/// The requests under way at once (None: they follow the link) and the
/// idle connections kept per host, for a mode and an `AURORA_FETCH_SLOTS`
/// override.
fn asset_limits(mode: AssetConnections, fixed: Option<usize>) -> (Option<usize>, usize) {
    match (fixed, mode) {
        (Some(n), _) => (Some(n), n.max(32)),
        // (connections kept for every request the pace may have)
        (None, AssetConnections::Adaptive) => (None, fetch_pace::MAX.max(32)),
        (None, AssetConnections::Limited) => (Some(LIMITED_FETCH_SLOTS), LIMITED_IDLE_PER_HOST),
    }
}

impl NetClient {
    pub fn new(asset_connections: AssetConnections) -> Result<NetClient, NetError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .thread_name("aurora-net")
            .enable_all()
            .build()?;
        // Login (public host): standard verification, plus Linden Lab's own CA.
        let ll_ca = reqwest::Certificate::from_pem_bundle(include_bytes!("../lindenlab.pem")).unwrap_or_default();
        let http = reqwest::Client::builder()
            .user_agent(aurora_version::user_agent())
            .connect_timeout(std::time::Duration::from_secs(15))
            .tls_certs_merge(ll_ca)
            .build()
            .map_err(|e| NetError::Http(e.to_string()))?;
        // Capabilities / assets: like LL's llcorehttp (mVerifyHost = false) the
        // certificate chain is verified (public roots + Linden Lab CA) but the
        // simulator hostname is not.
        let roots = simulator_roots();
        let caps_http = reqwest::Client::builder()
            .user_agent(aurora_version::user_agent())
            .pool_max_idle_per_host(32)
            .connect_timeout(std::time::Duration::from_secs(15))
            .tls_certs_only(roots.clone())
            .tls_danger_accept_invalid_hostnames(true)
            .build()
            .map_err(|e| NetError::Http(e.to_string()))?;
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<NetCommand>();
        let (ev_tx, ev_rx) = crossbeam_channel::unbounded();
        let controls = Arc::new(Mutex::new(AgentControls::default()));
        let stats = Arc::new(NetStats::default());
        let object_cache_dir = Arc::new(Mutex::new(None));
        let bandwidth_kbps = Arc::new(std::sync::atomic::AtomicU32::new(throttle::DEFAULT_KBPS as u32));
        let agent_language = Arc::new(Mutex::new((aurora_i18n::language().to_owned(), true)));
        let viewer_stats = Arc::new(Mutex::new(viewer_stats::ViewerSide::default()));
        let viewer_stats_posts = Arc::new(viewer_stats::PostsInFlight::default());
        // Asset GETs (ViewerAsset, bake service): same trust as the caps,
        // but like Firestorm's llcorehttp GET (curl without
        // CURLOPT_ENCODING): no Accept-Encoding and no transparent
        // decoding, a 30 s connect timeout and up to 10 redirects. Some
        // assets are served with an odd Content-Encoding that must not
        // make the fetch fail.
        let (fetch_slots, idle_per_host) = asset_limits(asset_connections, fetch_slots_override());
        match fetch_slots {
            None => log::info!("asset fetch: adaptive (24 to {} requests at once)", fetch_pace::MAX),
            Some(n) => log::info!("asset fetch: limited to {n} requests at once, {idle_per_host} kept connections per host"),
        }
        let asset_http = reqwest::Client::builder()
            .user_agent(aurora_version::user_agent())
            .pool_max_idle_per_host(idle_per_host)
            .connect_timeout(std::time::Duration::from_secs(30))
            .gzip(false)
            .redirect(reqwest::redirect::Policy::limited(10))
            .tls_certs_only(roots)
            .tls_danger_accept_invalid_hostnames(true)
            .build()
            .map_err(|e| NetError::Http(e.to_string()))?;
        let (fetcher, fetch_results) = Fetcher::paced(rt.handle(), asset_http, fetch_slots, stats.clone());

        let shared_caps_http = caps_http.clone();
        let shared = session::Shared {
            http,
            caps_http,
            events: EventSender(ev_tx),
            controls: controls.clone(),
            stats: stats.clone(),
            object_cache_dir: object_cache_dir.clone(),
            agent_language: agent_language.clone(),
            bandwidth_kbps: bandwidth_kbps.clone(),
            viewer_stats: viewer_stats.clone(),
            viewer_stats_posts: viewer_stats_posts.clone(),
        };
        rt.spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    NetCommand::Login(req) => {
                        session::run_session(&shared, *req, &mut cmd_rx).await;
                    }
                    // (the offline demo moves its agent every frame)
                    NetCommand::AgentControls(_) => {}
                    other => log::debug!("ignoring command while offline: {other:?}"),
                }
            }
        });

        Ok(NetClient {
            rt,
            cmd_tx,
            events: ev_rx,
            controls,
            pushed_controls: Mutex::new(AgentControls::default()),
            stats,
            fetcher,
            fetch_results,
            object_cache_dir,
            bandwidth_kbps,
            agent_language,
            caps_http: shared_caps_http,
            viewer_stats,
            viewer_stats_posts,
        })
    }

    /// Before the process ends: wait, `timeout` at most, for the viewer
    /// statistics reports still being posted (the last one leaves with the
    /// logout). Returns at once when none is; true when they all finished.
    pub fn wait_viewer_stats(&self, timeout: std::time::Duration) -> bool {
        self.viewer_stats_posts.wait(timeout)
    }

    /// The viewer's part of the next viewer statistics report, refreshed
    /// about once a second; the session posts it on its own schedule
    /// (viewer_stats.rs).
    pub fn set_viewer_stats(&self, side: viewer_stats::ViewerSide) {
        *self.viewer_stats.lock() = side;
    }

    /// Directory for the per-region object caches (None disables them);
    /// applies to the next session.
    pub fn set_object_cache_dir(&self, dir: Option<std::path::PathBuf>) {
        *self.object_cache_dir.lock() = dir;
    }

    /// The most the simulators may send, all channels together, in kb/s
    /// (Firestorm's `ThrottleBandwidthKBPS`); a session under way asks its
    /// simulator for it within a tick.
    pub fn set_bandwidth(&self, kbps: f32) {
        let kbps = kbps.clamp(*throttle::SETTING_KBPS.start(), *throttle::SETTING_KBPS.end());
        self.bandwidth_kbps.store(kbps as u32, std::sync::atomic::Ordering::Relaxed);
    }

    /// LLAgentLanguage::update: keep the choice offline and refresh the sim.
    pub fn set_agent_language(&self, language: &str, public: bool) {
        let next = (language.to_owned(), public);
        let mut saved = self.agent_language.lock();
        if *saved != next {
            *saved = next;
            drop(saved);
            self.send(NetCommand::UpdateAgentLanguage);
        }
    }

    /// Controls of this frame, sent by the session with its next AgentUpdate.
    /// A change of their flags (a key pressed or released) is timed until
    /// that update leaves (`NetStats::take_peaks`).
    ///
    /// The session reads them on its own tick, which is enough for the
    /// keep-alive; what must leave sooner is handed to it right away: every
    /// change of the flags, in order, and whatever moved past the thresholds
    /// of `send_agent_update` since the last hand-over (the session then
    /// applies its limit of 125 updates a second).
    pub fn set_controls(&self, c: AgentControls) {
        let flags_changed = {
            let mut controls = self.controls.lock();
            let changed = c.control_flags != controls.control_flags;
            self.stats.note_controls(changed);
            *controls = c;
            changed
        };
        let mut pushed = self.pushed_controls.lock();
        if flags_changed || c.update_worthy(&pushed) {
            *pushed = c;
            self.send(NetCommand::AgentControls(c));
        }
    }

    pub fn send(&self, cmd: NetCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn runtime(&self) -> &tokio::runtime::Handle {
        self.rt.handle()
    }

    /// HTTP client for capabilities (voice provisioning uses it too).
    pub fn caps_http(&self) -> reqwest::Client {
        self.caps_http.clone()
    }
}

#[cfg(test)]
mod asset_limit_tests {
    use super::*;

    #[test]
    fn adaptive_follows_the_link_and_limited_is_bounded() {
        assert_eq!(asset_limits(AssetConnections::Adaptive, None), (None, fetch_pace::MAX));
        assert_eq!(asset_limits(AssetConnections::Limited, None), (Some(24), 32));
        // the comparison override wins in both modes
        assert_eq!(asset_limits(AssetConnections::Adaptive, Some(64)), (Some(64), 64));
        assert_eq!(asset_limits(AssetConnections::Limited, Some(8)), (Some(8), 32));
    }
}
