//! UI-owned update state; HTTP, hashing, extraction and helper startup run on
//! workers. Optional policy follows Firestorm's UpdaterServiceSetting (0/1/3).
//!
//! After the check at launch, a running viewer checks again every
//! `Settings::update_check_interval` (Aurora only: the Legacy Viewer checks
//! at launch). Such a check is quiet: it never blocks the login, never opens
//! the update window by itself and says nothing when it fails; a version it
//! finds becomes a `Notice` (downloaded first in the Automatic mode), shown
//! once per version and session.

use crate::{
    settings::Settings,
    ui::news::{LoginInfo, VersionState},
};
use aurora_updater::{
    Cancel, Failure, Progress,
    feed::{Channel, Release, Source},
    install::Workspace,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    Disabled,
    #[default]
    Ask,
    Automatic,
}

/// How often a running viewer checks again (Preferences › Language and
/// updates). Each check is one request; while the list of releases has not
/// changed, GitHub answers it in a few bytes (`feed::Source`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Interval {
    #[default]
    Minutes10,
    Minutes30,
    Hour1,
    Hours6,
    /// Only the check at launch.
    Never,
}

impl Interval {
    pub fn duration(self) -> Option<Duration> {
        let minutes = match self {
            Interval::Minutes10 => 10,
            Interval::Minutes30 => 30,
            Interval::Hour1 => 60,
            Interval::Hours6 => 6 * 60,
            Interval::Never => return None,
        };
        Some(Duration::from_secs(minutes * 60))
    }
}

/// A new version a periodic check found, for the notice in the world (or
/// the update window on the login screen).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub version: String,
    /// Downloaded already (Automatic mode): the window installs it.
    pub ready: bool,
}

pub enum State {
    Idle,
    Checking,
    Offer,
    Downloading(Progress),
    Ready,
    Handoff,
    Error(Failure),
    Current,
    Rollback,
}
enum Event {
    /// The answer of the check numbered `generation`; one started later
    /// makes it stale.
    Checked {
        result: Result<Option<Release>, Failure>,
        generation: u64,
    },
    Progress(Progress),
    Prepared(Result<Workspace, Failure>),
    Handoff(Result<(), Failure>),
}

pub struct Updater {
    pub state: State,
    pub release: Option<Release>,
    pub open: bool,
    pub enabled: bool,
    source: Source,
    /// The releases followed (Preferences); a change checks again at once.
    channel: Channel,
    tx: crossbeam_channel::Sender<Event>,
    rx: crossbeam_channel::Receiver<Event>,
    manual: bool,
    rollback_notice: bool,
    policy_pending: bool,
    cancel: Cancel,
    prepared: Option<Workspace>,
    /// The number of the last check started (`Event::Checked`).
    generation: u64,
    /// That check is a periodic one, still running.
    quiet_check: bool,
    /// The download running or prepared was started by a periodic check:
    /// its end becomes a notice instead of opening the window.
    quiet_download: bool,
    /// When the last check ended (the next periodic one counts from there).
    last_check: Option<Instant>,
    /// The version already offered this session (window or notice): a later
    /// check does not announce it again, "Later" means the next launch.
    announced: String,
    notice: Option<Notice>,
}

pub fn checks_enabled() -> bool {
    std::env::var_os("AURORA_DEMO").is_none()
        && (aurora_updater::test_environment().is_some()
            || (env!("AURORA_BUILD_PROFILE") == "release" && published_version(aurora_version::version())))
}

/// A version the Release workflow published: a stable one, or a beta. A
/// development build (Git distance and commit after the tag) never updates.
fn published_version(version: &str) -> bool {
    aurora_updater::feed::version(version).is_some_and(|v| Channel::Beta.accepts(&v))
}

