//! Aurora's own media plugins (SLPlugin, CEF, LibVLC): a pack downloaded once
//! into the user's data folder when no plugins are found, then used by every
//! session. The state is owned by the UI thread; HTTP, hashing and extraction
//! run on workers (`aurora_updater::media_pack`).
//!
//! Nothing is downloaded before the user agrees: the first time the pack is
//! needed, a card asks (`ui/media_pack.rs`), and the answer is kept in the
//! settings (`MediaSettings::pack_consent`). After "Not now", the
//! preferences, the pages that would show the web and the media controls
//! offer the download. The Legacy Viewer ships its plugins inside its
//! installer, so it never has to ask.

use aurora_updater::{
    Cancel, Failure, Progress,
    feed::Source,
    media_pack::{self, PACK},
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The user's answer to the download of the pack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Consent {
    /// Never asked: the card asks when the pack is first needed.
    #[default]
    Unasked,
    /// "Download": the pack is downloaded (and its newer versions) when needed.
    Accepted,
    /// "Not now": nothing is downloaded until a Download button is pressed.
    Declined,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum PackState {
    /// Not installed and not downloading: other plugins are used, or the
    /// download waits for the user's agreement.
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
    consent: Consent,
    /// No plugins were found at the last `poll`.
    wanted: bool,
    /// `AURORA_DEMO_MEDIA_PACK=consent`: the card, whatever was answered.
    demo_card: bool,
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

/// A pack of any version was installed under `root` (its folder is named by
/// its version number): it was downloaded before the card asked.
fn any_pack_installed(root: &Path) -> bool {
    std::fs::read_dir(root).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.path().is_dir() && e.file_name().to_string_lossy().parse::<u32>().is_ok())
    })
}

/// The answer to keep: an installation that already has a pack is never
/// asked (its newer versions follow it as before).
fn settled(consent: Consent, had_pack: bool) -> Consent {
    match consent {
        Consent::Unasked if had_pack => Consent::Accepted,
        other => other,
    }
}

