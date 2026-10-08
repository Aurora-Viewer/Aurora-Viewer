//! Block (mute) list, ported from Firestorm's LLMuteList (llmutelist.cpp,
//! originally LGPL 2.1): agents, objects and groups blocked by id, legacy blocks by name
//! (also used for group chat, exoGroupMuteList's "Group:<id>"), per-entry
//! flags, the server copy (MuteListRequest / UpdateMuteListEntry /
//! RemoveMuteListEntry) and the `<agent>.cached_mute` cache file.

use aurora_net::NetCommand;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

/// LLMute::EType.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteType {
    ByName = 0,
    Agent = 1,
    Object = 2,
    Group = 3,
    External = 4,
}

impl MuteType {
    fn from_i32(v: i32) -> MuteType {
        match v {
            1 => MuteType::Agent,
            2 => MuteType::Object,
            3 => MuteType::Group,
            4 => MuteType::External,
            _ => MuteType::ByName,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MuteType::ByName => "Par nom",
            MuteType::Agent => "Résident",
            MuteType::Object => "Objet",
            MuteType::Group => "Groupe",
            MuteType::External => "Externe",
        }
    }
}

/// Entry flags: a set bit means this property is NOT muted (inverted so
/// that old entries without flags mute everything). LL's comment says the
/// object sounds bit is the other way round, but isMuted treats it alike.
pub mod flag {
    pub const TEXT_CHAT: u32 = 0x1;
    pub const VOICE_CHAT: u32 = 0x2;
    pub const PARTICLES: u32 = 0x4;
    pub const OBJECT_SOUNDS: u32 = 0x8;
    pub const ALL: u32 = 0xF;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mute {
    pub id: Uuid,
    /// Avatar or object name ("Resident" last name left out).
    pub name: String,
    pub kind: MuteType,
    pub flags: u32,
}

/// Load state (LLMuteList::EMuteListState / EMuteListSource).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadState {
    Initial,
    Requested(Instant),
    /// Loaded from the server (or the cache it told us to use).
    Loaded,
    /// Only our cached copy (server silent): never written back.
    Degraded,
}

/// MuteListLimit.
pub const MUTE_LIMIT: usize = 1000;
/// LLMuteList::updateLoadState: wait this long for the server.
const REQUEST_TIMEOUT_SECS: u64 = 30;

#[derive(Debug)]
pub struct MuteList {
    mutes: BTreeMap<Uuid, Mute>,
    legacy: BTreeSet<String>,
    pub state: LoadState,
    /// Bumped on every change (cheap "did it change" checks).
    pub version: u64,
    agent: Uuid,
}

impl Default for MuteList {
    fn default() -> Self {
        MuteList {
            mutes: BTreeMap::new(),
            legacy: BTreeSet::new(),
            state: LoadState::Initial,
            version: 0,
            agent: Uuid::nil(),
        }
    }
}

/// Why an entry could not be added (LLMuteList::add notifications).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    Linden,
    Myself,
    Limit,
    Empty,
    Duplicate,
}

impl AddError {
    pub fn message(&self) -> String {
        match self {
            AddError::Linden => "Impossible de bloquer le chat d'un Linden.".into(),
            AddError::Myself => "Vous ne pouvez pas vous bloquer vous-même.".into(),
            AddError::Limit => format!("Liste de blocage pleine ({MUTE_LIMIT} entrées)."),
            AddError::Empty => "Nom vide.".into(),
            AddError::Duplicate => "Déjà dans la liste de blocage.".into(),
        }
    }
}

/// Group chat blocks are legacy by-name entries (exoGroupMuteList).
pub fn group_chat_name(group: &Uuid) -> String {
    format!("Group:{group}")
}

/// "First Linden" / "first.linden" (LLMuteList::isLinden).
pub fn is_linden(name: &str) -> bool {
    let n = name.replace('.', " ");
    let mut it = n.split_whitespace();
    it.next();
    it.next().is_some_and(|last| last.eq_ignore_ascii_case("linden"))
}

impl MuteList {
    pub fn reset(&mut self, agent: Uuid) {
        *self = MuteList {
            agent,
            ..Default::default()
        };
    }

    pub fn is_loaded(&self) -> bool {
        matches!(self.state, LoadState::Loaded | LoadState::Degraded)
    }

    /// `<cache>/<agent>.cached_mute` (LLMuteList::getCacheFilename).
    pub fn cache_file(cache_dir: &Path, agent: &Uuid) -> PathBuf {
        cache_dir.join(format!("{agent}.cached_mute"))
    }

