//! Bumps, pushes and hits: the residents the simulator names in
//! MeanCollisionAlert, kept for the "Bumps, pushes & hits" window
//! (`ui/bumps.rs`), and the Legacy Viewer's two Preferences › Alerts
//! options, a line in nearby chat and a report to the agent's scripts.
//! Port of process_mean_collision_alert_message and mean_name_callback
//! (indra/newview/llviewermessage.cpp) and LLFloaterBump's list
//! (llfloaterbump.cpp), originally LGPL 2.1.

use super::World;
use aurora_net::{MeanCollision, NetCommand};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use uuid::Uuid;

/// The rows kept (max_collision_list_size, and the floater's 20 rows).
pub const MAX: usize = 20;

/// The channel scripts listen on (FSReportCollisionMessagesChannel).
pub const DEFAULT_CHANNEL: i32 = -25_000;

/// EMeanCollisionType (indra/llmessage/mean_collision_data.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bump,
    /// llPushObject.
    Push,
    /// An object the resident was editing.
    SelectedObject,
    ScriptedObject,
    PhysicalObject,
}

impl Kind {
    /// None for MEAN_INVALID and the types after MEAN_EOF, which the
    /// floater leaves out.
    pub fn from_wire(v: u8) -> Option<Kind> {
        Some(match v {
            1 => Kind::Bump,
            2 => Kind::Push,
            3 => Kind::SelectedObject,
            4 => Kind::ScriptedObject,
            5 => Kind::PhysicalObject,
            _ => return None,
        })
    }

    /// "<name> bumped you." and the others (strings.xml Collision_*); the
    /// name is a link in nearby chat, plain text in the window.
    pub fn sentence(self, name: &str) -> String {
        let name = name.to_owned();
        match self {
            Kind::Bump => aurora_i18n::tr!("world-collisions-bump", name = name),
            Kind::Push => aurora_i18n::tr!("world-collisions-push", name = name),
            Kind::SelectedObject => aurora_i18n::tr!("world-collisions-selected-object", name = name),
            Kind::ScriptedObject => aurora_i18n::tr!("world-collisions-scripted-object", name = name),
            Kind::PhysicalObject => aurora_i18n::tr!("world-collisions-physical-object", name = name),
        }
    }
}

/// One row of the list: the last collision of a resident of this kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Collision {
    pub perp: Uuid,
    pub kind: Kind,
    /// Unix time from the simulator.
    pub time: u32,
    pub magnitude: f32,
}

/// Preferences › Interface › Bumps, pushes & hits (the Legacy Viewer's
/// defaults: both off).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// FSCollisionMessagesInChat: "<name> bumped you." in nearby chat.
    pub in_chat: bool,
    /// FSReportCollisionMessages: `<key>,<magnitude>,<type>` sent to the
    /// agent's scripts on `channel` (FSReportCollisionMessagesChannel).
    pub report: bool,
    pub channel: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            in_chat: false,
            report: false,
            channel: DEFAULT_CHANNEL,
        }
    }
}

#[derive(Debug, Default)]
pub struct Collisions {
    pub cfg: Settings,
    /// Newest first.
    list: VecDeque<Collision>,
}

impl Collisions {
    pub fn list(&self) -> impl Iterator<Item = &Collision> {
        self.list.iter()
    }

    /// A resident already listed for this kind gets the new time and
    /// magnitude. The Legacy Viewer leaves that row where it was; it moves
    /// to the top here, so the list stays newest first.
    fn record(&mut self, c: Collision) {
        self.list.retain(|o| (o.perp, o.kind) != (c.perp, c.kind));
        self.list.push_front(c);
        self.list.truncate(MAX);
    }
}

/// The text scripts receive (llformat "%s,%.2f,%d").
pub fn report_text(c: &MeanCollision) -> String {
    format!("{},{:.2},{}", c.perp, c.magnitude, c.kind)
}

/// is_prelude (indra/llmessage/llregionflags.h): RESET_HOME_ON_TELEPORT set,
/// ALLOW_LANDMARK and ALLOW_SET_HOME unset.
fn is_prelude(region_flags: u32) -> bool {
    const ALLOW_LANDMARK: u32 = 1 << 1;
    const ALLOW_SET_HOME: u32 = 1 << 2;
    const RESET_HOME_ON_TELEPORT: u32 = 1 << 3;
    region_flags & (ALLOW_LANDMARK | ALLOW_SET_HOME) == 0 && region_flags & RESET_HOME_ON_TELEPORT != 0
}

