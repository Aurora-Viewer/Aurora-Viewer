//! RLV communication filtering from LLIMProcessing::processNewMessage,
//! LLViewerChatterBoxInvitation::post and LLFloaterIMSession::sendMsg
//! (llimprocessing.cpp, llimview.cpp, llfloaterimsession.cpp, LGPL 2.1).
use super::super::World;
use aurora_net::{InstantMessage, NetCommand};
use std::borrow::Cow;
use uuid::Uuid;

#[derive(PartialEq, Eq)]
pub(crate) enum Received {
    Original,
    Masked,
}

impl World {
    /// RlvActions::canStartIM: existing resident and group/conference
    /// sessions remain usable, including a session still being initialized.
    pub fn rlva_can_start_im(&self, recipient: Uuid) -> bool {
        self.rlva_allows_im("startim", recipient) || self.social.has_session(&recipient) || self.groups.sessions.contains_key(&recipient)
    }

    pub(crate) fn rlva_start_ims(&mut self, recipients: &[Uuid]) -> bool {
        if recipients.iter().all(|recipient| self.rlva_can_start_im(*recipient)) {
            true
        } else {
            self.system_notice(aurora_i18n::tr!("rlva-blocked"), crate::world::notice_topics::Topic::Warning);
            false
        }
    }

    /// A refusal is an IM_BUSY_AUTO_RESPONSE with the supplied conversation
    /// UUID, not a normal IM, teleport decline or arbitrary permission bypass.
    pub(crate) fn rlva_busy_reply(&mut self, to: Uuid, session: Uuid, message: &str) {
        if self.rlva.enabled() && !to.is_nil() && self.rlva.out.len() < 256 {
            self.rlva.out.push(NetCommand::SendImDialog {
                to,
                dialog: 20,
                id: session,
                message: message.into(),
                bucket: vec![0],
            });
        }
    }

    pub(crate) fn rlva_receive_im(&mut self, message: &mut InstantMessage) -> Received {
        if !self.rlva.enabled() {
            return Received::Original;
        }
        match message.dialog {
            0 if !message.offline
                && !crate::world::mutes::is_linden(&message.from_name)
                && !self.rlva_allows_im("recvim", message.from_agent_id) =>
            {
                self.rlva_busy_reply(
                    message.from_agent_id,
                    message.session_id,
                    aurora_i18n::tr!("rlva-im-blocked-receive-remote"),
                );
                message.message = aurora_i18n::tr!("rlva-im-blocked-receive").into();
                Received::Masked
            }
            // IM_SESSION_SEND in processNewMessage does not filter existing
            // sessions. The invitation handler separately checks group UUIDs
            // and censors a conference's initiating message.
            _ => Received::Original,
        }
    }

    pub(crate) fn rlva_can_send_session(&self, id: Uuid) -> bool {
        if !self.rlva.enabled() || (!self.rlva.has("sendim") && self.rlva.rules_for("sendimto", Uuid::nil()).next().is_none()) {
            return true;
        }
        if self.groups.is_member(&id) || self.groups.sessions.get(&id).is_some_and(|session| session.group) {
            return self.rlva_allows_im("sendim", id);
        }
        self.groups.sessions.get(&id).is_some_and(|session| {
            session
                .participants
                .iter()
                .filter(|participant| **participant != self.agent_id)
                .all(|participant| self.rlva_allows_im("sendim", *participant))
        })
    }