impl PluginPack {
    pub fn new(root: PathBuf, consent: Consent) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let dir = media_pack::installed(&root, &PACK);
        let consent = settled(consent, dir.is_some() || any_pack_installed(&root));
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
            consent,
            wanted: false,
            demo_card: false,
        };
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
            this.demo_card = mode == "consent";
            this.state = match mode.as_str() {
                "consent" | "declined" => PackState::Idle,
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

    /// The answer to keep in the settings (an existing pack counts as yes).
    pub fn consent(&self) -> Consent {
        self.consent
    }

    /// The consent card is due: the pack is needed, missing, and the user
    /// was never asked.
    pub fn asks(&self) -> bool {
        self.demo_card || (self.automatic && self.wanted && self.consent == Consent::Unasked && matches!(self.state, PackState::Idle))
    }

    /// The card's answer, or a Download / Retry button (`true`). Returns the
    /// answer to keep in the settings; a state that is only shown keeps
    /// nothing.
    pub fn answer(&mut self, download: bool, runtime: &tokio::runtime::Handle) -> Option<Consent> {
        self.demo_card = false;
        if !download {
            log::info!("media: plugins pack not downloaded (Not now)");
            self.consent = Consent::Declined;
            return self.automatic.then_some(self.consent);
        }
        self.consent = Consent::Accepted;
        if !self.automatic {
            // the demo shows what follows, without the network
            if matches!(self.state, PackState::Idle) {
                self.state = PackState::Downloading(Progress {
                    received: PACK.size / 20 * 9,
                    total: PACK.size,
                    preparing: false,
                });
            }
            return None;
        }
        if matches!(self.state, PackState::Idle | PackState::Failed(_)) {
            self.start(runtime);
        }
        Some(self.consent)
    }

    /// Once per frame. `wanted`: no plugins were found (the web pages of the
    /// profiles and of the search need them even with media disabled). True
    /// when the pack has just been installed. Downloads by itself only once
    /// the user agreed.
    pub fn poll(&mut self, wanted: bool, runtime: &tokio::runtime::Handle) -> bool {
        self.wanted = wanted;
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
        if wanted && self.automatic && self.consent == Consent::Accepted {
            match self.state {
                PackState::Idle => self.start(runtime),
                PackState::Failed(_) if self.retry_at.is_some_and(|at| Instant::now() >= at) => self.start(runtime),
                _ => {}
            }
        }
        installed
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
        // session starts the download again (the answer was yes)
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
        let mut pack = PluginPack::new(root, Consent::Accepted);
        let missing = |state: PackState| matches!(state, PackState::Idle);
        assert!(pack.dir().is_none() && missing(pack.state()));
        assert!(!pack.poll(true, runtime.handle()));
        assert!(missing(pack.state()), "no download in a unit test");
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

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("aurora-media-pack-{}", uuid::Uuid::new_v4()))
    }

    /// The download waits for "Download": never asked and "Not now" start
    /// nothing, even when the pack is wanted and automatic downloads allowed.
    #[test]
    fn nothing_downloads_before_the_user_agrees_and_the_answer_is_kept() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let mut pack = PluginPack::new(temp_root(), Consent::Unasked);
        assert_eq!(pack.consent(), Consent::Unasked);
        // as in a published build (no download can start below: every poll
        // keeps the state Idle, and "Download" is never answered here)
        pack.automatic = true;
        assert!(!pack.asks(), "not before a poll says the pack is wanted");
        assert!(!pack.poll(true, runtime.handle()));
        assert!(matches!(pack.state(), PackState::Idle), "no download without an answer");
        assert!(pack.asks());
        // not wanted (plugins found in a chosen folder): no card
        pack.poll(false, runtime.handle());
        assert!(!pack.asks());
        pack.poll(true, runtime.handle());
        // "Not now": kept, no card, still no download
        assert_eq!(pack.answer(false, runtime.handle()), Some(Consent::Declined));
        assert!(!pack.poll(true, runtime.handle()));
        assert!(matches!(pack.state(), PackState::Idle) && !pack.asks());
        // the next session reads the kept answer: no card again
        let mut next = PluginPack::new(temp_root(), Consent::Declined);
        next.automatic = true;
        next.poll(true, runtime.handle());
        assert!(matches!(next.state(), PackState::Idle) && !next.asks());
        // a failure after an earlier yes retries only once its delay passed
        let mut failed = PluginPack::new(temp_root(), Consent::Accepted);
        failed.automatic = true;
        failed.state = PackState::Failed(Failure::Network);
        failed.poll(true, runtime.handle());
        assert!(matches!(failed.state(), PackState::Failed(_)) && !failed.asks());
    }

    /// A shown state (demo, tests) answers without the network and keeps
    /// nothing in the settings.
    #[test]
    fn a_shown_state_answers_without_downloading_or_saving() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let mut pack = PluginPack::new(temp_root(), Consent::Unasked);
        pack.demo_card = true;
        assert!(pack.asks());
        assert_eq!(pack.answer(true, runtime.handle()), None);
        assert!(!pack.asks());
        assert!(matches!(pack.state(), PackState::Downloading(_)), "shows what follows");
        let mut later = PluginPack::new(temp_root(), Consent::Unasked);
        later.demo_card = true;
        assert_eq!(later.answer(false, runtime.handle()), None);
        assert!(matches!(later.state(), PackState::Idle) && !later.asks());
    }

    /// An installation that already holds a pack, of this version or an
    /// older one, is never asked.
    #[test]
    fn an_existing_pack_counts_as_yes() {
        assert_eq!(settled(Consent::Unasked, true), Consent::Accepted);
        assert_eq!(settled(Consent::Unasked, false), Consent::Unasked);
        assert_eq!(settled(Consent::Declined, true), Consent::Declined);
        assert_eq!(settled(Consent::Accepted, false), Consent::Accepted);
        let root = temp_root();
        assert!(!any_pack_installed(&root), "no folder");
        std::fs::create_dir_all(root.join(".staging-1")).expect("fixture");
        std::fs::write(root.join("7"), b"").expect("fixture");
        assert!(!any_pack_installed(&root), "a staging folder or a file is no pack");
        std::fs::create_dir_all(root.join("1")).expect("fixture");
        assert!(any_pack_installed(&root));
        assert_eq!(PluginPack::new(root.clone(), Consent::Unasked).consent(), Consent::Accepted);
        let _ = std::fs::remove_dir_all(root);
    }
}
