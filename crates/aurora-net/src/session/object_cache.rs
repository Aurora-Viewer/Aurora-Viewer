//! The session's side of the per-region object caches (objcache.rs): which
//! simulator has which cache, and their files kept off the session loop.
//!
//! The loop serves every circuit: while it reads or writes a file, nothing is
//! received, acknowledged or answered, and what the simulators send piles up
//! in the socket. A cache file is therefore read by a blocking task, and
//! RegionHandshakeReply leaves once it is loaded, since its "cache empty" bit
//! tells the simulator whether to send cache probes. The caches are written
//! by one writer, in the order their snapshots were taken.
//!
//! Firestorm reads the file in its RegionHandshake handler, on the main
//! thread (LLViewerRegion::unpackRegionHandshake → loadObjectCache), and
//! sends the same reply right after.

use super::Session;
use crate::objcache::{RegionObjectCache, Snapshot};
use aurora_msg::msgs::RegionHandshakeReply;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Changed caches are written this often, so that a crash loses little.
const SAVE_PERIOD: Duration = Duration::from_secs(120);

/// REGION_HANDSHAKE_SUPPORTS_SELF_APPEARANCE (llviewerregion.cpp).
const SUPPORTS_SELF_APPEARANCE: u32 = 0x4;
/// "Send all cacheable objects" (`sVOCacheCullingEnabled`).
const SEND_ALL_CACHEABLE: u32 = 0x1;
/// "The cache file is empty, no need to send cache probes."
const CACHE_EMPTY: u32 = 0x2;

/// A cache file that was read, on its way back to the session.
pub(super) struct Loaded {
    sim: SocketAddr,
    region_id: Uuid,
    /// `ObjectCaches::cleared` when the read started.
    cleared: u64,
    cache: RegionObjectCache,
    took: Duration,
}

struct Writer {
    queue: mpsc::UnboundedSender<(u64, Snapshot)>,
    task: JoinHandle<()>,
}

pub(super) struct ObjectCaches {
    /// Directory of the files (None = no cache).
    dir: Option<PathBuf>,
    caches: HashMap<SocketAddr, RegionObjectCache>,
    /// Simulators whose file is being read, with the region it is for: their
    /// RegionHandshakeReply waits for it.
    loading: HashMap<SocketAddr, Uuid>,
    loaded: mpsc::UnboundedSender<Loaded>,
    /// Started with the first snapshot.
    writer: Option<Writer>,
    last_save: Instant,
    /// How many times the user cleared the cache: what was read or snapshot
    /// before is not wanted any more.
    cleared: Arc<AtomicU64>,
}

impl ObjectCaches {
    /// The receiver yields the files that were read: each goes to
    /// `Session::on_object_cache_loaded`.
    pub fn new(dir: Option<PathBuf>) -> (ObjectCaches, mpsc::UnboundedReceiver<Loaded>) {
        let (loaded, files) = mpsc::unbounded_channel();
        let caches = ObjectCaches {
            dir,
            caches: HashMap::new(),
            loading: HashMap::new(),
            loaded,
            writer: None,
            last_save: Instant::now(),
            cleared: Arc::default(),
        };
        (caches, files)
    }

    pub fn get(&self, sim: SocketAddr) -> Option<&RegionObjectCache> {
        self.caches.get(&sim)
    }

    pub fn get_mut(&mut self, sim: SocketAddr) -> Option<&mut RegionObjectCache> {
        self.caches.get_mut(&sim)
    }

