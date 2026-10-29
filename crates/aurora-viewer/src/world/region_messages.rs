//! A message to the whole region: an estate manager's "Send message to
//! region" (`simulatormessage`) or a god's, which the simulator sends as an
//! IM_NOTHING_SPECIAL addressed to nobody. The Legacy Viewer gives it the
//! GodMessage notification (the sender and the text, kept under the bell)
//! and a line of nearby chat, before any rule of conversations: no mute,
//! no friends-only filter, no automatic response (LLIMProcessing::
//! processNewMessage, `to_id.isNull()`, and god_message_name_cb,
//! indra/newview/llimprocessing.cpp, originally LGPL 2.1). Aurora opened a
//! conversation with the sender instead. Shown in the middle of the screen
//! by `ui::region_message_prompt` (#1404).

use super::notice_topics::Topic;
use super::notifications::{Data, Kind};
use super::World;
use aurora_net::InstantMessage;

/// IM_NOTHING_SPECIAL from a resident to nobody: the whole region.
pub fn is_region_message(im: &InstantMessage) -> bool {
    im.dialog == 0 && im.to_agent_id.is_nil() && !im.from_agent_id.is_nil()
}

impl World {
    /// Its notification and its line of nearby chat.
    pub(super) fn on_region_message(&mut self, im: &InstantMessage) {
        let text = im.message.trim();
        if text.is_empty() {
            return;
        }
        // the server writes legacy names: the complete one when known
        // (god_message_name_cb looks it up)
        let name = self
            .social
            .avatar_names
            .complete(&im.from_agent_id)
            .unwrap_or_else(|| im.from_name.clone());
        if !im.from_name.is_empty() {
            self.social.names.entry(im.from_agent_id).or_insert_with(|| im.from_name.clone());
        }
        let region = self.region_name();
        self.notifications.push(
            Kind::Info,
            aurora_i18n::tr!("world-region-message-title", name = name.clone()),
            text,
            Data::RegionMessage {
                from: im.from_agent_id,
                region,
            },
        );
        self.system_notice(
            aurora_i18n::tr!("world-region-message-line", name = name, message = text.to_owned()),
            Topic::Region,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::mutes::{Mute, MuteType};
    use super::*;
    use std::sync::Arc;
    use uuid::Uuid;

    const ME: Uuid = Uuid::from_u128(1);
    const MANAGER: Uuid = Uuid::from_u128(2);

    fn im(to: Uuid, dialog: u8, text: &str) -> InstantMessage {
        InstantMessage {
            from_agent_id: MANAGER,
            from_name: "Estate Manager".into(),
            to_agent_id: to,
            dialog,
            session_id: Uuid::from_u128(3),
            message: text.into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        }
    }

    #[test]
    fn only_a_message_to_nobody_is_for_the_region() {
        assert!(is_region_message(&im(Uuid::nil(), 0, "Restart in 5")));
        assert!(!is_region_message(&im(ME, 0, "hi")));
        // an object's message, a friendship offer
        assert!(!is_region_message(&im(Uuid::nil(), 19, "hello")));
        assert!(!is_region_message(&im(Uuid::nil(), 38, "")));
        // nobody to nobody: the SystemMessage of a sender-less IM
        let mut system = im(Uuid::nil(), 0, "Maintenance");
        system.from_agent_id = Uuid::nil();
        assert!(!is_region_message(&system));
    }

    /// The notification under the bell and the line of nearby chat, no
    /// conversation; a friends-only rule and a block do not stop it.
    #[test]
    fn a_region_message_is_a_notification_and_a_nearby_line() {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = ME;
        world.notifications.started = true;
        world.status.cfg.friends_only = true;
        let blocked = Mute {
            id: MANAGER,
            name: "Estate Manager".into(),
            kind: MuteType::Agent,
            flags: 0,
        };
        world.mutes.add(blocked, 0).expect("blocked");
        world.apply(aurora_net::NetEvent::InstantMessage(im(Uuid::nil(), 0, "  Back in 5 minutes  ")));
        let n = world.notifications.list.last().expect("a notification");
        assert!(matches!(&n.data, Data::RegionMessage { from, .. } if *from == MANAGER));
        assert_eq!(n.body, "Back in 5 minutes");
        assert!(n.title.contains("Estate Manager"));
        assert!(!world.social.has_session(&MANAGER), "no conversation");
        let line = world.chat.back().expect("a nearby line");
        assert!(line.text.contains("Back in 5 minutes"));
        // an empty one says nothing
        let before = world.notifications.list.len();
        world.apply(aurora_net::NetEvent::InstantMessage(im(Uuid::nil(), 0, "   ")));
        assert_eq!(world.notifications.list.len(), before);
    }
}
