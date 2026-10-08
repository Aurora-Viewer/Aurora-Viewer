//! Spatial state (listener / avatar) and the participant table.
//!
//! Ported from Firestorm's `indra/newview/llvoicewebrtc.cpp`
//! (`setAvatarPosition`, `setListenerPosition`, `enforceTether`,
//! `sendPositionUpdate`, `OnDataReceivedImpl`).
//! Copyright (C) Linden Research, Inc. and The Phoenix Firestorm Project.
//! Licensed under the GNU Lesser General Public License, version 2.1.

use std::collections::{HashMap, HashSet};

use glam::{DVec3, Quat};
use uuid::Uuid;

use crate::protocol::{self, JOIN_GAIN_CONVERSION_FACTOR, MAX_AUDIO_DIST, ParticipantUpdate, gain_value};

/// `MINUSCULE_ANGLE_COS = cos(0.5 * FOUR_DEGREES)`.
fn minuscule_angle_cos() -> f32 {
    (0.5f32 * 4.0f32.to_radians()).cos()
}

/// Avatar / listener coordinates sent on the data channel.
#[derive(Debug, Clone)]
pub(crate) struct SpatialState {
    avatar_pos: DVec3,
    avatar_rot: Quat,
    listener_requested: DVec3,
    listener_pos: DVec3,
    listener_rot: Quat,
    dirty: bool,
    known: bool,
}

impl Default for SpatialState {
    fn default() -> Self {
        Self {
            avatar_pos: DVec3::ZERO,
            avatar_rot: Quat::IDENTITY,
            listener_requested: DVec3::ZERO,
            listener_pos: DVec3::ZERO,
            listener_rot: Quat::IDENTITY,
            dirty: false,
            known: false,
        }
    }
}

impl SpatialState {
    /// `updatePosition`: listener first, then the avatar (raised to head
    /// height by 1 m), then the 50 m tether.
    pub(crate) fn update(&mut self, avatar_pos: DVec3, avatar_rot: Quat, listener_pos: DVec3, listener_rot: Quat) {
        if !(avatar_pos.is_finite() && listener_pos.is_finite() && avatar_rot.is_finite() && listener_rot.is_finite()) {
            return;
        }
        self.known = true;
        self.set_listener(listener_pos, listener_rot);
        self.set_avatar(avatar_pos + DVec3::new(0.0, 0.0, 1.0), avatar_rot);
        self.enforce_tether();
    }

    fn set_listener(&mut self, pos: DVec3, rot: Quat) {
        self.listener_requested = pos;
        if self.listener_rot != rot {
            self.listener_rot = rot;
            self.dirty = true;
        }
    }

    fn set_avatar(&mut self, pos: DVec3, rot: Quat) {
        if self.avatar_pos.distance_squared(pos) > 0.01 {
            self.avatar_pos = pos;
            self.dirty = true;
        }
        let rot_cos_diff = self.avatar_rot.dot(rot).abs();
        if self.avatar_rot != rot && rot_cos_diff < minuscule_angle_cos() {
            self.avatar_rot = rot;
            self.dirty = true;
        }
    }

    fn enforce_tether(&mut self) {
        let mut tethered = self.listener_requested;
        let offset = self.listener_requested - self.avatar_pos;
        let distance = offset.length();
        if distance > MAX_AUDIO_DIST {
            tethered = self.avatar_pos + offset * (MAX_AUDIO_DIST / distance);
        }
        if self.listener_pos.distance_squared(tethered) > 0.01 {
            self.listener_pos = tethered;
            self.dirty = true;
        }
    }

    /// `sendPositionUpdate(force)`: the JSON to send, if anything changed.
    pub(crate) fn take_message(&mut self, force: bool) -> Option<String> {
        if !self.known || !(self.dirty || force) {
            return None;
        }
        self.dirty = false;
        Some(protocol::position_message(
            self.avatar_pos,
            self.avatar_rot,
            self.listener_pos,
            self.listener_rot,
        ))
    }

    #[cfg(test)]
    fn listener(&self) -> DVec3 {
        self.listener_pos
    }
}

/// A voice participant as seen on the data channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Participant {
    pub agent_id: Uuid,
    /// Audio power 0..~1 (`"p" / 128`).
    pub level: f32,
    /// Voice activity reported by the server.
    pub speaking: bool,
    /// Muted by a moderator.
    pub moderator_muted: bool,
}

/// Per-user preferences applied when a participant joins.
#[derive(Debug, Default, Clone)]
pub(crate) struct LocalPrefs {
    pub mutes: HashSet<Uuid>,
    /// Volume 0..=1 (Firestorm's speaker volume storage).
    pub volumes: HashMap<Uuid, f32>,
}

#[derive(Debug, Default)]
pub(crate) struct ParticipantTable {
    map: HashMap<Uuid, Participant>,
}

impl ParticipantTable {
    pub(crate) fn clear(&mut self) {
        self.map.clear();
    }

    pub(crate) fn list(&self) -> Vec<Participant> {
        let mut v: Vec<Participant> = self.map.values().cloned().collect();
        v.sort_by_key(|p| p.agent_id);
        v
    }