impl Updater {
    pub fn new(runtime: &tokio::runtime::Handle, channel: Channel) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let test = aurora_updater::test_environment();
        let enabled = checks_enabled();
        let rollback = std::env::var_os("AURORA_UPDATE_ROLLBACK").is_some();
        let source = test.map(|t| t.source.clone()).unwrap_or_default();
        let mut this = Self {
            state: if rollback { State::Rollback } else { State::Idle },
            release: None,
            open: rollback,
            enabled,
            source,
            channel,
            tx,
            rx,
            manual: false,
            rollback_notice: rollback,
            policy_pending: false,
            cancel: Cancel::default(),
            prepared: None,
            generation: 0,
            quiet_check: false,
            quiet_download: false,
            last_check: None,
            announced: String::new(),
            notice: None,
        };
        // Disabled still checks the minimum supported version, as Firestorm does.
        if enabled {
            this.check(runtime, false);
            if rollback {
                this.state = State::Rollback;
                this.open = true;
            }
        }
        this
    }

    pub fn busy(&self) -> bool {
        matches!(self.state, State::Checking | State::Downloading(_) | State::Handoff)
    }
    pub fn required(&self) -> bool {
        self.release.as_ref().is_some_and(|r| r.required)
    }
    pub fn blocks_login(&self) -> bool {
        self.required() || self.policy_pending || matches!(self.state, State::Handoff)
    }

    pub fn login_block_reason(&self) -> Option<&'static str> {
        if self.required() {
            Some(aurora_i18n::tr!("update-login-required"))
        } else if self.policy_pending {
            Some(aurora_i18n::tr!("update-login-checking"))
        } else if matches!(self.state, State::Handoff) {
            Some(aurora_i18n::tr!("update-restarting"))
        } else {
            None
        }
    }

    /// The new version a periodic check found, once.
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notice.take()
    }

    pub fn check(&mut self, runtime: &tokio::runtime::Handle, manual: bool) {
        if !self.enabled || self.busy() {
            return;
        }
        self.manual = manual;
        self.policy_pending = true;
        if manual {
            self.rollback_notice = false;
        }
        self.prepared.take();
        self.quiet_download = false;
        self.state = State::Checking;
        self.open = manual;
        self.quiet_check = false;
        self.spawn_check(runtime);
    }

    fn spawn_check(&mut self, runtime: &tokio::runtime::Handle) {
        self.generation += 1;
        let generation = self.generation;
        let source = self.source.clone();
        let channel = self.channel;
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let result =
                async { tokio::time::timeout(std::time::Duration::from_secs(8), source.check(aurora_version::version(), channel)).await? }
                    .await
                    .map_err(|e: anyhow::Error| {
                        log::warn!("release check: {e:#}");
                        Failure::Network
                    });
            let _ = tx.send(Event::Checked { result, generation });
        });
    }

    /// A periodic check is due: checks enabled, an interval chosen and gone
    /// by since the last check, and nothing else going on (the window
    /// closed, no download, nothing prepared to install).
    fn periodic_due(&self, settings: &Settings, now: Instant) -> bool {
        let Some(every) = settings.update_check_interval.duration() else {
            return false;
        };
        self.enabled
            && settings.update_mode != Mode::Disabled
            && !self.open
            && !self.quiet_check
            && !self.rollback_notice
            && matches!(self.state, State::Idle | State::Current | State::Error(_) | State::Offer)
            && self.last_check.is_some_and(|at| now.duration_since(at) >= every)
    }

    pub fn poll(&mut self, settings: &Settings, info: &mut LoginInfo, runtime: &tokio::runtime::Handle) -> bool {
        info.can_check_updates = self.enabled;
        // Joining the Beta channel shows its newest beta now; leaving it
        // forgets an offered beta (a stable newer than the installed one
        // comes at the next check).
        if settings.update_channel != self.channel && !self.busy() {
            self.channel = settings.update_channel;
            if matches!(self.state, State::Offer | State::Ready) {
                self.prepared.take();
                self.release = None;
                self.state = State::Idle;
                self.open = false;
            }
            self.check(runtime, false);
        }
        if self.periodic_due(settings, Instant::now()) {
            self.quiet_check = true;
            self.spawn_check(runtime);
        }
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Checked { generation, .. } if generation != self.generation => {}
                Event::Checked { result, .. } if self.quiet_check => {
                    self.quiet_check = false;
                    self.last_check = Some(Instant::now());
                    self.quiet_result(result, settings, info, runtime);
                }
                Event::Checked { result: Ok(release), .. } => {
                    self.last_check = Some(Instant::now());
                    self.policy_pending = false;
                    self.release = release;
                    self.state = if self.release.is_some() { State::Offer } else { State::Current };
                    if let Some(r) = &self.release {
                        info.version = VersionState::Available {
                            version: r.version.clone(),
                            url: String::new(),
                        };
                        self.open = should_offer(r.required, self.manual, settings.update_mode, &settings.ignored_update, &r.tag);
                        if self.open {
                            self.announced = r.tag.clone();
                        }
                        if self.open && !self.manual && !self.rollback_notice && !r.required && settings.update_mode == Mode::Automatic {
                            self.download(runtime);
                        }
                    } else {
                        info.version = VersionState::UpToDate;
                        self.open = self.manual;
                    }
                    // Rollback still queries the release policy: restoring an
                    // obsolete binary must not bypass a required update.
                    if self.rollback_notice {
                        self.state = State::Rollback;
                        self.open = true;
                    }
                }
                Event::Checked { result: Err(error), .. } => {
                    self.last_check = Some(Instant::now());
                    self.policy_pending = false;
                    info.version = VersionState::Failed;
                    self.state = State::Error(error);
                    self.open = self.manual;
                    if self.rollback_notice {
                        self.state = State::Rollback;
                        self.open = true;
                    }
                }
                Event::Progress(p) => {
                    if !self.cancel.is_cancelled() {
                        self.state = State::Downloading(p);
                    }
                }
                Event::Prepared(result) => {
                    if self.cancel.is_cancelled() {
                        drop(result);
                        self.state = State::Offer;
                    } else {
                        let quiet = std::mem::take(&mut self.quiet_download) && !self.open;
                        match result {
                            Ok(work) => {
                                self.prepared = Some(work);
                                self.state = State::Ready;
                                if quiet {
                                    self.notice = self.release.as_ref().map(|r| Notice {
                                        version: r.version.clone(),
                                        ready: true,
                                    });
                                } else {
                                    self.open = true;
                                }
                            }
                            // a quiet download that failed is offered as a
                            // plain new version: the window says why if it
                            // fails again
                            Err(error) if quiet => {
                                log::warn!("background update download: {error:?}");
                                self.state = State::Offer;
                                self.notice = self.release.as_ref().map(|r| Notice {
                                    version: r.version.clone(),
                                    ready: false,
                                });
                            }
                            Err(error) => {
                                self.state = State::Error(error);
                                self.open = true;
                            }
                        }
                    }
                }
                Event::Handoff(Ok(())) => return true,
                Event::Handoff(Err(error)) => {
                    self.state = State::Error(error);
                    self.open = true;
                }
            }
        }
        if matches!(self.state, State::Checking) {
            info.version = VersionState::Checking;
        }
        false
    }

    /// The answer of a periodic check: a version not announced yet becomes
    /// a notice (downloaded first in the Automatic mode); a failure is only
    /// logged, the next check tries again.
    fn quiet_result(
        &mut self,
        result: Result<Option<Release>, Failure>,
        settings: &Settings,
        info: &mut LoginInfo,
        runtime: &tokio::runtime::Handle,
    ) {
        // the user opened the window or started something meanwhile: theirs wins
        if self.open || !matches!(self.state, State::Idle | State::Current | State::Error(_) | State::Offer) {
            return;
        }
        let release = match result {
            Ok(release) => release,
            Err(_) => return,
        };
        let Some(r) = release else {
            self.release = None;
            self.state = State::Current;
            info.version = VersionState::UpToDate;
            return;
        };
        info.version = VersionState::Available {
            version: r.version.clone(),
            url: String::new(),
        };
        let announce = announces(&r, &self.announced, &settings.ignored_update);
        let (tag, version, required) = (r.tag.clone(), r.version.clone(), r.required);
        self.release = Some(r);
        self.state = State::Offer;
        if !announce {
            return;
        }
        self.announced = tag;
        if !required && settings.update_mode == Mode::Automatic {
            self.quiet_download = true;
            self.start_download(runtime, false);
        } else {
            self.notice = Some(Notice { version, ready: false });
        }
    }

    pub fn download(&mut self, runtime: &tokio::runtime::Handle) {
        self.quiet_download = false;
        self.start_download(runtime, true);
    }

    /// `show`: the window shows the download (else it waits for the end).
    fn start_download(&mut self, runtime: &tokio::runtime::Handle, show: bool) {
        let Some(release) = self.release.clone() else { return };
        if self.busy() {
            return;
        }
        if !cfg!(windows) || release.archive.is_none() || release.checksum.is_none() {
            if show {
                self.state = State::Error(Failure::Unsupported);
                self.open = true;
            } else {
                // no package to install here (macOS, Linux): the notice
                // offers the version, the window links its page
                self.quiet_download = false;
                self.notice = Some(Notice {
                    version: release.version.clone(),
                    ready: false,
                });
            }
            return;
        }
        self.state = State::Downloading(Progress::default());
        if show {
            self.open = true;
        }
        self.cancel = Cancel::default();
        let cancel = self.cancel.clone();
        let source = self.source.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let result = async {
                let work = tokio::task::spawn_blocking(|| {
                    let path = std::env::current_exe().map_err(|e| {
                        log::warn!("update install path: {e}");
                        Failure::Filesystem
                    })?;
                    let parent = path.parent().ok_or(Failure::Filesystem)?;
                    if !Workspace::supported_install(parent) {
                        return Err(Failure::Unsupported);
                    }
                    Workspace::new(parent).map_err(|e| {
                        log::warn!("update prepare: {e:#}");
                        Failure::Filesystem
                    })
                })
                .await
                .map_err(|_| Failure::Filesystem)??;
                let progress = tx.clone();
                aurora_updater::package::download(&source, &release, &work.root, &cancel, move |p| {
                    let _ = progress.send(Event::Progress(p));
                })
                .await?;
                Ok(work)
            }
            .await;
            let _ = tx.send(Event::Prepared(result));
        });
    }

    pub fn cancel(&mut self) {
        self.cancel.cancel();
    }
    pub fn restart(&mut self) {
        let Some(work) = self.prepared.take() else { return };
        let Some(release) = &self.release else { return };
        let version = release.version.clone();
        let maintenance = release.maintenance;
        let tx = self.tx.clone();
        self.state = State::Handoff;
        std::thread::spawn(move || {
            let result = work.handoff(&version, maintenance).map_err(|e| {
                log::warn!("update handoff: {e:#}");
                Failure::Startup
            });
            let _ = tx.send(Event::Handoff(result));
        });
    }
}

