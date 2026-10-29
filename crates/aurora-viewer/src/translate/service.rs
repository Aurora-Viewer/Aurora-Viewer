//! The app's translator: the state of the lines, the model's thread
//! (spawned at the first job), the model download and the suggestions.

use super::state::{Effect, Held, LineKey, LineShow, Suggest, TranslateSettings, Translations};
use aurora_translate::{Backend, Download, DownloadState, Lang, ModelEntry, Status, Translator, lang};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

/// The model leaves the memory (and the GPU) after this long without a job.
const IDLE_UNLOAD: Duration = Duration::from_secs(600);

/// Where the models are downloaded: `<data folder>/models`.
pub fn models_dir() -> PathBuf {
    if let Some(test) = aurora_updater::test_environment() {
        return test.root.join("models");
    }
    directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
        .map(|d| d.data_local_dir().join("models"))
        .unwrap_or_else(|| PathBuf::from("models"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetResult {
    Ready,
    Off,
    /// The model is not on disk yet: the download dialog asks first.
    NeedsDownload,
    /// This viewer was built without a model runtime.
    NotBuilt,
}

/// What a frame of the translator gives the app.
#[derive(Debug, Default)]
pub struct Polled {
    pub effects: Vec<Effect>,
    /// The settings changed (a conversation became translated after the
    /// download): save them.
    pub settings_changed: bool,
}

/// What the interface shows of the translation (kept on the World, which
/// every window reads).
#[derive(Debug, Default, Clone)]
pub struct TranslateView {
    /// By the time of their line.
    pub lines: HashMap<SystemTime, LineShow>,
    pub held: Vec<Held>,
    pub suggestions: HashMap<Uuid, &'static Lang>,
    pub partners: HashMap<Uuid, &'static Lang>,
    pub me: Option<&'static Lang>,
    /// Where the model runs, once a job started it.
    pub status: Option<Status>,
    /// The model's thread died.
    pub offline: bool,
}

impl TranslateView {
    pub fn held_in(&self, session: Uuid) -> impl Iterator<Item = &Held> {
        self.held.iter().filter(move |h| h.key.session == session)
    }
}

#[derive(Default)]
pub struct TranslateService {
    /// Bumped when the suggestions change (the lines have their own).
    bumps: u64,
    translations: Translations,
    suggest: Suggest,
    suggestions: HashMap<Uuid, &'static Lang>,
    translator: Option<Translator>,
    download: Option<(&'static ModelEntry, Download)>,
    /// The conversation and language waiting for the model download.
    pub pending: Option<(Uuid, &'static Lang)>,
    /// The demo's backend instead of llama.cpp.
    backend: Option<fn() -> Box<dyn Backend>>,
}

impl TranslateService {
    /// A service whose model is `backend` (the demo, the tests): always
    /// ready, nothing to download.
    pub fn with_backend(backend: fn() -> Box<dyn Backend>) -> Self {
        Self {
            backend: Some(backend),
            ..Self::default()
        }
    }

    fn model(settings: &TranslateSettings) -> &'static ModelEntry {
        aurora_translate::entry(&settings.model)
            .or_else(|| aurora_translate::entry(aurora_translate::DEFAULT_MODEL))
            .unwrap_or(&aurora_translate::CATALOGUE[0])
    }

    /// My language: the one chosen in the preferences, else the interface's.
    pub fn me(settings: &TranslateSettings) -> &'static Lang {
        lang(&settings.my_language)
            .or_else(|| lang(aurora_i18n::language()))
            .or_else(|| lang("en"))
            .unwrap_or(&aurora_translate::LANGS[0])
    }

    pub fn partner(settings: &TranslateSettings, session: Uuid) -> Option<&'static Lang> {
        settings.sessions.get(&session).and_then(|c| lang(c))
    }

    fn ready(&self, settings: &TranslateSettings, dir: &Path) -> bool {
        self.backend.is_some() || aurora_translate::is_ready(dir, Self::model(settings))
    }

    /// `/tr`, the pill or the suggestion card: translate the conversation to
    /// `to`, or stop with `None`.
    pub fn set_partner(&mut self, settings: &mut TranslateSettings, session: Uuid, to: Option<&'static Lang>, dir: &Path) -> SetResult {
        self.suggestions.remove(&session);
        self.bumps += 1;
        let Some(to) = to else {
            settings.sessions.remove(&session);
            self.translations.close_session(session);
            return SetResult::Off;
        };
        if self.backend.is_none() && !aurora_translate::BUILT {
            return SetResult::NotBuilt;
        }
        if !self.ready(settings, dir) {
            self.pending = Some((session, to));
            return SetResult::NeedsDownload;
        }
        settings.sessions.insert(session, to.code.into());
        SetResult::Ready
    }

    pub fn start_download(&mut self, settings: &TranslateSettings, dir: PathBuf) {
        let e = Self::model(settings);
        log::info!("downloading the translation model {}", e.id);
        self.download = Some((e, Download::start(dir, e)));
    }

    /// The model being downloaded and how far it is.
    pub fn download_state(&self) -> Option<(&'static ModelEntry, DownloadState)> {
        self.download.as_ref().map(|(e, d)| (*e, d.progress()))
    }

    /// The download dialog, while a conversation waits for the model: the
    /// model and how far its download is (`None`: not started).
    pub fn dialog(&self, settings: &TranslateSettings) -> Option<(&'static ModelEntry, Option<DownloadState>)> {
        self.pending?;
        Some((Self::model(settings), self.download_state().map(|(_, s)| s)))
    }

    /// "Cancel" or "Not now" in the download dialog.
    pub fn cancel_download(&mut self) {
        if let Some((_, d)) = self.download.take() {
            d.cancel();
        }
        self.pending = None;
    }

    fn submit(&mut self, settings: &TranslateSettings, job: aurora_translate::Job) {
        let t = self.translator.get_or_insert_with(|| {
            let backend = match self.backend {
                Some(b) => b(),
                None => aurora_translate::backend_for(aurora_translate::model_path(&models_dir(), Self::model(settings))),
            };
            Translator::spawn(backend, Self::model(settings).prompt, IDLE_UNLOAD)
        });
        t.submit(job);
    }

    /// Our message in a translated conversation; its line is already shown.
    pub fn submit_outgoing(
        &mut self,
        settings: &TranslateSettings,
        session: Uuid,
        time: SystemTime,
        text: &str,
        now: Instant,
    ) -> Vec<Effect> {
        let Some(to) = Self::partner(settings, session) else {
            return vec![Effect::Send {
                to: session,
                text: text.into(),
            }];
        };
        let (job, fx) = self
            .translations
            .outgoing(LineKey { session, time }, text, Self::me(settings), to, now);
        if let Some(job) = job {
            self.submit(settings, job);
        }
        fx
    }

    /// A private message received: translated, or watched for a suggestion.
    pub fn on_incoming(&mut self, settings: &TranslateSettings, session: Uuid, time: SystemTime, text: &str) {
        let me = Self::me(settings);
        match Self::partner(settings, session) {
            Some(partner) => {
                if let Some(job) = self.translations.incoming(LineKey { session, time }, text, partner, me) {
                    self.submit(settings, job);
                }
            }
            None => {
                if let Some(l) = self.suggest.observe(session, text, me) {
                    self.suggestions.insert(session, l);
                    self.bumps += 1;
                }
            }
        }
    }

    /// Every frame: finished translations, late messages, the download.
    pub fn poll(&mut self, settings: &mut TranslateSettings, now: Instant) -> Polled {
        let mut out = Polled::default();
        if let Some(t) = &self.translator {
            while let Some(done) = t.try_recv() {
                out.effects.extend(self.translations.done(done));
            }
        }
        out.effects.extend(self.translations.tick(now));
        if let Some((_, DownloadState::Ready(_))) = self.download_state() {
            self.download = None;
            if let Some((session, l)) = self.pending.take() {
                settings.sessions.insert(session, l.code.into());
                out.settings_changed = true;
            }
        }
        out
    }

    pub fn original(&self, key: &LineKey) -> Option<String> {
        self.translations.original(key)
    }

    pub fn retry(&mut self, settings: &TranslateSettings, key: LineKey, now: Instant) {
        if let Some(job) = self.translations.retry(key, now) {
            self.submit(settings, job);
        }
    }

    pub fn send_original(&mut self, key: LineKey) -> Option<Effect> {
        self.translations.send_original(key)
    }

    /// Where the model runs, once a job started it.
    pub fn status(&self) -> Option<Status> {
        self.translator.as_ref().map(Translator::status)
    }

    /// The model's thread died.
    pub fn offline(&self) -> bool {
        self.translator.as_ref().is_some_and(Translator::is_closed)
    }

    pub fn close_session(&mut self, session: Uuid) {
        self.translations.close_session(session);
        self.suggestions.remove(&session);
        self.bumps += 1;
    }

    /// Our held message dropped: "Edit" puts it back in the field.
    pub fn discard(&mut self, key: LineKey) {
        self.translations.discard(key);
    }

    /// "Not now" on the suggestion card.
    pub fn dismiss(&mut self, settings: &mut TranslateSettings, session: Uuid) {
        settings.dismissed.insert(session);
        self.suggestions.remove(&session);
        self.bumps += 1;
    }

    /// Changes when [`Self::view`] would.
    pub fn version(&self) -> u64 {
        self.translations.version() + self.bumps
    }

    pub fn view(&self, settings: &TranslateSettings) -> TranslateView {
        let (lines, held) = self.translations.view();
        TranslateView {
            lines,
            held,
            suggestions: self
                .suggestions
                .iter()
                .filter(|(s, _)| !settings.dismissed.contains(*s) && !settings.sessions.contains_key(*s))
                .map(|(s, l)| (*s, *l))
                .collect(),
            partners: settings
                .sessions
                .keys()
                .filter_map(|s| Some((*s, Self::partner(settings, *s)?)))
                .collect(),
            me: Some(Self::me(settings)),
            status: self.status(),
            offline: self.offline(),
        }
    }

    /// Logout: nothing in flight survives the session.
    pub fn clear(&mut self) {
        self.translations.clear();
        self.suggestions.clear();
        self.pending = None;
        self.bumps += 1;
    }
}
