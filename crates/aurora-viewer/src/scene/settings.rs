//! Environment settings assets (sky, water, day cycle): fetched through the
//! ViewerAsset capability (`settings_id`, the AT_SETTINGS type name), kept
//! in the disk cache and parsed in the background pool. Port of
//! LLSettingsVOBase::getSettingsAsset / onAssetDownloadComplete
//! (newview/llsettingsvo.cpp, originally LGPL 2.1).
//!
//! Used for Firestorm's default day and the environment selector's sky,
//! water and day cycle items.

use super::jobs::{JobResult, Jobs};
use crate::world::eep::Settings;
use aurora_net::{FetchRequest, FetchResult, Fetcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FETCH_KIND_SETTINGS: u64 = 6 << 60;
/// Fetch attempts before the asset is reported missing.
const MAX_FAILURES: u32 = 3;

/// Where a settings asset stands.
#[derive(Debug, Clone)]
pub enum SettingsState {
    Pending,
    Ready(Arc<Settings>),
    /// Not found or not parseable after the retries.
    Failed,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Step {
    New,
    CacheCheck,
    Fetching,
    Ready,
    /// Waiting for a (re)fetch.
    Retry,
    Failed,
}

struct Entry {
    settings: Option<Arc<Settings>>,
    step: Step,
    retry_at: Option<Instant>,
    failures: u32,
}

type Parsed = (Uuid, Option<Settings>, bool);

pub struct SettingsStreamer {
    entries: HashMap<Uuid, Entry>,
    by_key: HashMap<u64, Uuid>,
    next_key: u64,
    cache_dir: PathBuf,
    rx: crossbeam_channel::Receiver<Parsed>,
    tx: crossbeam_channel::Sender<Parsed>,
}

impl SettingsStreamer {
    pub fn new(cache_dir: PathBuf) -> SettingsStreamer {
        let _ = std::fs::create_dir_all(cache_dir.join("settings"));
        let (tx, rx) = crossbeam_channel::unbounded();
        SettingsStreamer {
            entries: HashMap::new(),
            by_key: HashMap::new(),
            next_key: 1,
            cache_dir,
            rx,
            tx,
        }
    }

    fn path(&self, id: &Uuid) -> PathBuf {
        self.cache_dir.join("settings").join(format!("{id}.settings"))
    }

    /// The asset, requesting it on first use.
    pub fn get(&mut self, id: Uuid) -> SettingsState {
        let e = self.entries.entry(id).or_insert(Entry {
            settings: None,
            step: Step::New,
            retry_at: None,
            failures: 0,
        });
        match (&e.settings, e.step) {
            (Some(s), _) => SettingsState::Ready(s.clone()),
            (None, Step::Failed) => SettingsState::Failed,
            _ => SettingsState::Pending,
        }
    }

    /// A settings asset known without fetching it (the offline demo's
    /// library).
    pub fn insert_local(&mut self, id: Uuid, settings: Settings) {
        self.entries.insert(
            id,
            Entry {
                settings: Some(Arc::new(settings)),
                step: Step::Ready,
                retry_at: None,
                failures: 0,
            },
        );
    }

    fn failed(e: &mut Entry, retry: Duration) {
        e.failures += 1;
        if e.failures >= MAX_FAILURES {
            e.step = Step::Failed;
        } else {
            e.step = Step::Retry;
            e.retry_at = Some(Instant::now() + retry);
        }
    }

    pub fn update(&mut self, jobs: &Jobs, fetcher: &Fetcher, viewer_asset: Option<&str>) {
        while let Ok((id, settings, from_cache)) = self.rx.try_recv() {
            let Some(e) = self.entries.get_mut(&id) else {
                continue;
            };
            match settings {
                Some(s) => {
                    e.settings = Some(Arc::new(s));
                    e.step = Step::Ready;
                }
                None if from_cache => e.step = Step::Retry,
                None => {
                    log::warn!("settings asset {id} could not be parsed");
                    Self::failed(e, Duration::from_secs(30));
                }
            }
        }
        let now = Instant::now();
        let mut to_fetch = Vec::new();
        for (id, e) in self.entries.iter_mut() {
            if e.retry_at.is_some_and(|t| now < t) {
                continue;
            }
            match e.step {
                Step::New => {
                    e.step = Step::CacheCheck;
                    let path = self.cache_dir.join("settings").join(format!("{id}.settings"));
                    let tx = self.tx.clone();
                    let id = *id;
                    jobs.spawn(move || {
                        let s = crate::cache::read_touch(&path).ok().and_then(|d| Settings::from_asset(&d));
                        let _ = tx.send((id, s, true));
                        JobResult::Done
                    });
                }
                Step::Retry if viewer_asset.is_some() => {
                    e.step = Step::Fetching;
                    to_fetch.push(*id);
                }
                _ => {}
            }
        }
        let Some(base) = viewer_asset else {
            return;
        };
        for id in to_fetch {
            let key = FETCH_KIND_SETTINGS | self.next_key;
            self.next_key += 1;
            self.by_key.insert(key, id);
            fetcher.request(FetchRequest {
                key,
                url: super::textures::asset_url(base, "settings_id", &id),
                range: None,
                // the sky waits on it: ahead of textures and meshes
                priority: 1e12,
                accept: "*/*",
            });
        }
    }

    pub fn on_fetch(&mut self, r: FetchResult, jobs: &Jobs) {
        let Some(id) = self.by_key.remove(&r.key) else {
            return;
        };
        match r.data {
            Ok(d) => {
                let path = self.path(&id);
                let tx = self.tx.clone();
                jobs.spawn(move || {
                    let s = Settings::from_asset(&d);
                    if s.is_some() {
                        let _ = crate::cache::write(path, &d);
                    }
                    let _ = tx.send((id, s, false));
                    JobResult::Done
                });
            }
            Err(err) => {
                log::info!("settings asset {id}: HTTP {} {err}", r.status);
                if let Some(e) = self.entries.get_mut(&id) {
                    if r.status == 404 {
                        // no such asset (e.g. a grid without Linden's library)
                        e.step = Step::Failed;
                    } else {
                        Self::failed(e, Duration::from_secs(10));
                    }
                }
            }
        }
    }
}
