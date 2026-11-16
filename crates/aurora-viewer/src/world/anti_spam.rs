//! Session-local flood protection, following NACLAntiSpamQueue::checkEntry
//! and NACLAntiSpamRegistry::checkQueue (indra/newview/NACLantispam.cpp).
//! Blocks last until reset or logout; they never modify the server mute list.

use super::World;
use aurora_net::{ChatMessage, ChatSourceType, ChatType, InstantMessage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub own_objects: bool,
    pub global: bool,
    pub period: u32,
    pub amount: u32,
    pub newlines: u32,
    pub sound_multiplier: u32,
    pub preload_multiplier: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            own_objects: false,
            global: false,
            period: 4,
            amount: 10,
            newlines: 70,
            sound_multiplier: 6,
            preload_multiplier: 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Queue {
    Im,
    Chat,
    ScriptDialog,
    Sound,
    SoundPreload,
}

impl Queue {
    pub fn label(self) -> &'static str {
        match self {
            Self::Im => aurora_i18n::tr!("anti-spam-messages"),
            Self::Chat => aurora_i18n::tr!("anti-spam-chat"),
            Self::ScriptDialog => aurora_i18n::tr!("anti-spam-dialogs"),
            Self::Sound => aurora_i18n::tr!("anti-spam-sounds"),
            Self::SoundPreload => aurora_i18n::tr!("anti-spam-preloads"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block,
    NewBlock,
}

struct Entry {
    started: Instant,
    amount: u32,
    blocked: bool,
}

/// The blocks of one burst, told in one nearby chat notice that grows with
/// them (`World::anti_spam_notice`). NACLAntiSpamRegistry::notify posts one
/// AntiSpamBlocked tip per source; an attack by many objects (a rezzer's
/// temporary prims, each a new id) then fills the chat with them.
struct Burst {
    first: Instant,
    /// The number of its line in the nearby chat (`LineClock`), and the
    /// text written there: an edit finds it only while it is unchanged.
    seq: u64,
    text: String,
    /// Each source and category blocked, in order.
    blocks: Vec<(Queue, Uuid, bool)>,
}

/// How long after a burst's first block another one joins its notice. An
/// attack's sources start together and each trips its limit within the
/// counting interval of the first events (4 s by default), so their blocks
/// land within a few seconds of each other; a rezzer keeps adding new
/// sources for as long as it runs. 10 s holds the spread of a default
/// interval two and a half times over, and a long attack still gets a fresh
/// line every 10 s, which the lines above the chat bar show again. A longer
/// interval set in Preferences widens it to two intervals.
pub fn burst_window(period: u32) -> Duration {
    Duration::from_secs(10).max(Duration::from_secs(2 * u64::from(period.clamp(1, 60))))
}

/// A block at `now` joins the burst that began at `first`.
fn joins(first: Instant, now: Instant, window: Duration) -> bool {
    now.saturating_duration_since(first) <= window
}

/// Bound attacker-controlled UUIDs. At capacity, new sources share a small
/// overflow counter per queue; existing blocks are never evicted by a flood.
const MAX_SOURCES: usize = 8192;

#[derive(Default)]
pub struct Registry {
    pub settings: Settings,
    pub version: u64,
    entries: HashMap<(Queue, Uuid), Entry>,
    overflow: HashMap<Queue, Entry>,
    sweep_after: Option<Instant>,
    burst: Option<Burst>,
}

impl Registry {
    pub fn configure(&mut self, settings: Settings) {
        if self.settings != settings {
            self.reset();
            self.settings = settings;
        }
    }

    pub fn reset(&mut self) {
        self.version = self.version.wrapping_add(1);
        self.entries.clear();
        self.overflow.clear();
        self.sweep_after = None;
        self.burst = None;
    }

    pub fn blocks(&self) -> Vec<(Queue, Uuid)> {
        let mut blocks: Vec<_> = self.entries.iter().filter(|(_, e)| e.blocked).map(|(k, _)| *k).collect();
        blocks.sort_unstable_by_key(|(queue, id)| (*queue as u8, *id));
        blocks
    }

    pub fn unblock(&mut self, queue: Queue, source: Uuid) {
        self.entries.remove(&(queue, source));
        self.version = self.version.wrapping_add(1);
    }

    pub fn is_blocked(&self, queue: Queue, source: Uuid) -> bool {
        let queue = if self.settings.global { Queue::Im } else { queue };
        self.settings.enabled && self.entries.get(&(queue, source)).is_some_and(|e| e.blocked)
    }

    fn check(&mut self, queue: Queue, source: Uuid, now: Instant) -> Decision {
        if !self.settings.enabled || source.is_nil() {
            return Decision::Allow;
        }
        let multiplier = match queue {
            Queue::Sound => self.settings.sound_multiplier,
            Queue::SoundPreload => self.settings.preload_multiplier,
            _ => 1,
        };
        // Keep startup and edited thresholds identical. setAllQueueAmounts
        // historically adds another x5 to sounds only after editing it;
        // the explicit sound multiplier is sufficient and predictable here.
        let limit = self.settings.amount.clamp(1, 1000).saturating_mul(multiplier.clamp(1, 100));
        let period = Duration::from_secs(self.settings.period.clamp(1, 60) as u64);
        let queue = if self.settings.global { Queue::Im } else { queue };
        let entry = self.entry(queue, source, now);
        if entry.blocked {
            return Decision::Block;
        }
        // The interval starts with the first event; its exact end is still
        // inside, as in checkEntry's <= comparison.
        if now.saturating_duration_since(entry.started) > period {
            entry.started = now;
            entry.amount = 0;
        }
        entry.amount = entry.amount.saturating_add(1);
        if entry.amount > limit {
            entry.blocked = true;
            self.version = self.version.wrapping_add(1);
            Decision::NewBlock
        } else {
            Decision::Allow
        }
    }

    /// checkNewlineFlood counts LF characters, not visual wrapping or the
    /// first line. Both chat and IM newline floods block the IM queue in the
    /// Legacy Viewer; later chat checks consult that same block as well.
    fn check_newlines(&mut self, source: Uuid, text: &str) -> Decision {
        if !self.settings.enabled || source.is_nil() {
            return Decision::Allow;
        }
        if self.is_blocked(Queue::Im, source) {
            return Decision::Block;
        }
        let limit = self.settings.newlines.clamp(5, 1000) as usize;
        if text.bytes().filter(|b| *b == b'\n').take(limit + 1).count() <= limit {
            return Decision::Allow;
        }
        let entry = self.entry(Queue::Im, source, Instant::now());
        if entry.blocked {
            return Decision::Block;
        }
        entry.blocked = true;
        self.version = self.version.wrapping_add(1);
        Decision::NewBlock
    }

    fn entry(&mut self, queue: Queue, source: Uuid, now: Instant) -> &mut Entry {
        let key = (queue, source);
        let period = Duration::from_secs(self.settings.period.clamp(1, 60) as u64);
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_SOURCES && self.sweep_after.is_none_or(|next| now >= next) {
            // Only sweep at capacity; no work proportional to scene size in
            // the normal receive path. Retain session blocks until reset.
            self.entries
                .retain(|_, e| e.blocked || now.saturating_duration_since(e.started) <= period);
            self.sweep_after = Some(now + period);
        }
        let new = || Entry {
            started: now,
            amount: 0,
            blocked: false,
        };
        if self.entries.len() >= MAX_SOURCES && !self.entries.contains_key(&key) {
            self.overflow.entry(queue).or_insert_with(new)
        } else {
            self.entries.entry(key).or_insert_with(new)
        }
    }
}

impl World {
    /// Filter before notifications, attention, sound fetches and playback.
    /// Permission questions and URL prompts share the script-dialog queue,
    /// as process_script_question / process_load_url do.
    pub(super) fn anti_spam_event(&mut self, event: &mut aurora_net::NetEvent) -> bool {
        use aurora_net::NetEvent;
        match event {
            NetEvent::ScriptDialog { object_id, .. } | NetEvent::LoadUrl { object_id, .. } => {
                self.anti_spam_check(Queue::ScriptDialog, *object_id, true)
            }
            NetEvent::ScriptQuestion { task_id, .. } => self.anti_spam_check(Queue::ScriptDialog, *task_id, true),
            NetEvent::SoundTrigger { sound, object, owner, .. } => {
                !crate::scene::sounds::is_collision(sound) && self.anti_spam_sound(Queue::Sound, *object, *owner)
            }
            // A stop must always get through, even after its source flooded.
            NetEvent::AttachedSound { sound, object, owner, .. } => !sound.is_nil() && self.anti_spam_sound(Queue::Sound, *object, *owner),
            NetEvent::PreloadSounds(sounds) => {
                sounds.retain(|s| {
                    !self.mutes.is_muted_id(&s.object)
                        && !self.mutes.sounds_muted(&s.owner)
                        && !self.anti_spam_sound(Queue::SoundPreload, s.object, s.owner)
                });
                sounds.is_empty()
            }
            _ => false,
        }
    }

    /// Source exemptions are checked before counting, including known owned
    /// objects. An unknown object is not assumed to belong to us.
    pub fn anti_spam_check(&mut self, queue: Queue, source: Uuid, object: bool) -> bool {
        if !self.anti_spam.settings.enabled || source.is_nil() || source == self.agent_id {
            return false;
        }
        if object && self.anti_spam_owned(source) && !self.anti_spam.settings.own_objects {
            return false;
        }
        let decision = self.anti_spam.check(queue, source, Instant::now());
        self.anti_spam_decision(queue, source, decision, false)
    }

    fn anti_spam_owned(&self, source: Uuid) -> bool {
        !self.agent_id.is_nil()
            && self
                .objects
                .index_of_uuid(&source)
                .and_then(|i| self.objects.get(i))
                .is_some_and(|o| o.owner_id == self.agent_id)
    }

    fn anti_spam_decision(&mut self, queue: Queue, source: Uuid, decision: Decision, newlines: bool) -> bool {
        match decision {
            Decision::Allow => false,
            Decision::Block => true,
            Decision::NewBlock => {
                self.anti_spam_notice(queue, source, newlines, Instant::now());
                true
            }
        }
    }

    /// A new block, told in the notice of its burst: the source named while
    /// it is the only one, then their count, one line edited in place while
    /// it is still in the nearby chat as written (`burst_window`).
    fn anti_spam_notice(&mut self, queue: Queue, source: Uuid, newlines: bool, now: Instant) {
        let window = burst_window(self.anti_spam.settings.period);
        let first = self.chat_clock.first;
        let line = self.anti_spam.burst.as_ref().filter(|b| joins(b.first, now, window)).and_then(|b| {
            let i = usize::try_from(b.seq.checked_sub(first)?).ok()?;
            self.chat
                .get(i)
                .filter(|l| l.kind == super::ChatKind::System && l.text == b.text)
                .map(|_| i)
        });
        let Some(i) = line else {
            let blocks = vec![(queue, source, newlines)];
            let text = self.anti_spam_text(&blocks);
            self.system_message(text.clone());
            let seq = self.chat_clock.first + self.chat.len().saturating_sub(1) as u64;
            self.anti_spam.burst = Some(Burst {
                first: now,
                seq,
                text,
                blocks,
            });
            return;
        };
        let Some(mut burst) = self.anti_spam.burst.take() else {
            return;
        };
        if !burst.blocks.iter().any(|&(q, s, _)| q == queue && s == source) {
            burst.blocks.push((queue, source, newlines));
        }
        let text = self.anti_spam_text(&burst.blocks);
        if let Some(l) = self.chat.get_mut(i) {
            l.text.clone_from(&text);
            self.chat_clock.edited();
        }
        burst.text = text;
        self.anti_spam.burst = Some(burst);
    }

    /// The notice of a burst's blocks: the source by its name while it is
    /// alone (AntiSpamBlocked, AntiSpam*NewLineFloodBlocked), else how many
    /// sources and the categories; the list and Unblock stay in
    /// Preferences > Protections > Anti-spam.
    fn anti_spam_text(&self, blocks: &[(Queue, Uuid, bool)]) -> String {
        let mut sources: Vec<Uuid> = blocks.iter().map(|&(_, s, _)| s).collect();
        sources.sort_unstable();
        sources.dedup();
        let mut kinds: Vec<&str> = Vec::new();
        for &(queue, _, _) in blocks {
            if !kinds.contains(&queue.label()) {
                kinds.push(queue.label());
            }
        }
        let kinds = kinds.join(", ");
        match (sources.as_slice(), blocks) {
            ([source], [(_, _, true)]) => aurora_i18n::tr!(
                "anti-spam-newlines-blocked",
                source = self.anti_spam_source_name(*source),
                limit = self.anti_spam.settings.newlines.clamp(5, 1000)
            ),
            ([source], _) => aurora_i18n::tr!("anti-spam-blocked", source = self.anti_spam_source_name(*source), kind = kinds),
            _ => aurora_i18n::tr!("anti-spam-blocked-burst", count = sources.len(), kinds = kinds),
        }
    }

    /// A resident by name (none under RLVa @shownames), else the id, as
    /// before; an object's name is not known here (the Legacy Viewer asks
    /// for it with RequestObjectPropertiesFamily).
    fn anti_spam_source_name(&self, id: Uuid) -> String {
        let name = if self.rlva_can_name(id) {
            self.person_name(&id)
        } else {
            self.avatar_name(&id)
        };
        name.unwrap_or_else(|| id.to_string())
    }

    /// process_improved_im counts incoming IMs, including inventory and
    /// teleport offers, on the same queue. Typing and saved text are exempt.
    pub(super) fn anti_spam_im(&mut self, im: &InstantMessage) -> bool {
        if matches!(im.dialog, 41 | 42) {
            return false;
        }
        let saved = im.offline && matches!(im.dialog, 0 | 19);
        // TASK_INVENTORY_OFFERED identifies the object's OWNER, not the
        // object (inventory_offer_handler). Honour Include my own objects
        // for those offers too. This deliberately fixes checkQueue's blanket
        // self exemption; ordinary self IMs remain exempt.
        let own_gift = im.dialog == super::notifications::im::TASK_INVENTORY_OFFERED
            && im.from_agent_id == self.agent_id
            && self.anti_spam.settings.own_objects;
        if !saved {
            let blocked = if own_gift {
                let decision = self.anti_spam.check(Queue::Im, im.from_agent_id, Instant::now());
                self.anti_spam_decision(Queue::Im, im.from_agent_id, decision, false)
            } else {
                self.anti_spam_check(Queue::Im, im.from_agent_id, false)
            };
            if blocked {
                return true;
            }
        }
        // llimprocessing.cpp applies newline protection even to saved IMs;
        // group invitations, self/system messages and known own objects are
        // exempt. An object's IM carries its object ID in session_id.
        if im.dialog == 3 || im.from_agent_id == self.agent_id || self.anti_spam_owned(im.session_id) {
            return false;
        }
        let decision = self.anti_spam.check_newlines(im.from_agent_id, &im.message);
        self.anti_spam_decision(Queue::Im, im.from_agent_id, decision, true)
    }

    /// process_chat_from_simulator: separate chat rate counters; newline
    /// protection aggregates by owner unless own objects are included.
    pub(super) fn anti_spam_chat(&mut self, chat: &ChatMessage) -> bool {
        if !self.anti_spam.settings.enabled
            || chat.source_type == ChatSourceType::System
            || matches!(chat.chat_type, ChatType::StartTyping | ChatType::StopTyping | ChatType::Debug)
            || chat.source_id == self.agent_id
        {
            return false;
        }
        let object = chat.source_type == ChatSourceType::Object;
        let rate_source = if object { chat.source_id } else { chat.owner_id };
        if self.anti_spam_check(Queue::Chat, rate_source, object) {
            return true;
        }
        if chat.owner_id.is_nil()
            || (!self.anti_spam.settings.own_objects && (chat.owner_id == self.agent_id || self.anti_spam_owned(chat.source_id)))
        {
            return false;
        }
        let source = if self.anti_spam.settings.own_objects {
            chat.source_id
        } else {
            chat.owner_id
        };
        let decision = self.anti_spam.check_newlines(source, &chat.message);
        self.anti_spam_decision(Queue::Chat, source, decision, true)
    }

    /// process_sound_trigger / process_attached_sound / process_preload_sound:
    /// count both object and owner, allowing the owned-object exemption.
    pub fn anti_spam_sound(&mut self, queue: Queue, object: Uuid, owner: Uuid) -> bool {
        self.anti_spam_check(queue, object, true) || (owner != object && self.anti_spam_check(queue, owner, false))
    }
}