fn should_offer(required: bool, manual: bool, mode: Mode, ignored: &str, tag: &str) -> bool {
    required || manual || (mode != Mode::Disabled && ignored != tag)
}

/// A periodic check announces a version once per session, unless it was
/// skipped ("Skip this version"); a required one even then.
fn announces(release: &Release, announced: &str, ignored: &str) -> bool {
    release.tag != announced && (release.required || release.tag != ignored)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skipped_version_manual_check_and_required_policy() {
        assert!(!should_offer(false, false, Mode::Ask, "v0.3.0", "v0.3.0"));
        assert!(should_offer(false, true, Mode::Disabled, "v0.3.0", "v0.3.0"));
        assert!(should_offer(true, false, Mode::Disabled, "v0.3.0", "v0.3.0"));
        assert!(should_offer(false, false, Mode::Ask, "v0.3.0", "v0.3.1"));
        assert!(!should_offer(false, false, Mode::Disabled, "", "v0.3.1"));
    }
    #[test]
    fn a_beta_keeps_checking_for_updates_a_development_build_does_not() {
        for (version, checks) in [
            ("0.7.6", true),
            ("0.7.7-beta.20261025.1", true),
            ("0.7.7-rc.1", false),
            ("0.7.6+3.gabcdef123", false),
            ("0.7.7-beta.20261025.1+2.gabcdef123", false),
            ("0.0.0+0.gunknown", false),
        ] {
            assert_eq!(published_version(version), checks, "{version}");
        }
    }

    fn release(tag: &str, required: bool) -> Release {
        Release {
            maintenance: Default::default(),
            tag: tag.into(),
            version: tag.trim_start_matches('v').into(),
            notes: String::new(),
            url: aurora_updater::feed::RELEASE_PAGE.into(),
            archive: None,
            checksum: None,
            required,
        }
    }

    fn checked(updater: &Updater, result: Result<Option<Release>, Failure>) {
        let generation = updater.generation;
        updater.tx.send(Event::Checked { result, generation }).expect("result");
    }

    #[test]
    fn initial_policy_check_blocks_login_until_a_result_or_network_failure() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let mut updater = Updater::new(rt.handle(), Channel::Stable);
        updater.policy_pending = true;
        assert!(updater.blocks_login());
        assert!(updater.login_block_reason().is_some());
        checked(&updater, Err(Failure::Network));
        updater.poll(&Settings::default(), &mut LoginInfo::preview_release("0.3.0"), rt.handle());
        assert!(!updater.blocks_login());
        assert!(updater.login_block_reason().is_none());
    }

    #[test]
    fn rollback_keeps_required_update_blocking_login() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let mut updater = Updater::new(rt.handle(), Channel::Stable);
        updater.rollback_notice = true;
        updater.state = State::Rollback;
        checked(&updater, Ok(Some(release("v0.3.0", true))));
        let settings = Settings {
            update_mode: Mode::Disabled,
            ignored_update: "v0.3.0".into(),
            ..Default::default()
        };
        let mut info = LoginInfo::preview_release("0.3.0");
        updater.poll(&settings, &mut info, rt.handle());
        assert!(updater.blocks_login());
        assert!(updater.open && matches!(updater.state, State::Rollback));
    }

    #[test]
    fn the_check_interval_defaults_to_ten_minutes() {
        assert_eq!(Settings::default().update_check_interval.duration(), Some(Duration::from_secs(600)));
        assert_eq!(Interval::Never.duration(), None);
    }

    #[test]
    fn a_periodic_check_waits_its_interval_and_a_quiet_moment() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let mut updater = Updater::new(rt.handle(), Channel::Stable);
        updater.enabled = true;
        updater.state = State::Current;
        let settings = Settings::default();
        let start = Instant::now();
        // the check at launch has not answered yet
        assert!(!updater.periodic_due(&settings, start));
        updater.last_check = Some(start);
        assert!(!updater.periodic_due(&settings, start + Duration::from_secs(599)));
        let later = start + Duration::from_secs(600);
        assert!(updater.periodic_due(&settings, later));
        // never over the open window, a download or an install waiting
        updater.open = true;
        assert!(!updater.periodic_due(&settings, later));
        updater.open = false;
        for state in [
            State::Checking,
            State::Downloading(Progress::default()),
            State::Ready,
            State::Rollback,
        ] {
            updater.state = state;
            assert!(!updater.periodic_due(&settings, later));
        }
        updater.state = State::Offer;
        assert!(updater.periodic_due(&settings, later));
        // nor when switched off
        let never = Settings {
            update_check_interval: Interval::Never,
            ..Default::default()
        };
        let disabled = Settings {
            update_mode: Mode::Disabled,
            ..Default::default()
        };
        assert!(!updater.periodic_due(&never, later));
        assert!(!updater.periodic_due(&disabled, later));
    }

    #[test]
    fn a_periodic_check_announces_a_new_version_once_without_blocking_or_opening() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let mut updater = Updater::new(rt.handle(), Channel::Stable);
        updater.enabled = true;
        updater.state = State::Current;
        updater.last_check = Some(Instant::now() - Duration::from_secs(3600));
        let settings = Settings::default();
        let mut info = LoginInfo::preview_release("0.3.0");
        // the poll starts the periodic check; its answer comes back
        updater.poll(&settings, &mut info, rt.handle());
        assert!(updater.quiet_check && !updater.blocks_login());
        checked(&updater, Ok(Some(release("v0.4.0", false))));
        updater.poll(&settings, &mut info, rt.handle());
        assert!(!updater.open && !updater.blocks_login());
        assert!(matches!(updater.state, State::Offer));
        let notice = updater.take_notice().expect("notice");
        assert_eq!((notice.version.as_str(), notice.ready), ("0.4.0", false));
        assert!(updater.take_notice().is_none());
        // the same version, checked again: no second notice
        assert!(!announces(&release("v0.4.0", false), &updater.announced, ""));
        // a newer one, a skipped one, a required skipped one
        assert!(announces(&release("v0.4.1", false), &updater.announced, ""));
        assert!(!announces(&release("v0.4.1", false), &updater.announced, "v0.4.1"));
        assert!(announces(&release("v0.4.1", true), &updater.announced, "v0.4.1"));
    }

    #[test]
    fn a_failed_periodic_check_stays_silent_and_a_manual_check_wins_over_it() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let mut updater = Updater::new(rt.handle(), Channel::Stable);
        updater.enabled = true;
        updater.state = State::Current;
        updater.last_check = Some(Instant::now() - Duration::from_secs(3600));
        let settings = Settings::default();
        let mut info = LoginInfo::preview_release("0.3.0");
        updater.poll(&settings, &mut info, rt.handle());
        checked(&updater, Err(Failure::Network));
        updater.poll(&settings, &mut info, rt.handle());
        assert!(!updater.open && matches!(updater.state, State::Current));
        assert!(updater.take_notice().is_none());
        // a periodic check running, then the user checks: the periodic answer
        // is stale and the manual one opens the window
        updater.last_check = Some(Instant::now() - Duration::from_secs(3600));
        updater.poll(&settings, &mut info, rt.handle());
        let stale = updater.generation;
        updater.check(rt.handle(), true);
        updater
            .tx
            .send(Event::Checked {
                result: Ok(Some(release("v0.4.0", false))),
                generation: stale,
            })
            .expect("stale");
        updater.poll(&settings, &mut info, rt.handle());
        assert!(updater.open && matches!(updater.state, State::Checking));
        checked(&updater, Ok(Some(release("v0.4.0", false))));
        updater.poll(&settings, &mut info, rt.handle());
        assert!(updater.open && matches!(updater.state, State::Offer));
        assert!(updater.take_notice().is_none());
    }
}
