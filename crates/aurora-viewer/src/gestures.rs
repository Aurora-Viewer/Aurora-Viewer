//! Active gestures and nonblocking playback, port of LLGestureMgr
//! (Firestorm indra/newview/llgesturemgr.cpp, originally LGPL 2.1).

pub mod builtins;
pub mod keys;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

use aurora_assets::gesture::{ANIM_STOP, Gesture, KEY_NONE, Step, WAIT_ANIMATIONS, WAIT_RELEASE, WAIT_TIME};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, PartialEq)]
pub enum Effect {
    Animation(Uuid, bool),
    Sound(Uuid),
    Chat(String),
}

pub struct Entry {
    pub asset: Uuid,
    pub definition: Gesture,
}

struct Playing {
    definition: Gesture,
    step: usize,
    wait: Option<Instant>,
    requested: BTreeSet<Uuid>,
    observed: BTreeSet<Uuid>,
    key: Option<u8>,
    released: bool,
    /// At a step that waits for its key to be released
    /// (LLMultiGesture::mWaitingKeyRelease): the only time a press of that
    /// key does not play the gesture again.
    waiting_release: bool,
}

pub struct Manager {
    pub entries: BTreeMap<Uuid, Entry>,
    pub active: BTreeSet<Uuid>,
    pub enabled: bool,
    pub revision: u64,
    playing: BTreeMap<Uuid, Playing>,
    random: u64,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            active: BTreeSet::new(),
            enabled: true,
            revision: 0,
            playing: BTreeMap::new(),
            random: 1,
        }
    }
}

impl Manager {
    pub fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn is_playing(&self, item: Uuid) -> bool {
        self.playing.contains_key(&item)
    }
    pub fn has_playing(&self) -> bool {
        !self.playing.is_empty()
    }

    /// LLGestureMgr::playGesture: a gesture played again while it plays
    /// starts over from its first step (LLMultiGesture::reset), with no stop
    /// of the animations it started: their start is simply asked again.
    /// Stopping them first made each press of a repeated hotkey stop and
    /// restart them.
    pub fn play(&mut self, item: Uuid, definition: Gesture, key: Option<u8>) {
        if !self.enabled {
            return;
        }
        self.playing.insert(
            item,
            Playing {
                definition,
                step: 0,
                wait: None,
                requested: BTreeSet::new(),
                observed: BTreeSet::new(),
                key,
                released: false,
                waiting_release: false,
            },
        );
    }

    pub fn stop(&mut self, item: Uuid) -> Vec<Effect> {
        self.playing.remove(&item).map_or_else(Vec::new, |p| {
            p.requested.union(&p.observed).map(|a| Effect::Animation(*a, false)).collect()
        })
    }

    pub fn stop_all(&mut self) -> Vec<Effect> {
        let ids: Vec<_> = self.playing.keys().copied().collect();
        ids.into_iter().flat_map(|id| self.stop(id)).collect()
    }

    pub fn release(&mut self, key: Option<u8>) {
        for p in self.playing.values_mut() {
            if key.is_none() || p.key == key {
                p.released = true;
            }
        }
    }

    fn choose(&mut self, ids: &[Uuid]) -> Option<Uuid> {
        if ids.is_empty() {
            return None;
        }
        self.random = self.random.wrapping_mul(6364136223846793005).wrapping_add(1);
        Some(ids[(self.random >> 32) as usize % ids.len()])
    }

    /// LLGestureMgr::triggerGesture: every press plays the gesture again,
    /// except while it waits for its key to be released. Holding the key
    /// back a whole playback made a repeated hotkey look like a cooldown.
    pub fn key_trigger(&mut self, key: u8, mask: u32) -> Option<Uuid> {
        if !self.enabled || key == KEY_NONE {
            return None;
        }
        let matching: Vec<_> = self
            .active
            .iter()
            .copied()
            .filter(|id| {
                self.entries
                    .get(id)
                    .is_some_and(|e| e.definition.key == key && e.definition.mask == mask)
                    && !self.playing.get(id).is_some_and(|p| p.waiting_release)
            })
            .collect();
        self.choose(&matching)
    }

    /// LLViewerInput::handleKey: a key held down repeats, and its repeats
    /// trigger gestures too, except the function keys F1 to F12.
    pub fn repeat_triggers(key: u8) -> bool {
        !(0xa1..=0xac).contains(&key)
    }

    /// LLGestureMgr::matchPrefix: exact triggers win, otherwise complete only
    /// the common suffix of active triggers, preserving the typed casing.
    pub fn complete(&self, prefix: &str) -> Option<String> {
        if !self.enabled || prefix.is_empty() {
            return None;
        }
        let count = prefix.chars().count();
        let mut common: Option<Vec<char>> = None;
        for entry in self.active.iter().filter_map(|id| self.entries.get(id)) {
            let trigger = &entry.definition.trigger;
            if trigger.chars().count() < count
                || !trigger
                    .chars()
                    .zip(prefix.chars())
                    .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
            {
                continue;
            }
            let rest: Vec<_> = trigger.chars().skip(count).collect();
            if rest.is_empty() {
                return Some(prefix.to_owned());
            }
            if let Some(common) = common.as_mut() {
                let shared = common.iter().zip(&rest).take_while(|(a, b)| a == b).count();
                common.truncate(shared);
            } else {
                common = Some(rest);
            }
        }
        common.map(|rest| prefix.chars().chain(rest).collect())
    }