    /// MuteListRequest with the CRC of our cached copy.
    pub fn request(&mut self, cache_dir: Option<&Path>) -> NetCommand {
        let crc = cache_dir
            .and_then(|d| std::fs::read(Self::cache_file(d, &self.agent)).ok())
            .map(|b| aurora_net::social::ll_crc(&b))
            .unwrap_or(0);
        self.state = LoadState::Requested(Instant::now());
        NetCommand::RequestMuteList { crc }
    }

    /// The server did not answer in time: our cached copy, if any
    /// (LLMuteList::tryLoadCacheFallback). True when the state changed.
    pub fn check_timeout(&mut self, cache_dir: Option<&Path>) -> bool {
        let LoadState::Requested(t) = self.state else {
            return false;
        };
        if t.elapsed().as_secs() < REQUEST_TIMEOUT_SECS {
            return false;
        }
        log::warn!("mute list request timed out; using the cached copy");
        let text = cache_dir.and_then(|d| std::fs::read_to_string(Self::cache_file(d, &self.agent)).ok());
        self.load_text(text.as_deref().unwrap_or(""));
        self.state = LoadState::Degraded;
        true
    }

    /// From the server (file or "use your cache").
    pub fn on_server(&mut self, src: aurora_net::MuteListSource, cache_dir: Option<&Path>) {
        let text = match src {
            aurora_net::MuteListSource::File(t) => t,
            aurora_net::MuteListSource::UseCache => cache_dir
                .and_then(|d| std::fs::read_to_string(Self::cache_file(d, &self.agent)).ok())
                .unwrap_or_default(),
        };
        // entries added before the list arrived stay (they were sent already)
        let pending: Vec<Mute> = if self.is_loaded() {
            Vec::new()
        } else {
            self.mutes.values().cloned().collect()
        };
        self.load_text(&text);
        for m in pending {
            self.mutes.entry(m.id).or_insert(m);
        }
        self.state = LoadState::Loaded;
        log::info!("mute list: {} entries, {} by name", self.mutes.len(), self.legacy.len());
        self.save(cache_dir);
    }

    /// LLMuteList::loadFromFile: lines " %d %254s %254[^|]| %u".
    fn load_text(&mut self, text: &str) {
        self.mutes.clear();
        self.legacy.clear();
        for line in text.lines() {
            if let Some(m) = parse_line(line) {
                if m.id.is_nil() || m.kind == MuteType::ByName {
                    if !m.name.is_empty() {
                        self.legacy.insert(m.name);
                    }
                } else {
                    self.mutes.insert(m.id, m);
                }
            }
        }
        self.version += 1;
    }

    /// LLMuteList::saveToFile (never from the degraded state, like cache()).
    pub fn save(&self, cache_dir: Option<&Path>) {
        if self.state != LoadState::Loaded || self.agent.is_nil() {
            return;
        }
        let Some(dir) = cache_dir else {
            return;
        };
        let _ = std::fs::create_dir_all(dir);
        if let Err(e) = std::fs::write(Self::cache_file(dir, &self.agent), self.to_text()) {
            log::warn!("mute list cache: {e}");
        }
    }

    fn to_text(&self) -> String {
        let mut s = String::new();
        for n in &self.legacy {
            s.push_str(&format!("{} {} {}|\n", MuteType::ByName as i32, Uuid::nil(), n));
        }
        for m in self.mutes.values().filter(|m| m.kind != MuteType::External) {
            s.push_str(&format!("{} {} {}|{}\n", m.kind as i32, m.id, m.name, m.flags));
        }
        s
    }

    /// LLMuteList::isMuted(id, name, flags): `flags` = the properties the
    /// caller cares about (0 = any). Legacy entries match by name.
    pub fn is_muted(&self, id: &Uuid, name: &str, flags: u32) -> bool {
        if *id == self.agent && !id.is_nil() {
            return false;
        }
        if let Some(m) = self.mutes.get(id) {
            // a set flag means this property is not muted
            return flags & m.flags == 0;
        }
        !name.is_empty() && self.legacy.contains(name)
    }

    pub fn is_muted_id(&self, id: &Uuid) -> bool {
        self.is_muted(id, "", 0)
    }

    /// Text chat / IM blocked.
    pub fn text_muted(&self, id: &Uuid, name: &str) -> bool {
        self.is_muted(id, name, flag::TEXT_CHAT)
    }

    /// Object sounds of an owner / object (process_sound_trigger: plain
    /// isMuted with flagObjectSounds).
    pub fn sounds_muted(&self, id: &Uuid) -> bool {
        self.is_muted(id, "", flag::OBJECT_SOUNDS)
    }