    /// Has the file of a simulator's region read, unless its cache is there
    /// already. True when the reply to the handshake has to wait for it.
    fn load(&mut self, sim: SocketAddr, region_id: Uuid) -> bool {
        let Some(dir) = self.dir.clone() else {
            return false;
        };
        if region_id.is_nil() || self.caches.contains_key(&sim) {
            return false;
        }
        // a handshake sent again while the file is being read
        if self.loading.insert(sim, region_id) == Some(region_id) {
            return true;
        }
        let loaded = self.loaded.clone();
        let cleared = self.cleared.load(Ordering::Relaxed);
        tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            let cache = RegionObjectCache::load(&dir, region_id);
            let _ = loaded.send(Loaded {
                sim,
                region_id,
                cleared,
                cache,
                took: start.elapsed(),
            });
        });
        true
    }

    /// Takes a file that was read. False when nobody waits for it any more:
    /// the simulator is gone, or it now stands for another region.
    fn accept(&mut self, file: Loaded) -> bool {
        if self.loading.get(&file.sim) != Some(&file.region_id) {
            return false;
        }
        self.loading.remove(&file.sim);
        let cache = match &self.dir {
            // the cache was cleared while the file was being read
            Some(dir) if file.cleared != self.cleared.load(Ordering::Relaxed) => RegionObjectCache::empty(dir, file.region_id),
            _ => file.cache,
        };
        self.caches.insert(file.sim, cache);
        true
    }

    /// The simulator is gone: its cache is written and dropped.
    pub fn forget(&mut self, sim: SocketAddr) {
        self.loading.remove(&sim);
        if let Some(mut cache) = self.caches.remove(&sim) {
            let snapshot = cache.snapshot();
            self.write(snapshot);
        }
    }

    /// Writes the caches that changed, every `SAVE_PERIOD`.
    pub fn save_when_due(&mut self, now: Instant) {
        if now.duration_since(self.last_save) > SAVE_PERIOD {
            self.last_save = now;
            self.save();
        }
    }

    pub fn save(&mut self) {
        let snapshots: Vec<_> = self.caches.values_mut().filter_map(RegionObjectCache::snapshot).collect();
        for snapshot in snapshots {
            self.write(Some(snapshot));
        }
    }

    /// The user cleared the cache on disk: nothing held, being read or
    /// waiting for the writer goes back to it.
    pub fn clear(&mut self) {
        self.cleared.fetch_add(1, Ordering::Relaxed);
        for cache in self.caches.values_mut() {
            cache.reset();
        }
    }

    fn write(&mut self, snapshot: Option<Snapshot>) {
        let Some(snapshot) = snapshot else {
            return;
        };
        let cleared = &self.cleared;
        let writer = self.writer.get_or_insert_with(|| {
            let (queue, snapshots) = mpsc::unbounded_channel();
            let cleared = cleared.clone();
            Writer {
                queue,
                task: tokio::task::spawn_blocking(move || write_all(snapshots, &cleared)),
            }
        });
        let _ = writer.queue.send((cleared.load(Ordering::Relaxed), snapshot));
    }

    /// Waits until everything handed to the writer is on disk (end of the
    /// session: the next one reads these files).
    pub async fn flush(&mut self) {
        if let Some(Writer { queue, task }) = self.writer.take() {
            drop(queue);
            let _ = task.await;
        }
    }
}

/// The writer: one file at a time, in the order of the snapshots, so that
/// two of the same region never write over each other and the latest wins.
fn write_all(mut snapshots: mpsc::UnboundedReceiver<(u64, Snapshot)>, cleared: &AtomicU64) {
    while let Some((taken, snapshot)) = snapshots.blocking_recv() {
        if taken != cleared.load(Ordering::Relaxed) {
            continue;
        }
        let start = Instant::now();
        match snapshot.write() {
            Ok(bytes) => log::info!(
                "object cache written: {} objects, {} KiB in {:.1} ms",
                snapshot.len(),
                bytes / 1024,
                start.elapsed().as_secs_f32() * 1000.0
            ),
            Err(e) => log::warn!("object cache not written ({} objects): {e}", snapshot.len()),
        }
    }
}