    /// Applies updates like `OnDataReceivedImpl`; returns the mute / gain
    /// message to send back for newly joined participants, if any.
    pub(crate) fn apply(&mut self, updates: &[ParticipantUpdate], prefs: &LocalPrefs, spatial: bool, self_id: Uuid) -> Option<String> {
        let mut mutes = Vec::new();
        let mut gains = Vec::new();
        for up in updates {
            let id = up.agent_id;
            let mut primary = false;
            if let Some(p) = up.joined {
                primary = p;
                if prefs.mutes.contains(&id) {
                    mutes.push(id);
                }
                if let Some(&vol) = prefs.volumes.get(&id) {
                    gains.push((id, gain_value(vol, JOIN_GAIN_CONVERSION_FACTOR)));
                }
            }
            // Ignore joins reported by non-primary servers in spatial
            // (cross-region) voice, to avoid duplicates.
            if !self.map.contains_key(&id) && up.joined.is_some() && (primary || !spatial) {
                self.map.insert(
                    id,
                    Participant {
                        agent_id: id,
                        level: 0.0,
                        speaking: false,
                        moderator_muted: false,
                    },
                );
            }
            if up.left {
                if id != self_id {
                    self.map.remove(&id);
                }
                continue;
            }
            if let Some(p) = self.map.get_mut(&id) {
                if let Some(level) = up.level {
                    p.level = level;
                }
                if let Some(v) = up.speaking {
                    p.speaking = v;
                }
                if let Some(m) = up.moderator_muted {
                    p.moderator_muted = m;
                }
            }
        }
        protocol::mute_gain_reply(&mutes, &gains)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::parse_data_message;

    #[test]
    fn spatial_dirty_tracking_and_tether() {
        let mut s = SpatialState::default();
        assert_eq!(s.take_message(true), None, "nothing before the first update");
        let avatar = DVec3::new(1000.0, 2000.0, 30.0);
        s.update(avatar, Quat::IDENTITY, avatar, Quat::IDENTITY);
        let first = s.take_message(false).unwrap();
        assert!(first.contains(r#""sp":{"x":100000,"y":200000,"z":3100}"#), "{first}");
        // Same input -> nothing new unless forced.
        s.update(avatar, Quat::IDENTITY, avatar, Quat::IDENTITY);
        assert_eq!(s.take_message(false), None);
        assert!(s.take_message(true).is_some());
        // Sub-10 cm moves are ignored.
        s.update(avatar + DVec3::new(0.05, 0.0, 0.0), Quat::IDENTITY, avatar, Quat::IDENTITY);
        assert_eq!(s.take_message(false), None);
        // Tiny rotation (< 2 degrees) is ignored; a big one is not.
        s.update(avatar, Quat::from_rotation_z(0.01), avatar, Quat::IDENTITY);
        assert_eq!(s.take_message(false), None);
        s.update(avatar, Quat::from_rotation_z(0.5), avatar, Quat::IDENTITY);
        assert!(s.take_message(false).is_some());
        // Camera 200 m away is pulled back to 50 m from the (head-height) avatar.
        let far = avatar + DVec3::new(200.0, 0.0, 1.0);
        s.update(avatar, Quat::from_rotation_z(0.5), far, Quat::IDENTITY);
        let head = avatar + DVec3::new(0.0, 0.0, 1.0);
        assert!((s.listener().distance(head) - 50.0).abs() < 1e-6);
        assert!(s.take_message(false).is_some());
        // Non-finite input is rejected.
        s.update(DVec3::NAN, Quat::IDENTITY, avatar, Quat::IDENTITY);
        assert_eq!(s.take_message(false), None);
    }

    #[test]
    fn participants_join_update_leave() {
        let me = Uuid::from_u128(1);
        let a = Uuid::from_u128(2);
        let b = Uuid::from_u128(3);
        let mut prefs = LocalPrefs::default();
        prefs.mutes.insert(b);
        prefs.volumes.insert(a, 0.5);
        let mut t = ParticipantTable::default();

        let msg = format!(r#"{{"{me}":{{"j":{{"p":true}}}},"{a}":{{"j":{{"p":true}},"p":128,"v":true}},"{b}":{{"j":{{}}}}}}"#);
        let reply = t.apply(&parse_data_message(&msg).unwrap(), &prefs, true, me).unwrap();
        // b joined as non-primary on a spatial channel: not listed, but the
        // mute is still sent (as Firestorm does).
        assert_eq!(reply, format!(r#"{{"m":{{"{b}":true}},"ug":{{"{a}":100}}}}"#));
        let list = t.list();
        assert_eq!(list.len(), 2);
        let pa = list.iter().find(|p| p.agent_id == a).unwrap();
        assert!(pa.speaking);
        assert_eq!(pa.level, 1.0);

        // Updates for unknown participants are ignored.
        let msg = format!(r#"{{"{b}":{{"v":true}}}}"#);
        assert_eq!(t.apply(&parse_data_message(&msg).unwrap(), &prefs, true, me), None);
        assert_eq!(t.list().len(), 2);

        // Leaves remove others but never ourselves.
        let msg = format!(r#"{{"{a}":{{"l":true}},"{me}":{{"l":true}}}}"#);
        t.apply(&parse_data_message(&msg).unwrap(), &prefs, true, me);
        let list = t.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].agent_id, me);

        // Ad-hoc channels accept non-primary joins.
        let mut t = ParticipantTable::default();
        let msg = format!(r#"{{"{b}":{{"j":{{}},"m":true}}}}"#);
        t.apply(&parse_data_message(&msg).unwrap(), &LocalPrefs::default(), false, me);
        assert!(t.list()[0].moderator_muted);
    }
}