    pub fn group_chat_muted(&self, group: &Uuid) -> bool {
        self.legacy.contains(&group_chat_name(group))
    }

    /// Residents with an entry (any flags: LLVOAvatar::isInMuteList).
    pub fn blocked_ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.mutes.values().filter(|m| m.kind == MuteType::Agent).map(|m| m.id)
    }

    pub fn get(&self, id: &Uuid) -> Option<&Mute> {
        self.mutes.get(id)
    }

    pub fn len(&self) -> usize {
        self.mutes.len() + self.legacy.len()
    }

    /// Entries sorted by name (LLMuteList::getMutes), group chat blocks left out.
    pub fn entries(&self) -> Vec<Mute> {
        let mut v: Vec<Mute> = self.mutes.values().cloned().collect();
        v.extend(self.legacy.iter().filter(|n| !n.starts_with("Group:")).map(|n| Mute {
            id: Uuid::nil(),
            name: n.clone(),
            kind: MuteType::ByName,
            flags: 0,
        }));
        v.sort_by_key(|m| m.name.to_uppercase());
        v
    }

    /// LLMuteList::add: `flags` = properties to mute (0 = all). Returns the
    /// message for the server.
    pub fn add(&mut self, mute: Mute, flags: u32) -> Result<NetCommand, AddError> {
        if mute.kind == MuteType::Agent && is_linden(&mute.name) && (flags & flag::TEXT_CHAT != 0 || flags == 0) {
            return Err(AddError::Linden);
        }
        if mute.kind == MuteType::Agent && mute.id == self.agent {
            return Err(AddError::Myself);
        }
        if self.len() >= MUTE_LIMIT {
            return Err(AddError::Limit);
        }
        if mute.kind == MuteType::ByName {
            if mute.name.is_empty() {
                return Err(AddError::Empty);
            }
            if !self.legacy.insert(mute.name.clone()) {
                return Err(AddError::Duplicate);
            }
            self.changed();
            return Ok(NetCommand::UpdateMute {
                id: Uuid::nil(),
                name: mute.name,
                kind: 0,
                flags: 0,
            });
        }
        // an existing entry keeps its flags, a new one starts "nothing muted"
        let mut m = mute;
        m.flags = self.mutes.get(&m.id).map(|e| e.flags).unwrap_or(flag::ALL);
        if let (true, Some(e)) = (m.name.is_empty(), self.mutes.get(&m.id)) {
            m.name = e.name.clone();
        }
        m.flags = if flags != 0 { m.flags & !flags } else { 0 };
        let cmd = NetCommand::UpdateMute {
            id: m.id,
            name: m.name.clone(),
            kind: m.kind as i32,
            flags: m.flags,
        };
        self.mutes.insert(m.id, m);
        self.changed();
        Ok(cmd)
    }

    /// LLMuteList::remove: `flags` = properties to unmute (0 = the whole
    /// entry). Legacy entries are matched by name.
    pub fn remove(&mut self, id: &Uuid, name: &str, flags: u32) -> Option<NetCommand> {
        if let Some(mut m) = self.mutes.remove(id) {
            let cmd = if flags != 0 {
                m.flags |= flags;
                if m.flags == flag::ALL {
                    NetCommand::RemoveMute {
                        id: m.id,
                        name: m.name.clone(),
                    }
                } else {
                    let c = NetCommand::UpdateMute {
                        id: m.id,
                        name: m.name.clone(),
                        kind: m.kind as i32,
                        flags: m.flags,
                    };
                    self.mutes.insert(m.id, m);
                    c
                }
            } else {
                NetCommand::RemoveMute {
                    id: m.id,
                    name: m.name.clone(),
                }
            };
            self.changed();
            return Some(cmd);
        }
        if self.legacy.remove(name) {
            self.changed();
            return Some(NetCommand::RemoveMute {
                id: Uuid::nil(),
                name: name.to_owned(),
            });
        }
        None
    }

    fn changed(&mut self) {
        self.version += 1;
        // LLMuteList::updateAdd: the server may never answer when it has no
        // file, so any change makes the list "loaded"
        if !self.is_loaded() {
            self.state = LoadState::Loaded;
        }
    }
}

