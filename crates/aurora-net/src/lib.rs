//! Aurora Viewer network layer: login, UDP circuits, capabilities, event
//! queue and HTTP asset fetching, running on a dedicated tokio runtime.

pub mod build;
pub mod caps;
pub mod circuit;
pub mod fetch;
pub mod inventory;
pub mod land;
pub mod login;
pub mod objcache;
pub mod objects;
pub mod profile;
mod session;
pub mod social;
pub mod stats;
pub mod task_inventory;
pub mod terrain;
pub mod types;
pub mod xmlrpc;

pub use fetch::{FetchRequest, FetchResult, Fetcher};
pub use login::{LoginRequest, LoginResponse, StartLocation};
pub use profile::{AvatarProfile, ClassifiedInfo, ParcelSummary, PickInfo, ProfileGroup};
pub use social::{GroupMembership, HistoryLine, MuteListSource, SessionAgent, SessionInvite, SessionMethod};
pub use stats::{NetStats, NetStatsSnapshot};
pub use types::*;

use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Handle owned by the viewer: send commands, poll events.
pub struct NetClient {
    rt: tokio::runtime::Runtime,
    cmd_tx: mpsc::UnboundedSender<NetCommand>,
    pub events: crossbeam_channel::Receiver<NetEvent>,
    pub controls: Arc<Mutex<AgentControls>>,
    pub stats: Arc<NetStats>,
    pub fetcher: Fetcher,
    pub fetch_results: crossbeam_channel::Receiver<FetchResult>,
    object_cache_dir: Arc<Mutex<Option<std::path::PathBuf>>>,
    caps_http: reqwest::Client,
}

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("http client: {0}")]
    Http(String),
}

impl NetClient {
    pub fn new() -> Result<NetClient, NetError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .thread_name("aurora-net")
            .enable_all()
            .build()?;
        // Login (public host): standard verification, plus Linden Lab's own CA.
        let ll_ca = reqwest::Certificate::from_pem_bundle(include_bytes!("../lindenlab.pem")).unwrap_or_default();
        let http = reqwest::Client::builder()
            .user_agent(concat!("AuroraViewer/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(15))
            .tls_certs_merge(ll_ca.clone())
            .build()
            .map_err(|e| NetError::Http(e.to_string()))?;
        // Capabilities / assets: like LL's llcorehttp (mVerifyHost = false) the
        // certificate chain is verified (public roots + Linden Lab CA) but the
        // simulator hostname is not.
        let mut roots: Vec<reqwest::Certificate> = webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .filter_map(|c| reqwest::Certificate::from_der(c.as_ref()).ok())
            .collect();
        roots.extend(ll_ca);
        let caps_http = reqwest::Client::builder()
            .user_agent(concat!("AuroraViewer/", env!("CARGO_PKG_VERSION")))
            .pool_max_idle_per_host(32)
            .connect_timeout(std::time::Duration::from_secs(15))
            .tls_certs_only(roots)
            .tls_danger_accept_invalid_hostnames(true)
            .build()
            .map_err(|e| NetError::Http(e.to_string()))?;
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<NetCommand>();
        let (ev_tx, ev_rx) = crossbeam_channel::unbounded();
        let controls = Arc::new(Mutex::new(AgentControls::default()));
        let stats = Arc::new(NetStats::default());
        let object_cache_dir = Arc::new(Mutex::new(None));
        let (fetcher, fetch_results) = Fetcher::new(rt.handle(), caps_http.clone(), 24, stats.clone());

        let shared_caps_http = caps_http.clone();
        let shared = session::Shared {
            http,
            caps_http,
            events: ev_tx,
            controls: controls.clone(),
            stats: stats.clone(),
            object_cache_dir: object_cache_dir.clone(),
        };
        rt.spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    NetCommand::Login(req) => {
                        session::run_session(&shared, *req, &mut cmd_rx).await;
                    }
                    other => log::debug!("ignoring command while offline: {other:?}"),
                }
            }
        });

        Ok(NetClient {
            rt,
            cmd_tx,
            events: ev_rx,
            controls,
            stats,
            fetcher,
            fetch_results,
            object_cache_dir,
            caps_http: shared_caps_http,
        })
    }

    /// Directory for the per-region object caches (None disables them);
    /// applies to the next session.
    pub fn set_object_cache_dir(&self, dir: Option<std::path::PathBuf>) {
        *self.object_cache_dir.lock() = dir;
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
