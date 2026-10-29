//! Aurora's own media plugins (SLPlugin, CEF, LibVLC): a pack downloaded once
//! into the user's data folder when no plugins are found, then used by every
//! session. The state is owned by the UI thread; HTTP, hashing and extraction
//! run on workers (`aurora_updater::media_pack`).

use aurora_updater::{
    Cancel, Failure, Progress,
    feed::Source,
    media_pack::{self, PACK},
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default)]
pub enum PackState {
    /// Not installed and not asked for: other plugins are used.
    #[default]
    Idle,
    Downloading(Progress),
    Ready,
    Failed(Failure),
}

enum Event {
    Progress(Progress),
    Done(Result<PathBuf, Failure>),
}

pub struct PluginPack {
    root: PathBuf,
    dir: Option<PathBuf>,
    state: PackState,
    source: Source,
    tx: crossbeam_channel::Sender<Event>,
    rx: crossbeam_channel::Receiver<Event>,
    cancel: Cancel,
    /// Failures in a row, and when the download starts again by itself.
    failures: u32,
    retry_at: Option<Instant>,
    /// False for a state that is only shown (demo, tests): no download.
    automatic: bool,
}

/// Another viewer is installing the pack: it will be there shortly.
const BUSY_RETRY: Duration = Duration::from_secs(5);

/// 30 s, 1 min, 2 min… up to 15 min: the network may come back.
fn retry_delay(failure: Failure, failures: u32) -> Option<Duration> {
    match failure {
        Failure::Busy => Some(BUSY_RETRY),
        // this viewer names no published pack: asking again changes nothing
        Failure::Unsupported => None,
        _ => Some(Duration::from_secs((30_u64 << failures.saturating_sub(1).min(5)).min(900))),
    }
}

impl PluginPack {
    pub fn new(root: PathBuf) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let dir = media_pack::installed(&root, &PACK);
        let mut this = Self {
            state: if dir.is_some() { PackState::Ready } else { PackState::Idle },
            root,
            dir,
            source: Source::default(),
            tx,
            rx,
            cancel: Cancel::default(),
            failures: 0,
            retry_at: None,
            // the isolated update tests have their own local feed, and unit
            // tests no network at all
            automatic: !cfg!(test) && aurora_updater::test_environment().is_none(),
        };
        // No pack is built for macOS yet (TASKS.md): the Windows one would
        // download and never start. Plugins of a chosen folder still work.
        if cfg!(target_os = "macos") {
            this.automatic = false;
            this.dir = None;
            this.state = PackState::Failed(Failure::Unsupported);
        }
        // AURORA_MEDIA_PACK_FEED: a pack served by a test server of this PC
        // (loopback only), to try it before it is published. Its size and
        // SHA-256 are still those this viewer was built for.
        if let Ok(feed) = std::env::var("AURORA_MEDIA_PACK_FEED") {
            match Source::local(&feed) {
                Ok(source) => this.source = source,
                Err(e) => log::warn!("media: AURORA_MEDIA_PACK_FEED ignored: {e:#}"),
            }
        }
        // AURORA_DEMO_MEDIA_PACK: a fixed state for the screenshots
        if std::env::var_os("AURORA_DEMO").is_some()
            && let Ok(mode) = std::env::var("AURORA_DEMO_MEDIA_PACK")
        {
            this.automatic = false;
            this.dir = None;
            this.state = match mode.as_str() {
                "failed" => PackState::Failed(Failure::Network),
                "installing" => PackState::Downloading(Progress {
                    received: PACK.size,
                    total: PACK.size,
                    preparing: true,
                }),
                _ => PackState::Downloading(Progress {
                    received: PACK.size / 20 * 9,
                    total: PACK.size,
                    preparing: false,
                }),
            };
        }
        this
    }

    /// The folder that holds `llplugin\`, once the pack is installed.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn state(&self) -> PackState {
        self.state
    }

    /// Once per frame. `wanted`: no plugins were found (the web pages of the
    /// profiles and of the search need them even with media disabled). True
    /// when the pack has just been installed.
    pub fn poll(&mut self, wanted: bool, runtime: &tokio::runtime::Handle) -> bool {
        let mut installed = false;
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Progress(p) => {
                    if matches!(self.state, PackState::Downloading(_)) {
                        self.state = PackState::Downloading(p);
                    }
                }
                Event::Done(Ok(dir)) => {
                    log::info!("media: plugins pack {} installed in {}", PACK.version, dir.display());
                    self.dir = Some(dir);
                    self.state = PackState::Ready;
                    self.failures = 0;
                    self.retry_at = None;
                    installed = true;
                }
                Event::Done(Err(failure)) => {
                    self.failures += 1;
                    self.retry_at = retry_delay(failure, self.failures).map(|d| Instant::now() + d);
                    self.state = PackState::Failed(failure);
                }
            }
        }
        if wanted && self.automatic {
            match self.state {
                PackState::Idle => self.start(runtime),
                PackState::Failed(_) if self.retry_at.is_some_and(|at| Instant::now() >= at) => self.start(runtime),
                _ => {}
            }
        }
        installed
    }

    /// The "Retry" button: now, without waiting for the next automatic try.
    pub fn retry(&mut self, runtime: &tokio::runtime::Handle) {
        if self.automatic && matches!(self.state, PackState::Failed(_)) {
            self.start(runtime);
        }
    }

    fn start(&mut self, runtime: &tokio::runtime::Handle) {
        log::info!(
            "media: downloading the plugins pack {} ({} MB) to {}",
            PACK.version,
            PACK.size / (1024 * 1024),
            self.root.display()
        );
        self.state = PackState::Downloading(Progress {
            received: 0,
            total: PACK.size,
            preparing: false,
        });
        self.retry_at = None;
        self.cancel = Cancel::default();
        let (source, root, cancel, tx) = (self.source.clone(), self.root.clone(), self.cancel.clone(), self.tx.clone());
        runtime.spawn(async move {
            let progress = tx.clone();
            let result = media_pack::install(&source, &PACK, &root, &cancel, move |p| {
                let _ = progress.send(Event::Progress(p));
            })
            .await;
            let _ = tx.send(Event::Done(result));
        });
    }
}