fn parse_line(line: &str) -> Option<Mute> {
    let line = line.trim_start();
    let (kind, rest) = line.split_once(char::is_whitespace)?;
    let kind: i32 = kind.parse().ok()?;
    let rest = rest.trim_start();
    let (id, rest) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let id = Uuid::parse_str(id).unwrap_or(Uuid::nil());
    let rest = rest.trim_start();
    let (name, flags) = match rest.split_once('|') {
        Some((n, f)) => (n, f.trim().parse::<u32>().unwrap_or(0)),
        None => (rest.trim_end(), 0),
    };
    Some(Mute {
        id,
        name: name.to_owned(),
        kind: MuteType::from_i32(kind),
        flags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> MuteList {
        let mut m = MuteList::default();
        m.reset(Uuid::from_u128(99));
        m
    }

    #[test]
    fn file_round_trip() {
        let a = Uuid::from_u128(1);
        let text = format!(
            "0 {} Spammer Bob|\n1 {a} Jane Doe|0\n2 {} Noisy Box|3\n",
            Uuid::nil(),
            Uuid::from_u128(2)
        );
        let mut m = list();
        m.load_text(&text);
        assert!(m.is_muted(&Uuid::nil(), "Spammer Bob", 0));
        assert!(m.text_muted(&a, ""));
        // object 2 has text + voice allowed
        assert!(!m.text_muted(&Uuid::from_u128(2), ""));
        assert!(m.is_muted(&Uuid::from_u128(2), "", flag::PARTICLES));
        let mut again = list();
        again.load_text(&m.to_text());
        assert_eq!(again.entries(), m.entries());
    }

    #[test]
    fn add_and_remove_flags() {
        let a = Uuid::from_u128(1);
        let mut m = list();
        let cmd = m
            .add(
                Mute {
                    id: a,
                    name: "Jane".into(),
                    kind: MuteType::Agent,
                    flags: 0,
                },
                flag::VOICE_CHAT,
            )
            .unwrap();
        assert!(matches!(cmd, NetCommand::UpdateMute { flags, .. } if flags == flag::ALL & !flag::VOICE_CHAT));
        assert!(m.is_muted(&a, "", flag::VOICE_CHAT));
        assert!(!m.text_muted(&a, ""));
        m.add(
            Mute {
                id: a,
                name: String::new(),
                kind: MuteType::Agent,
                flags: 0,
            },
            0,
        )
        .unwrap();
        assert!(m.text_muted(&a, ""));
        assert_eq!(m.get(&a).unwrap().name, "Jane");
        // unmuting voice only keeps the entry
        assert!(matches!(m.remove(&a, "", flag::VOICE_CHAT), Some(NetCommand::UpdateMute { .. })));
        assert!(m.text_muted(&a, ""));
        assert!(matches!(m.remove(&a, "", 0), Some(NetCommand::RemoveMute { .. })));
        assert!(!m.is_muted_id(&a));
    }

    #[test]
    fn refusals() {
        let mut m = list();
        let me = Uuid::from_u128(99);
        assert_eq!(
            m.add(
                Mute {
                    id: me,
                    name: "Me".into(),
                    kind: MuteType::Agent,
                    flags: 0
                },
                0
            )
            .err(),
            Some(AddError::Myself)
        );
        let l = Uuid::from_u128(5);
        assert_eq!(
            m.add(
                Mute {
                    id: l,
                    name: "Patch Linden".into(),
                    kind: MuteType::Agent,
                    flags: 0
                },
                0
            )
            .err(),
            Some(AddError::Linden)
        );
        assert!(
            m.add(
                Mute {
                    id: l,
                    name: "Patch Linden".into(),
                    kind: MuteType::Agent,
                    flags: 0
                },
                flag::VOICE_CHAT
            )
            .is_ok()
        );
        let g = Uuid::from_u128(7);
        m.add(
            Mute {
                id: Uuid::nil(),
                name: group_chat_name(&g),
                kind: MuteType::ByName,
                flags: 0,
            },
            0,
        )
        .unwrap();
        assert!(m.group_chat_muted(&g));
        assert!(m.entries().iter().all(|e| !e.name.starts_with("Group:")));
    }

    #[test]
    fn sounds_follow_is_muted() {
        let a = Uuid::from_u128(1);
        let mut m = list();
        m.add(
            Mute {
                id: a,
                name: "Box".into(),
                kind: MuteType::Object,
                flags: 0,
            },
            0,
        )
        .unwrap();
        assert!(m.sounds_muted(&a));
        let b = Uuid::from_u128(2);
        m.add(
            Mute {
                id: b,
                name: "Bob".into(),
                kind: MuteType::Agent,
                flags: 0,
            },
            flag::TEXT_CHAT,
        )
        .unwrap();
        assert!(!m.sounds_muted(&b));
    }
}
