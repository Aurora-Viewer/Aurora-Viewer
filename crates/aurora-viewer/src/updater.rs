//! UI-owned update state; HTTP, hashing, extraction and helper startup run on
//! workers. Optional policy follows Firestorm's UpdaterServiceSetting (0/1/3).

use crate::{
    settings::Settings,
    ui::news::{LoginInfo, VersionState},
};
use aurora_updater::{
    Cancel, Failure, Progress,
    feed::{Channel, Release, Source},
    install::Workspace,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    Disabled,
    #[default]
    Ask,
    Automatic,
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
    Checked(Result<Option<Release>, Failure>),
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
        self.state = State::Checking;
        self.open = manual;
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
            let _ = tx.send(Event::Checked(result));
        });
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
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Checked(Ok(release)) => {
                    self.policy_pending = false;
                    self.release = release;
                    self.state = if self.release.is_some() { State::Offer } else { State::Current };
                    if let Some(r) = &self.release {
                        info.version = VersionState::Available {
                            version: r.version.clone(),
                            url: String::new(),
                        };
                        self.open = should_offer(r.required, self.manual, settings.update_mode, &settings.ignored_update, &r.tag);
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
                Event::Checked(Err(error)) => {
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
                        match result {
                            Ok(work) => {
                                self.prepared = Some(work);
                                self.state = State::Ready;
                                self.open = true;
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

    pub fn download(&mut self, runtime: &tokio::runtime::Handle) {
        let Some(release) = self.release.clone() else { return };
        if self.busy() {
            return;
        }
        if !cfg!(windows) || release.archive.is_none() || release.checksum.is_none() {
            self.state = State::Error(Failure::Unsupported);
            self.open = true;
            return;
        }
        self.state = State::Downloading(Progress::default());
        self.open = true;
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

    #[test]
    fn initial_policy_check_blocks_login_until_a_result_or_network_failure() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let mut updater = Updater::new(rt.handle(), Channel::Stable);
        updater.policy_pending = true;
        assert!(updater.blocks_login());
        assert!(updater.login_block_reason().is_some());
        updater.tx.send(Event::Checked(Err(Failure::Network))).expect("fixture");
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
        let release = Release {
            maintenance: Default::default(),
            tag: "v0.3.0".into(),
            version: "0.3.0".into(),
            notes: String::new(),
            url: aurora_updater::feed::RELEASE_PAGE.into(),
            archive: None,
            checksum: None,
            required: true,
        };
        updater.tx.send(Event::Checked(Ok(Some(release)))).expect("result");
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
}