    /// The active triggers [`Manager::complete`] matches for `prefix`, each
    /// once and in alphabetical order, with the text they say instead: the
    /// list of the chat field under its inline completion.
    pub fn matching(&self, prefix: &str) -> Vec<(String, String)> {
        if !self.enabled || prefix.is_empty() {
            return Vec::new();
        }
        let count = prefix.chars().count();
        let mut found: Vec<(String, String)> = self
            .active
            .iter()
            .filter_map(|id| self.entries.get(id))
            .filter(|e| {
                let trigger = &e.definition.trigger;
                trigger.chars().count() >= count
                    && trigger
                        .chars()
                        .zip(prefix.chars())
                        .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
            })
            .map(|e| (e.definition.trigger.clone(), e.definition.replacement.clone()))
            .collect();
        found.sort_by_cached_key(|(t, _)| (t.to_lowercase(), t.clone()));
        found.dedup_by(|a, b| a.0.to_lowercase() == b.0.to_lowercase());
        found
    }

    /// Firestorm matches space-delimited words, case-insensitively, and
    /// replaces only the first matching trigger in a chat line.
    pub fn revise(&mut self, text: &str) -> (String, Option<Uuid>) {
        if !self.enabled {
            return (text.into(), None);
        }
        let mut found = None;
        let mut words = Vec::new();
        for token in text.split(' ').filter(|t| !t.is_empty()) {
            let matching: Vec<_> = if found.is_none() {
                self.active
                    .iter()
                    .copied()
                    .filter(|id| {
                        self.entries.get(id).is_some_and(|e| {
                            !e.definition.trigger.is_empty() && e.definition.trigger.to_lowercase() == token.to_lowercase()
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            if let Some(id) = self.choose(&matching) {
                found = Some(id);
                if let Some(e) = self.entries.get(&id)
                    && !e.definition.replacement.is_empty()
                {
                    words.push(if e.definition.replacement.eq_ignore_ascii_case(token) {
                        token.to_owned()
                    } else {
                        e.definition.replacement.clone()
                    });
                }
            } else {
                words.push(token.to_owned());
            }
        }
        (words.join(" "), found)
    }

    pub fn similar(&self, item: Uuid, definition: &Gesture) -> Vec<Uuid> {
        self.active
            .iter()
            .copied()
            .filter(|id| {
                *id != item
                    && self.entries.get(id).is_some_and(|entry| {
                        let g = &entry.definition;
                        (!g.trigger.is_empty() && g.trigger == definition.trigger)
                            || (g.key != KEY_NONE && g.key == definition.key && g.mask == definition.mask)
                    })
            })
            .collect()
    }

    /// Requested animations remain pending until a simulator signal confirms
    /// them. Waiting for animations times out after 30 seconds like Firestorm.
    #[cfg(test)]
    pub fn tick(&mut self, now: Instant, signaled: &BTreeSet<Uuid>) -> Vec<Effect> {
        self.tick_ready(now, signaled, |_| true)
    }

    pub fn tick_ready(&mut self, now: Instant, signaled: &BTreeSet<Uuid>, mut ready: impl FnMut(&Step) -> bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.playing.retain(|_, p| {
            if p.definition.steps.iter().any(|step| !ready(step)) {
                return true;
            }
            p.observed.retain(|id| signaled.contains(id));
            let started: Vec<_> = p.requested.intersection(signaled).copied().collect();
            for id in started {
                p.requested.remove(&id);
                p.observed.insert(id);
            }
            loop {
                let Some(step) = p.definition.steps.get(p.step) else {
                    return !p.requested.is_empty() || !p.observed.is_empty();
                };
                match step {
                    Step::Animation { asset, flags, .. } => {
                        if !asset.is_nil() {
                            let start = flags & ANIM_STOP == 0;
                            effects.push(Effect::Animation(*asset, start));
                            if start {
                                p.requested.insert(*asset);
                            } else {
                                p.requested.remove(asset);
                            }
                        }
                    }
                    Step::Sound { asset, .. } => {
                        if !asset.is_nil() {
                            effects.push(Effect::Sound(*asset));
                        }
                    }
                    Step::Chat { text, .. } => effects.push(Effect::Chat(text.clone())),
                    Step::Wait { seconds, flags } => {
                        let began = *p.wait.get_or_insert(now);
                        let elapsed = now.saturating_duration_since(began);
                        let waiting = if flags & WAIT_RELEASE != 0 && p.key.is_some() {
                            p.waiting_release = !p.released && elapsed < Duration::from_secs(600);
                            p.waiting_release
                        } else if flags & WAIT_TIME != 0 {
                            elapsed.as_secs_f64() < f64::from(*seconds)
                        } else if flags & WAIT_ANIMATIONS != 0 {
                            (!p.requested.is_empty() || !p.observed.is_empty()) && elapsed < Duration::from_secs(30)
                        } else {
                            false
                        };
                        if waiting {
                            return true;
                        }
                    }
                }
                p.step += 1;
                p.wait = None;
                p.waiting_release = false;
            }
        });
        effects
    }
}