impl Session<'_> {
    /// Answers a RegionHandshake: at once when the cache of the region is
    /// known (or there is none), else when its file has been read.
    pub(super) fn answer_region_handshake(&mut self, from: SocketAddr, region_id: Uuid) {
        if !self.object_cache.load(from, region_id) {
            self.send_region_handshake_reply(from);
        }
    }

    pub(super) fn on_object_cache_loaded(&mut self, file: Loaded) {
        let (sim, took) = (file.sim, file.took);
        if !self.object_cache.accept(file) {
            return;
        }
        log::info!(
            "object cache for {}: {} entries, read in {:.1} ms",
            self.sims.get(&sim).map_or("", |s| s.name.as_str()),
            self.object_cache.get(sim).map_or(0, RegionObjectCache::len),
            took.as_secs_f32() * 1000.0
        );
        self.send_region_handshake_reply(sim);
    }

    /// The reply that lets the simulator start sending
    /// (LLViewerRegion::unpackRegionHandshake).
    fn send_region_handshake_reply(&mut self, to: SocketAddr) {
        let mut reply = RegionHandshakeReply::default();
        reply.agent_data.agent_id = self.agent_id();
        reply.agent_data.session_id = self.session_id();
        let cache_empty = self.object_cache.get(to).is_none_or(RegionObjectCache::is_empty);
        reply.region_info.flags = SUPPORTS_SELF_APPEARANCE | SEND_ALL_CACHEABLE | if cache_empty { CACHE_EMPTY } else { 0 };
        self.send(to, &reply, true);
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("aurora-objc-{}", Uuid::new_v4()))
    }

    /// A cache file of one object for this region.
    fn write_file(dir: &std::path::Path, region_id: Uuid) {
        let mut cache = RegionObjectCache::empty(dir, region_id);
        cache.put(7, 1, 0, &[1, 2, 3]);
        cache.snapshot().expect("changed").write().expect("written");
    }

    async fn next(files: &mut mpsc::UnboundedReceiver<Loaded>) -> Loaded {
        tokio::time::timeout(Duration::from_secs(5), files.recv())
            .await
            .expect("file read in time")
            .expect("file")
    }

    #[tokio::test]
    async fn the_handshake_is_answered_once_the_cache_is_read() {
        let dir = temp_dir();
        let (cached, fresh) = (Uuid::new_v4(), Uuid::new_v4());
        write_file(&dir, cached);

        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let simulators = [
            UdpSocket::bind("127.0.0.1:0").await.expect("simulator"),
            UdpSocket::bind("127.0.0.1:0").await.expect("simulator"),
            UdpSocket::bind("127.0.0.1:0").await.expect("simulator"),
        ];
        let addrs = simulators.each_ref().map(|s| s.local_addr().expect("address"));
        let (events, _received) = crossbeam_channel::unbounded();
        let sh = Shared {
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
        };
        let login = Arc::new(LoginResponse {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            secure_session_id: Uuid::nil(),
            first_name: String::new(),
            last_name: String::new(),
            sim_ip: Ipv4Addr::LOCALHOST,
            sim_port: addrs[0].port(),
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
        let (object_cache, mut files) = ObjectCaches::new(Some(dir.clone()));
        let mut session = Session {
            sh: &sh,
            sock,
            login,
            sims: addrs.map(|addr| (addr, Sim::new(addr, 10))).into(),
            main: Some(addrs[0]),
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
            object_cache,
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
            inventory_copies: inventory_copies::Pending::default(),
            attach_queue: Default::default(),
            inventory_merchant_pending: false,
            next_inventory_callback: 0,
        };
        let waiting = |session: &Session, sim: usize| session.sims[&addrs[sim]].circuit.unacked_count();
        let handshake = |region_id: Uuid| {
            let mut m = RegionHandshake::default();
            m.region_info2.region_id = region_id;
            m
        };
        async fn reply_flags(simulator: &UdpSocket) -> u32 {
            let mut datagram = [0; 1200];
            let (size, _) = tokio::time::timeout(Duration::from_secs(2), simulator.recv_from(&mut datagram))
                .await
                .expect("reply in time")
                .expect("reply");
            let packet = parse_packet(&datagram[..size]).expect("datagram");
            assert_eq!(packet.id, RegionHandshakeReply::ID);
            let reply: RegionHandshakeReply = packet.decode().expect("reply");
            reply.region_info.flags
        }

        // a region with a cache file: no reply before the file is read, and
        // the simulator sending its handshake again does not read it twice
        session.on_region_handshake(addrs[0], handshake(cached));
        session.on_region_handshake(addrs[0], handshake(cached));
        assert_eq!(waiting(&session, 0), 0, "the reply waits for the cache");
        session.on_object_cache_loaded(next(&mut files).await);
        assert_eq!(waiting(&session, 0), 1);
        assert_eq!(reply_flags(&simulators[0]).await, SUPPORTS_SELF_APPEARANCE | SEND_ALL_CACHEABLE);
        assert!(session.object_cache.get(addrs[0]).is_some_and(|c| c.get(7, 1).is_some()));
        assert!(files.try_recv().is_err(), "one read for two handshakes");
        // a later handshake of a region whose cache is loaded: answered at once
        session.on_region_handshake(addrs[0], handshake(cached));
        assert_eq!(waiting(&session, 0), 2);

        // a region seen for the first time: "cache empty"
        session.on_region_handshake(addrs[1], handshake(fresh));
        assert_eq!(waiting(&session, 1), 0);
        session.on_object_cache_loaded(next(&mut files).await);
        assert_eq!(
            reply_flags(&simulators[1]).await,
            SUPPORTS_SELF_APPEARANCE | SEND_ALL_CACHEABLE | CACHE_EMPTY
        );

        // no region id: nothing to read, answered at once
        session.on_region_handshake(addrs[2], handshake(Uuid::nil()));
        assert_eq!(waiting(&session, 2), 1);
        assert_eq!(
            reply_flags(&simulators[2]).await,
            SUPPORTS_SELF_APPEARANCE | SEND_ALL_CACHEABLE | CACHE_EMPTY
        );

        // a simulator that left while its file was being read gets nothing
        session.remove_sim(addrs[1]);
        session.add_sim(addrs[1], 10, (256, 256), false);
        session.on_region_handshake(addrs[1], handshake(cached));
        session.remove_sim(addrs[1]);
        session.on_object_cache_loaded(next(&mut files).await);
        assert!(session.object_cache.get(addrs[1]).is_none());

        session.object_cache.flush().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn caches_are_written_off_the_loop_and_in_order() {
        let dir = temp_dir();
        let region = Uuid::new_v4();
        let sim: SocketAddr = "127.0.0.1:13001".parse().expect("address");
        let (mut caches, mut files) = ObjectCaches::new(Some(dir.clone()));
        assert!(caches.load(sim, region));
        let file = next(&mut files).await;
        assert!(caches.accept(file));

        // a periodic write and the one of the simulator leaving, back to
        // back: the file holds the later content
        caches.get_mut(sim).expect("cache").put(1, 10, 0, &[1]);
        caches.save_when_due(Instant::now());
        assert!(caches.writer.is_none(), "not due yet");
        caches.save_when_due(Instant::now() + SAVE_PERIOD + Duration::from_secs(1));
        caches.get_mut(sim).expect("cache").put(2, 20, 0, &[2]);
        caches.forget(sim);
        assert!(caches.get(sim).is_none());
        caches.flush().await;
        let read = RegionObjectCache::load(&dir, region);
        assert_eq!(read.len(), 2);

        // back in the region: the session reads what the previous visit wrote
        assert!(caches.load(sim, region));
        let file = next(&mut files).await;
        assert!(caches.accept(file));
        assert_eq!(caches.get(sim).map(RegionObjectCache::len), Some(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_cleared_cache_does_not_come_back() {
        let dir = temp_dir();
        let (region, other) = (Uuid::new_v4(), Uuid::new_v4());
        write_file(&dir, region);
        let sim: SocketAddr = "127.0.0.1:13001".parse().expect("address");
        let neighbour: SocketAddr = "127.0.0.1:13002".parse().expect("address");
        let (mut caches, mut files) = ObjectCaches::new(Some(dir.clone()));

        // cleared while the file was being read: the cache starts empty
        assert!(caches.load(sim, region));
        caches.clear();
        let file = next(&mut files).await;
        assert!(caches.accept(file));
        assert_eq!(caches.get(sim).map(RegionObjectCache::len), Some(0));

        // cleared while a snapshot waited for the writer: it is not written
        assert!(caches.load(neighbour, other));
        let file = next(&mut files).await;
        assert!(caches.accept(file));
        caches.get_mut(neighbour).expect("cache").put(1, 10, 0, &[1]);
        let snapshot = caches.get_mut(neighbour).expect("cache").snapshot();
        caches.clear();
        let stale = caches.cleared.load(Ordering::Relaxed) - 1;
        let (queue, snapshots) = mpsc::unbounded_channel();
        let _ = queue.send((stale, snapshot.expect("changed")));
        drop(queue);
        let cleared = caches.cleared.clone();
        tokio::task::spawn_blocking(move || write_all(snapshots, &cleared))
            .await
            .expect("writer");
        assert!(!dir.join(format!("{other}.objc")).exists());
        assert_eq!(caches.get(neighbour).map(RegionObjectCache::len), Some(0));

        // what arrives after the clearing is cached again
        caches.get_mut(neighbour).expect("cache").put(2, 20, 0, &[2]);
        caches.save();
        caches.flush().await;
        assert_eq!(RegionObjectCache::load(&dir, other).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