impl World {
    pub(super) fn on_mean_collisions(&mut self, list: Vec<MeanCollision>) {
        // "In prelude, bumping is OK": the newcomers' island says nothing
        if self
            .main()
            .and_then(|r| r.info.as_ref())
            .is_some_and(|i| is_prelude(i.region_flags))
        {
            return;
        }
        let cfg = self.collisions.cfg;
        for c in list {
            let Some(kind) = Kind::from_wire(c.kind) else {
                log::info!("collision of unknown type {} from {}", c.kind, c.perp);
                continue;
            };
            self.social.avatar_names.want(&c.perp);
            if cfg.in_chat {
                self.system_notice(
                    kind.sentence(&format!("secondlife:///app/agent/{}/about", c.perp)),
                    crate::world::notice_topics::Topic::People,
                );
            }
            // ScriptDialogReply from ourselves: chat on a negative channel
            // that reaches the agent's attachments
            if cfg.report {
                self.groups.out.push(NetCommand::ScriptDialogReply {
                    sender: None,
                    object_id: self.agent_id,
                    channel: cfg.channel,
                    index: 1,
                    label: report_text(&c),
                });
            }
            self.collisions.record(Collision {
                perp: c.perp,
                kind,
                time: c.time,
                magnitude: c.magnitude,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn hit(perp: u128, kind: u8, time: u32) -> MeanCollision {
        MeanCollision {
            perp: id(perp),
            time,
            magnitude: 12.5,
            kind,
        }
    }

    fn world() -> World {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.agent_id = id(99);
        w
    }

    fn rows(w: &World) -> Vec<(Uuid, Kind, u32)> {
        w.collisions.list().map(|c| (c.perp, c.kind, c.time)).collect()
    }

    #[test]
    fn a_resident_is_listed_once_per_kind_newest_first() {
        let mut w = world();
        w.on_mean_collisions(vec![hit(1, 1, 10), hit(2, 2, 11)]);
        w.on_mean_collisions(vec![hit(1, 1, 20), hit(1, 5, 21)]);
        assert_eq!(
            rows(&w),
            [(id(1), Kind::PhysicalObject, 21), (id(1), Kind::Bump, 20), (id(2), Kind::Push, 11)]
        );
    }

    #[test]
    fn unknown_types_are_dropped_and_the_list_keeps_twenty() {
        let mut w = world();
        w.on_mean_collisions(vec![hit(1, 0, 1), hit(1, 6, 1)]);
        assert!(rows(&w).is_empty());
        w.on_mean_collisions((0..30).map(|n| hit(n, 1, n as u32)).collect());
        let kept = rows(&w);
        assert_eq!(kept.len(), MAX);
        assert_eq!(kept[0].0, id(29));
        assert_eq!(kept[MAX - 1].0, id(10));
    }

    #[test]
    fn the_options_tell_nearby_chat_and_the_scripts() {
        let mut w = world();
        w.on_mean_collisions(vec![hit(1, 1, 10)]);
        assert!(w.chat.is_empty(), "off by default");
        assert!(w.groups.out.is_empty());
        w.collisions.cfg = Settings {
            in_chat: true,
            report: true,
            channel: -7,
        };
        w.on_mean_collisions(vec![hit(1, 2, 11)]);
        let line = w.chat.back().expect("a chat line");
        assert_eq!(line.text, Kind::Push.sentence(&format!("secondlife:///app/agent/{}/about", id(1))));
        let expected = format!("{},12.50,2", id(1));
        assert!(matches!(
            w.groups.out.as_slice(),
            [NetCommand::ScriptDialogReply { sender: None, object_id, channel: -7, index: 1, label }]
                if *object_id == id(99) && *label == expected
        ));
    }

    #[test]
    fn the_prelude_follows_the_region_flags() {
        assert!(is_prelude(1 << 3));
        assert!(!is_prelude((1 << 3) | (1 << 1)));
        assert!(!is_prelude(0));
    }
}
