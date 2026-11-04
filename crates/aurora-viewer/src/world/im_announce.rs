//! "Announce incoming IMs as soon as the sender starts typing"
//! (FSAnnounceIncomingIM): a resident's IM_TYPING_START with no conversation
//! open yet opens it with "Incoming message from <name>". Port of
//! LLIMMgr::processIMTypingCore and typingNameCallback
//! (indra/newview/llimview.cpp, originally LGPL 2.1).

use super::{ChatKind, ChatLine, Origin, World};
use crate::ui_sound::{ImKind, ImMessage};
use aurora_net::InstantMessage;
use std::time::SystemTime;
use uuid::Uuid;

impl World {
    /// A typing start from `im.from_agent_id` (blocked residents never get
    /// here). The line counts as unread but is never written to the
    /// transcript nor toasted (addMessageSilently and notify_of_message skip
    /// announcements); the new conversation's sound plays as for a message.
    pub(super) fn announce_typing(&mut self, im: &InstantMessage) {
        let from = im.from_agent_id;
        if !self.announce_incoming_im || from.is_nil() || from == self.agent_id || self.social.has_session(&from) {
            return;
        }
        // VoiceCallsFriendsOnly: not for a resident who is not a friend
        if self.friends_only_drops(&from, &im.from_name) || !self.rlva_allows_im("recvim", from) {
            return;
        }
        // the automatic response goes now, as typingNameCallback sends it:
        // the conversation the announcement opens would stop the message's
        // own (`autoresponse_due` answers only a message that opens one)
        let as_message = InstantMessage { dialog: 0, ..im.clone() };
        let autoresponse = self.autoresponse_due(&as_message);
        if !im.from_name.is_empty() {
            self.social.names.entry(from).or_insert_with(|| im.from_name.clone());
        }
        let name = self.social.avatar_names.complete(&from).unwrap_or_else(|| im.from_name.clone());
        self.im_messages.push(ImMessage {
            kind: ImKind::Private,
            session: from,
            new_session: true,
        });
        self.social.session_mut(from);
        let line = ChatLine {
            time: SystemTime::now(),
            from: String::new(),
            text: aurora_i18n::tr!("world-im-announce-incoming", name = name),
            kind: ChatKind::System,
            source: Uuid::nil(),
            origin: Origin::Live,
        };
        log::info!("incoming IM announced (the sender started typing)");
        self.conversation_card(from, line);
        self.send_autoresponse(&as_message, autoresponse);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{IM_TYPING_START, IM_TYPING_STOP};
    use aurora_net::NetEvent;

    const FROM: Uuid = Uuid::from_u128(77);

    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        world
    }

    fn typing(world: &World, dialog: u8) -> NetEvent {
        NetEvent::InstantMessage(InstantMessage {
            from_agent_id: FROM,
            from_name: "Typer Resident".into(),
            to_agent_id: world.agent_id,
            dialog,
            session_id: Uuid::from_u128(world.agent_id.as_u128() ^ FROM.as_u128()),
            message: "typing".into(),
            offline: false,
            from_group: false,
            binary_bucket: vec![0],
            timestamp: 0,
        })
    }

    fn lines(world: &World) -> Vec<String> {
        world
            .social
            .ims
            .iter()
            .find(|s| s.other == FROM)
            .map(|s| s.lines.iter().map(|l| l.text.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn off_by_default_typing_opens_nothing() {
        let mut world = world();
        world.apply(typing(&world, IM_TYPING_START));
        assert!(world.social.is_typing(&FROM));
        assert!(!world.social.has_session(&FROM));
        assert!(world.im_messages.is_empty());
    }

    #[test]
    fn a_typing_start_opens_the_conversation_once_with_an_unread_line() {
        let mut world = world();
        world.announce_incoming_im = true;
        world.apply(typing(&world, IM_TYPING_START));
        assert!(world.social.is_typing(&FROM), "still typing in the opened conversation");
        let text = lines(&world);
        assert_eq!(text.len(), 1);
        assert!(text[0].contains("Typer Resident"), "{text:?}");
        assert_eq!(world.social.total_unread(), 1);
        let s = world.social.ims.iter().find(|s| s.other == FROM).expect("session");
        assert_eq!(s.lines[0].kind, ChatKind::System);
        assert!(world.im_messages.iter().any(|m| m.session == FROM && m.new_session));
        // a stop and a new start: the conversation exists, no second line
        world.apply(typing(&world, IM_TYPING_STOP));
        world.apply(typing(&world, IM_TYPING_START));
        assert_eq!(lines(&world).len(), 1);
    }

    #[test]
    fn friends_only_keeps_a_stranger_out() {
        let mut world = world();
        world.announce_incoming_im = true;
        world.status.cfg.friends_only = true;
        world.apply(typing(&world, IM_TYPING_START));
        assert!(!world.social.has_session(&FROM));
    }

    #[test]
    fn the_automatic_response_goes_with_the_announcement() {
        let mut world = world();
        world.announce_incoming_im = true;
        world.status.dnd = true;
        world.apply(typing(&world, IM_TYPING_START));
        let text = lines(&world);
        assert_eq!(text.len(), 2, "the announcement, then the reply noted: {text:?}");
        let sent = world.groups.take_commands();
        assert!(
            sent.iter()
                .any(|c| matches!(c, aurora_net::NetCommand::SendImDialog { to, .. } if *to == FROM)),
            "the do not disturb reply is sent"
        );
    }
}