impl Drop for PluginPack {
    fn drop(&mut self) {
        // the viewer is closing: the staging folder is removed, the next
        // session starts the download again
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_wait_longer_each_time_and_never_for_an_unpublished_pack() {
        assert_eq!(retry_delay(Failure::Network, 1), Some(Duration::from_secs(30)));
        assert_eq!(retry_delay(Failure::Network, 2), Some(Duration::from_secs(60)));
        assert_eq!(retry_delay(Failure::Integrity, 4), Some(Duration::from_secs(240)));
        assert_eq!(retry_delay(Failure::Network, 40), Some(Duration::from_secs(900)));
        assert_eq!(retry_delay(Failure::Busy, 7), Some(BUSY_RETRY));
        assert_eq!(retry_delay(Failure::Unsupported, 1), None);
    }

    #[test]
    fn a_missing_pack_waits_until_media_want_it_and_tests_never_download() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let root = std::env::temp_dir().join(format!("aurora-media-pack-{}", uuid::Uuid::new_v4()));
        let mut pack = PluginPack::new(root);
        assert!(pack.dir().is_none() && matches!(pack.state(), PackState::Idle));
        assert!(!pack.poll(true, runtime.handle()));
        assert!(matches!(pack.state(), PackState::Idle), "no download in a unit test");
        // what the worker reports ends up in the state, and in `dir`
        pack.state = PackState::Downloading(Progress::default());
        pack.tx.send(Event::Done(Err(Failure::Network))).expect("fixture");
        assert!(!pack.poll(true, runtime.handle()));
        assert!(matches!(pack.state(), PackState::Failed(Failure::Network)) && pack.retry_at.is_some());
        pack.tx.send(Event::Done(Ok(PathBuf::from("pack")))).expect("fixture");
        assert!(pack.poll(true, runtime.handle()));
        assert!(matches!(pack.state(), PackState::Ready));
        assert_eq!(pack.dir(), Some(Path::new("pack")));
    }
}
