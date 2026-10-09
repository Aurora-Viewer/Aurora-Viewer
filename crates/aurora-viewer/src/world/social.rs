//! Friends, name cache and instant-message sessions.

use super::ChatLine;
use super::names::AvatarNames;
use aurora_llsd::Llsd;
use std::collections::HashMap;
use uuid::Uuid;

/// LLRelationship rights bits.
pub mod rights {
    pub const ONLINE_STATUS: i32 = 1;
    pub const MAP_LOCATION: i32 = 2;
    pub const MODIFY_OBJECTS: i32 = 4;
}

#[derive(Debug, Clone)]
pub struct Friend {
    pub id: Uuid,
    pub online: bool,
    /// Rights we granted (1 online status, 2 map, 4 modify objects).
    pub rights_given: i32,
    pub rights_has: i32,
}

#[derive(Debug, Clone)]
pub struct ImSession {
    pub other: Uuid,
    pub lines: Vec<ChatLine>,
    pub unread: usize,
    #[allow(dead_code)] // draft of an IM session, not kept by the UI yet
    pub input: String,
}

#[derive(Default)]
pub struct Social {
    pub friends: Vec<Friend>,
    /// Legacy names ("First Last", UUIDNameReply / IM headers).
    pub names: HashMap<Uuid, String>,
    /// Display names (LLAvatarNameCache).
    pub avatar_names: AvatarNames,
    /// Online / offline notices waiting for the friend's name (id, online, since).
    online_notices: Vec<(Uuid, bool, std::time::Instant)>,
    pub ims: Vec<ImSession>,
    /// IM session to focus in the conversations window.
    pub focus_im: Option<Uuid>,
}

pub fn format_name(first: &str, last: &str) -> String {
    if last.is_empty() || last.eq_ignore_ascii_case("resident") {
        first.to_owned()
    } else {
        format!("{first} {last}")
    }
}

impl Social {
    pub fn from_login(raw: &Llsd) -> Social {
        let mut s = Social::default();
        for b in raw["buddy-list"].as_array() {
            let id = b["buddy_id"].as_uuid();
            if id.is_nil() {
                continue;
            }
            s.friends.push(Friend {
                id,
                online: false,
                rights_given: b["buddy_rights_given"].as_i32(),
                rights_has: b["buddy_rights_has"].as_i32(),
            });
            s.want_name(id);
        }
        s
    }

    /// Queue a name lookup (missing or expired name).
    pub fn want_name(&mut self, id: Uuid) {
        self.avatar_names.want(&id);
    }

    /// Complete name ("Jane Doe (janedoe)"), else the legacy name.
    pub fn name_of(&self, id: &Uuid) -> String {
        self.avatar_names
            .complete(id)
            .or_else(|| self.names.get(id).cloned())
            .unwrap_or_else(|| "…".into())
    }

    /// Friends going on / off line; their notices wait for the names.
    pub fn set_online(&mut self, ids: &[Uuid], online: bool) {
        let now = std::time::Instant::now();
        for f in self.friends.iter_mut() {
            if ids.contains(&f.id) && f.online != online {
                f.online = online;
                self.online_notices.push((f.id, online, now));
            }
        }
    }

    /// Online / offline notices whose name is known (or after 10 s without).
    pub fn take_online_notices(&mut self) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.online_notices.len() {
            let (id, online, since) = self.online_notices[i];
            let known = self.avatar_names.get(&id).is_some() || self.names.contains_key(&id);
            if known || since.elapsed().as_secs() >= 10 {
                self.online_notices.remove(i);
                let n = self.name_of(&id);
                out.push((
                    if online {
                        format!("{n} est en ligne.")
                    } else {
                        format!("{n} est hors ligne.")
                    },
                    online,
                ));
            } else {
                i += 1;
            }
        }
        out
    }

    pub fn is_friend(&self, id: &Uuid) -> bool {
        self.friends.iter().any(|f| f.id == *id)
    }

    /// LLAvatarTracker::formFriendship: a new friend sees the other online
    /// both ways until rights change.
    pub fn form_friendship(&mut self, id: Uuid) {
        if id.is_nil() || self.is_friend(&id) {
            return;
        }
        self.friends.push(Friend {
            id,
            online: false,
            rights_given: rights::ONLINE_STATUS,
            rights_has: rights::ONLINE_STATUS,
        });
        self.want_name(id);
    }

    /// The friendship ended (terminateBuddy / processTerminateFriendship).
    pub fn end_friendship(&mut self, id: &Uuid) -> bool {
        let before = self.friends.len();
        self.friends.retain(|f| f.id != *id);
        self.friends.len() != before
    }

    /// A conversation with this avatar exists.
    pub fn has_session(&self, other: &Uuid) -> bool {
        self.ims.iter().any(|s| s.other == *other)
    }

    pub fn session_mut(&mut self, other: Uuid) -> &mut ImSession {
        if let Some(i) = self.ims.iter().position(|s| s.other == other) {
            return &mut self.ims[i];
        }
        self.want_name(other);
        self.ims.push(ImSession {
            other,
            lines: Vec::new(),
            unread: 0,
            input: String::new(),
        });
        let n = self.ims.len() - 1;
        &mut self.ims[n]
    }

    pub fn total_unread(&self) -> usize {
        self.ims.iter().map(|s| s.unread).sum()
    }

    /// Friends sorted: online first, then by name.
    pub fn sorted_friends(&self) -> Vec<Friend> {
        let mut v = self.friends.clone();
        v.sort_by(|a, b| {
            b.online
                .cmp(&a.online)
                .then_with(|| self.name_of(&a.id).to_lowercase().cmp(&self.name_of(&b.id).to_lowercase()))
        });
        v
    }
}