    pub(crate) fn rlva_outgoing_im_text<'a>(&self, to: Uuid, session: bool, message: &'a str) -> Cow<'a, str> {
        let allowed = if session {
            self.rlva_can_send_session(to)
        } else {
            self.rlva_allows_im("sendim", to)
        };
        if allowed {
            Cow::Borrowed(message)
        } else {
            Cow::Borrowed(aurora_i18n::tr!("rlva-im-blocked-send"))
        }
    }

    /// The true result authorizes only the replacement text this function
    /// installs, at the user's common send boundary. Never forward the source.
    pub fn rlva_prepare_outgoing_im(&self, command: &mut NetCommand) -> bool {
        let (to, session, message) = match command {
            NetCommand::SendIm { to, message } => (*to, false, message),
            NetCommand::SendImDialog { to, dialog, message, .. } if matches!(*dialog, 0 | 17) => (*to, *dialog == 17, message),
            _ => return false,
        };
        let allowed = if session {
            self.rlva_can_send_session(to)
        } else {
            self.rlva_allows_im("sendim", to)
        };
        if !allowed {
            *message = aurora_i18n::tr!("rlva-im-blocked-send").into();
        }
        !allowed
    }

    pub(crate) fn rlva_teleport_message(&self, message: &str, sender: Uuid, godlike: bool) -> String {
        if !self.rlva_allows_im("recvim", sender) || (!godlike && self.rlva.has("showloc")) {
            aurora_i18n::tr!("rlva-hidden-generic").into()
        } else {
            message.replace('\r', "")
        }
    }

    pub(crate) fn rlva_teleport_dnd(&mut self, sender: Uuid) {
        if self.status.dnd {
            let text = self.status.cfg.dnd_text.clone();
            self.auto_im(sender, Uuid::nil(), &text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetEvent, SessionInvite};
    const PERSON: Uuid = Uuid::from_u128(0x726c766170657273);
    const ISSUER: Uuid = Uuid::from_u128(0x726c766169737375);
    const SESSION: Uuid = Uuid::from_u128(0x726c766173657373);

    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(99);
        world.rlva.configure(true);
        world
    }
    fn restrict(world: &mut World, message: &str) {
        world.apply(NetEvent::Chat(ChatMessage {
            from_name: "Controller".into(),
            source_id: ISSUER,
            owner_id: world.agent_id,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: message.into(),
        }));
    }
    fn im(world: &World, dialog: u8) -> InstantMessage {
        InstantMessage {
            from_agent_id: PERSON,
            from_name: "Test Resident".into(),
            to_agent_id: world.agent_id,
            dialog,
            session_id: SESSION,
            message: "Private payload".into(),
            offline: false,
            from_group: false,
            binary_bucket: vec![],
            timestamp: 0,
        }
    }
    fn last(world: &World, session: Uuid) -> &str {
        &world
            .social
            .ims
            .iter()
            .find(|im| im.other == session)
            .unwrap()
            .lines
            .back()
            .unwrap()
            .text
    }

    #[test]
    fn rlva_received_im_masks_local_content_and_sends_the_native_busy_reply_in_the_same_session() {
        let mut world = world();
        restrict(&mut world, "@recvim=n,sendim=n");
        world.apply(NetEvent::InstantMessage(im(&world, 0)));
        assert_eq!(last(&world, PERSON), aurora_i18n::tr!("rlva-im-blocked-receive"));
        assert!(
            matches!(&world.rlva.out[..], [NetCommand::SendImDialog { to: PERSON, dialog: 20, id: SESSION, message, bucket }] if message == aurora_i18n::tr!("rlva-im-blocked-receive-remote") && bucket == &[0])
        );
        world.rlva.out.clear();
        let mut offline = im(&world, 0);
        offline.offline = true;
        world.apply(NetEvent::InstantMessage(offline));
        assert!(last(&world, PERSON).contains("Private payload") && world.rlva.out.is_empty());
        let mut staff = im(&world, 0);
        staff.from_name = "Test Linden".into();
        world.apply(NetEvent::InstantMessage(staff));
        assert_eq!(last(&world, PERSON), "Private payload");
        assert!(world.rlva.out.is_empty());
        let mut typing = im(&world, 41);
        typing.message.clear();
        world.apply(NetEvent::InstantMessage(typing));
        assert!(
            world.social.is_typing(&PERSON),
            "typing notices are distinct from restricted message content"
        );
    }

    #[test]
    fn rlva_outgoing_im_substitutes_both_the_local_line_and_wire_text_without_releasing_a_lock() {
        let mut world = world();
        restrict(&mut world, "@sendim=n");
        world.own_im(PERSON, "Do not disclose");
        assert_eq!(last(&world, PERSON), aurora_i18n::tr!("rlva-im-blocked-send"));
        let mut command = NetCommand::SendIm {
            to: PERSON,
            message: "Do not disclose".into(),
        };
        assert!(world.rlva_prepare_outgoing_im(&mut command));
        assert!(matches!(&command, NetCommand::SendIm { message, .. } if message == aurora_i18n::tr!("rlva-im-blocked-send")));
        assert!(
            !world.rlva_allows_command(&command, Uuid::nil()),
            "the narrow replacement route never grants general send permission"
        );
        restrict(&mut world, &format!("@sendim:{PERSON}=n"));
        let mut permitted = NetCommand::SendIm {
            to: PERSON,
            message: "Permitted".into(),
        };
        assert!(!world.rlva_prepare_outgoing_im(&mut permitted));
        assert!(world.rlva_allows_command(&permitted, Uuid::nil()));
        assert!(world.rlva_allows_command(
            &NetCommand::SendImDialog {
                to: PERSON,
                dialog: 41,
                id: SESSION,
                message: String::new(),
                bucket: vec![]
            },
            Uuid::nil()
        ));
    }

    #[test]
    fn rlva_conference_messages_check_the_sender_on_receive_and_every_participant_on_send() {
        let mut world = world();
        restrict(&mut world, "@recvim=n,sendim=n");
        world.apply(NetEvent::SessionInvite(SessionInvite {
            session: SESSION,
            from: PERSON,
            from_name: "Test Resident".into(),
            message: "First private payload".into(),
            session_name: "Test conference".into(),
            offline: false,
            timestamp: 0,
        }));
        assert_eq!(last(&world, SESSION), aurora_i18n::tr!("rlva-im-blocked-receive"));
        assert!(world.groups.sessions.get(&SESSION).unwrap().participants.contains(&PERSON));
        assert!(!world.rlva_can_send_session(SESSION));
        // Existing session traffic is not filtered again by IM_SESSION_SEND.
        world.apply(NetEvent::InstantMessage(im(&world, 17)));
        assert_eq!(last(&world, SESSION), "Private payload");
        restrict(&mut world, &format!("@sendim:{PERSON}=n,recvim:{PERSON}=n"));
        assert!(world.rlva_can_send_session(SESSION));
        world.apply(NetEvent::InstantMessage(im(&world, 17)));
        assert_eq!(last(&world, SESSION), "Private payload");
        world
            .groups
            .sessions
            .get_mut(&SESSION)
            .unwrap()
            .participants
            .insert(Uuid::from_u128(123));
        assert!(!world.rlva_can_send_session(SESSION));
        world.send_session_im(SESSION, "Do not disclose");
        assert_eq!(last(&world, SESSION), aurora_i18n::tr!("rlva-im-blocked-send"));
        assert!(
            matches!(world.groups.out.last(), Some(NetCommand::SendImDialog { dialog: 17, message, .. }) if message == aurora_i18n::tr!("rlva-im-blocked-send"))
        );
        let group = Uuid::from_u128(55);
        world.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Test group".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: false,
            list_in_profile: false,
            contribution: 0,
        });
        world.apply(NetEvent::SessionInvite(SessionInvite {
            session: group,
            from: PERSON,
            from_name: "Test Resident".into(),
            message: "Hidden group text".into(),
            session_name: "Test group".into(),
            offline: false,
            timestamp: 0,
        }));
        assert!(
            !world.groups.sessions.contains_key(&group),
            "a sender exception does not authorize a group invitation"
        );
    }

    #[test]
    fn rlva_refused_teleport_offers_reply_busy_and_allowed_invitation_text_obeys_receive_and_location_privacy() {
        let mut world = world();
        restrict(&mut world, "@tplure=n,tprequest=n");
        world.apply(NetEvent::InstantMessage(im(&world, 22)));
        world.apply(NetEvent::InstantMessage(im(&world, 26)));
        assert!(world.teleport_invites.list.is_empty());
        assert_eq!(world.rlva.out.len(), 2);
        assert!(world.rlva.out.iter().all(|command| matches!(command, NetCommand::SendImDialog { dialog: 20, id, message, .. } if id.is_nil() && message == aurora_i18n::tr!("rlva-im-blocked-teleport-remote"))));
        assert!(
            world.teleport_invites.take_commands().is_empty(),
            "RLVa refusal is not a native user decline"
        );
        restrict(&mut world, "@clear,recvim=n");
        world.apply(NetEvent::InstantMessage(im(&world, 22)));
        assert_eq!(
            world.teleport_invites.list.last().unwrap().message,
            aurora_i18n::tr!("rlva-hidden-generic")
        );
        restrict(&mut world, "@clear,showloc=n");
        world.apply(NetEvent::InstantMessage(im(&world, 26)));
        assert_eq!(
            world.teleport_invites.list.last().unwrap().message,
            aurora_i18n::tr!("rlva-hidden-generic")
        );
        world.rlva.configure(false);
        world.apply(NetEvent::InstantMessage(im(&world, 26)));
        assert_eq!(world.teleport_invites.list.last().unwrap().message, "Private payload");
    }

    #[test]
    fn rlva_startim_checks_all_conference_members_and_group_ids_before_changing_an_existing_call() {
        let mut world = world();
        let other = Uuid::from_u128(123);
        let group = Uuid::from_u128(55);
        world.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Test group".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: false,
            list_in_profile: false,
            contribution: 0,
        });
        restrict(&mut world, "@startim=n");
        world.start_text_conference(vec![PERSON]);
        world.start_group_chat(group);
        world.start_call(PERSON);
        assert!(world.social.ims.is_empty() && world.groups.sessions.is_empty() && world.groups.out.is_empty());
        assert!(world.calls.active.is_none());
        restrict(&mut world, &format!("@startim:{PERSON}=n"));
        world.start_call(PERSON);
        assert!(world.calls.active.as_ref().unwrap().p2p());
        world.groups.out.clear();
        world.start_conference(vec![PERSON, other]);
        world.start_text_conference(vec![PERSON, other]);
        world.convert_to_conference(PERSON, vec![other]);
        world.start_call(group);
        assert_eq!(world.calls.active.as_ref().unwrap().conversation(), PERSON);
        assert!(world.calls.conferences.is_empty() && world.groups.out.is_empty());
        restrict(
            &mut world,
            &format!("@startim:{PERSON}=y,startimto:{PERSON}=n,startim:{other}=n,startim:{group}=n"),
        );
        assert!(world.rlva_can_start_im(PERSON), "a real open session overrides startimto too");
        world.start_text_conference(vec![PERSON, other]);
        assert_eq!(world.calls.conferences.len(), 1);
        world.start_group_chat(group);
        restrict(&mut world, &format!("@startim:{group}=y,startimto:{group}=n"));
        world.groups.out.clear();
        world.start_call(group);
        assert_eq!(
            world.calls.active.as_ref().unwrap().kind,
            super::super::super::calls::CallKind::Group
        );
        assert!(world.rlva_can_start_im(group), "a group being initialized is already open");
    }

    #[test]
    fn rlva_autoresponses_do_not_disclose_custom_text_for_blocked_or_staff_messages() {
        let mut world = world();
        world.status.cfg.autorespond = true;
        world.status.cfg.autorespond_text = "Personal automatic response".into();
        restrict(&mut world, "@recvim=n");
        let mut staff = im(&world, 0);
        staff.from_name = "Test Linden".into();
        world.apply(NetEvent::InstantMessage(staff));
        assert_eq!(last(&world, PERSON), "Private payload");
        assert!(world.groups.out.is_empty() && world.rlva.out.is_empty());
        world.leave_session(PERSON);
        world.apply(NetEvent::InstantMessage(im(&world, 0)));
        assert_eq!(last(&world, PERSON), aurora_i18n::tr!("rlva-im-blocked-receive"));
        assert!(world.groups.out.is_empty());
    }

    #[test]
    fn rlva_forced_teleport_acceptance_overrides_status_rejection_but_still_obeys_locks_and_reports_dnd() {
        use crate::world::teleport_invites::Answer;
        let mut world = world();
        world.status.cfg.reject_teleports = true;
        world.status.dnd = true;
        restrict(&mut world, "@accepttp=n,accepttprequest=n");
        world.apply(NetEvent::InstantMessage(im(&world, 22)));
        assert_eq!(world.teleport_invites.list.last().unwrap().answer, Answer::Accepted);
        assert!(matches!(
            world.teleport_invites.take_commands().as_slice(),
            [NetCommand::AcceptLure { godlike: false, .. }]
        ));
        assert_eq!(world.groups.out.len(), 1);
        assert!(
            matches!(&world.groups.out[0], NetCommand::SendImDialog { dialog: 20, message, .. } if message == &world.status.cfg.dnd_text)
        );
        world.groups.out.clear();
        world.apply(NetEvent::InstantMessage(im(&world, 26)));
        assert_eq!(world.teleport_invites.list.last().unwrap().answer, Answer::Accepted);
        assert!(matches!(
            world.teleport_invites.take_commands().as_slice(),
            [NetCommand::OfferTeleport { .. }]
        ));
        restrict(&mut world, "@tplure=n,tprequest=n");
        let count = world.teleport_invites.list.len();
        world.apply(NetEvent::InstantMessage(im(&world, 22)));
        world.apply(NetEvent::InstantMessage(im(&world, 26)));
        assert_eq!(world.teleport_invites.list.len(), count);
        assert_eq!(world.rlva.out.len(), 2);
        assert!(world.teleport_invites.take_commands().is_empty());
    }
}
