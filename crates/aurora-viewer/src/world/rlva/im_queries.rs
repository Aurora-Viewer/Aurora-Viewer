//! RlvHandler::processIMQuery/onIMQueryListResponse and RlvFloaterBehaviours
//! formatting (rlvhandler.cpp / rlvfloaters.cpp, originally LGPL 2.1).
use super::{
    super::{
        World,
        notifications::{Data, Kind, NameTarget, Response},
    },
    VERSION,
    legacy_strings::{Remote, remote},
};
use aurora_net::{InstantMessage, NetCommand};
use uuid::Uuid;

#[derive(Default)]
pub struct Queries {
    pub close: Vec<Uuid>,
    pub remember: bool,
    /// IMs whose text was settled when written (`rlva_send_as_decided`):
    /// recipient, dialog, text.
    pub(crate) decided: Vec<(Uuid, u8, String)>,
}

fn exception(name: &str) -> bool {
    matches!(
        name,
        "recvchat"
            | "recvemote"
            | "sendim"
            | "recvim"
            | "startim"
            | "tplure"
            | "tprequest"
            | "accepttp"
            | "accepttprequest"
            | "shownames"
            | "shownametags"
    )
}

impl World {
    fn rlva_im_reply(&mut self, to: Uuid, message: &str, busy: bool) {
        let offline = self.social.friends.iter().any(|friend| friend.id == to && !friend.online);
        // RlvUtil::sendIMMessage (rlvcommon.cpp): utf8str_split on '\n'
        for chunk in super::chat::utf8_split(message, b'\n') {
            if self.rlva.out.len() >= 256 {
                break;
            }
            self.rlva.out.push(NetCommand::ImReply {
                to,
                message: chunk.to_owned(),
                busy,
                offline,
            });
        }
    }

    /// Called after mute, flood and friends-only checks, before recvim. IMs
    /// never become an object command source and exact query case is retained.
    pub(crate) fn rlva_im_query(&mut self, im: &InstantMessage) -> bool {
        if !self.rlva.enabled() || im.dialog != 0 || im.offline || im.from_agent_id.is_nil() {
            return false;
        }
        let from = im.from_agent_id;
        if im.message == "@stopim" {
            if !self.rlva_allows_im("startim", from) {
                self.rlva_im_reply(from, remote(Remote::StopImEndSession), true);
                if self.social.has_session(&from) {
                    self.conversation_closed(&from);
                    self.leave_session(from);
                    self.rlva.im.close.push(from);
                    // RlvUtil::notifyBlocked(StopIm::EndSessionLocal) with the
                    // resident's "about" link as [NAME], logged to chat
                    let link = format!("secondlife:///app/agent/{from}/about");
                    self.system_message(aurora_i18n::tr!("rlva-im-ended-local", name = link));
                }
                return true;
            }
            if !self.rlva.preferences.im_queries {
                return false;
            }
            self.rlva_im_reply(from, remote(Remote::StopImNoSession), true);
            return true;
        }
        if !self.rlva.preferences.im_queries {
            return false;
        }
        if im.message == "@version" {
            self.rlva_im_reply(from, VERSION, true);
            return true;
        }
        let exceptions = match im.message.as_str() {
            "@list" => false,
            "@except" => true,
            _ => return false,
        };
        // RlvHandler::processIMQuery: one RLVaListRequested notification per
        // query, however many are already waiting
        if self.rlva.preferences.im_queries_auto_share {
            self.rlva_im_list(from, exceptions);
        } else {
            let name = if self.rlva_can_name(from) {
                im.from_name.as_str()
            } else {
                self.rlva_anonymous(from).display_name.as_str()
            };
            let id = self.notifications.push(
                Kind::Permissions,
                aurora_i18n::tr!("rlva-im-query-title"),
                aurora_i18n::tr!("rlva-im-query-body", name = name),
                Data::RlvaQuery { from, exceptions },
            );
            self.notifications.link_name(id, name, NameTarget::Avatar(from));
        }
        true
    }

    /// RlvFloaterBehaviours::getFormattedBehaviourString (rlvfloaters.cpp):
    /// the objects in the order of their keys, each one's rules in the order
    /// it issued them; then, for @list, a blank line and the suffix
    /// (RlvHandler::onIMQueryListResponse).
    fn rlva_im_list(&mut self, to: Uuid, exceptions: bool) {
        let mut message = format!("{VERSION}\n");
        for (source, rules) in &self.rlva.issued {
            message.push_str(&format!("\n{}:\n", self.rlva_object_label(*source)));
            for rule in rules {
                if (!rule.option.is_empty() && exception(&rule.behaviour)) != exceptions {
                    continue;
                }
                message.push_str(&format!("  -> {}", super::rule_text(rule)));
                if let Some(name) = self.rlva_option_label(&rule.option).filter(|name| *name != rule.option) {
                    message.push_str(&format!("  [{name}]"));
                }
                message.push('\n');
            }
        }
        if !exceptions {
            message.push('\n');
            message.push_str(remote(Remote::ListSuffix));
        }
        self.rlva_im_reply(to, &message, false);
    }

    /// rlvGetItemNameFromObjID (rlvfloaters.cpp): an avatar's complete name;
    /// an attachment's item name with its point, and ", child" for a child
    /// prim; anything else its key.
    fn rlva_object_label(&self, id: Uuid) -> String {
        let Some((index, object)) = self.objects.index_of_uuid(&id).and_then(|i| self.objects.get(i).map(|o| (i, o))) else {
            return id.to_string();
        };
        if object.is_avatar() {
            return self
                .social
                .avatar_names
                .raw(&id)
                .map(|name| name.complete(&self.social.avatar_names.options))
                .or_else(|| object.display_name())
                .unwrap_or_else(|| id.to_string());
        }
        let root_index = self.objects.linkset_root(index);
        let root = self.objects.get(root_index).unwrap_or(object);
        let point = root.attachment_point();
        let worn = point != 0
            && self
                .objects
                .parent_of(root)
                .and_then(|i| self.objects.get(i))
                .is_some_and(|p| p.is_avatar());
        if !worn {
            return id.to_string();
        }
        let name = root
            .attachment_item_id()
            .and_then(|item| self.inventory.items.get(&item))
            .map_or_else(|| id.to_string(), |item| item.name.clone());
        let point_name = self.avatar_lib.attach_names.get(&point).map_or("Unknown", String::as_str);
        format!("{name} ({point_name}{})", if root_index == index { "" } else { ", child" })
    }

    /// The name RlvFloaterBehaviours::getFormattedBehaviourString gives a
    /// rule's key: an object in view as `rlva_object_label`, a resident's
    /// account name (their display name without one), a group's name.
    fn rlva_option_label(&self, option: &str) -> Option<String> {
        let id = (option.len() == 36)
            .then(|| Uuid::parse_str(option).ok())
            .flatten()
            .filter(|id| !id.is_nil())?;
        if self.objects.index_of_uuid(&id).is_some() {
            return Some(self.rlva_object_label(id));
        }
        if let Some(name) = self.social.avatar_names.raw(&id) {
            return Some(if name.username.is_empty() {
                name.display_name.clone()
            } else {
                name.username.clone()
            });
        }
        self.groups
            .group(&id)
            .map(|group| group.name.clone())
            .or_else(|| self.search.groups.get(&id).map(|group| group.name.clone()))
    }

    /// RlvHandler::onIMQueryListResponse answers whatever the IM query
    /// setting became since the question; RLVa turned off in the session
    /// (Aurora only) leaves nothing to list.
    pub(crate) fn rlva_answer_im_query(&mut self, from: Uuid, exceptions: bool, response: &Response) {
        if !self.rlva.enabled() {
            return;
        }
        match response {
            Response::Accept | Response::Button(0 | 1) => {
                if matches!(response, Response::Button(1)) {
                    self.rlva.preferences.im_queries_auto_share = true;
                    self.rlva.im.remember = true;
                }
                self.rlva_im_list(from, exceptions);
            }
            _ => self.rlva_im_reply(from, remote(Remote::ListDeny), true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetEvent};

    const ISSUER: Uuid = Uuid::from_u128(1);
    const PERSON: Uuid = Uuid::from_u128(2);

    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.rlva.configure(true);
        world
    }

    fn command(world: &mut World, text: &str) {
        assert!(world.rlva.receive(&ChatMessage {
            from_name: "Controller".into(),
            source_id: ISSUER,
            owner_id: world.agent_id,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: text.into()
        }));
    }

    fn query(world: &mut World, text: &str, offline: bool) {
        world.apply(NetEvent::InstantMessage(InstantMessage {
            from_agent_id: PERSON,
            from_name: "Demo Resident".into(),
            to_agent_id: world.agent_id,
            dialog: 0,
            session_id: Uuid::nil(),
            message: text.into(),
            offline,
            from_group: false,
            binary_bucket: vec![],
            timestamp: 0,
        }));
    }

    #[test]
    fn rlva_im_queries_are_online_exact_and_recognized_before_receive_im_restrictions() {
        let mut world = world();
        command(&mut world, "@recvim=n,sendim=n");
        let sessions = world.social.ims.len();
        query(&mut world, "@version", false);
        assert!(matches!(world.rlva.out.pop(), Some(NetCommand::ImReply { to: PERSON, message, busy: true, .. }) if message == VERSION));
        assert_eq!(world.social.ims.len(), sessions);
        for (text, offline) in [("@VERSION", false), ("@version", true), ("@fly=n", false)] {
            query(&mut world, text, offline);
            if offline {
                assert!(world.rlva.out.is_empty());
                assert!(
                    world
                        .social
                        .ims
                        .iter()
                        .find(|session| session.other == PERSON)
                        .unwrap()
                        .lines
                        .back()
                        .unwrap()
                        .text
                        .contains(text)
                );
            } else {
                assert!(matches!(world.rlva.out.pop(), Some(NetCommand::SendImDialog { dialog: 20, .. })));
            }
        }
        assert!(!world.rlva.has("fly"), "an IM cannot issue restrictions");
        world.rlva.configure(false);
        query(&mut world, "@version", false);
        assert!(world.rlva.out.is_empty());
        assert!(
            world
                .social
                .ims
                .iter()
                .any(|session| session.other == PERSON && session.lines.iter().any(|line| line.text == "@version"))
        );
    }

    #[test]
    fn rlva_im_list_requires_consent_and_separates_exceptions() {
        let mut world = world();
        let target = Uuid::from_u128(3);
        world.social.avatar_names.on_legacy(target, "Test", "Resident");
        command(&mut world, &format!("@fly=n,recvchat=n,recvchat:{target}=n,sendim=n"));
        query(&mut world, "@list", false);
        query(&mut world, "@list", false);
        assert!(world.rlva.out.is_empty());
        // RlvHandler::processIMQuery: one question per query
        let ids: Vec<_> = world
            .notifications
            .list
            .iter()
            .filter(|n| matches!(n.data, Data::RlvaQuery { .. }))
            .map(|n| n.id)
            .collect();
        assert_eq!(ids.len(), 2);
        for id in ids {
            world.respond_notification(id, Response::Decline);
            assert!(
                matches!(world.rlva.out.pop(), Some(NetCommand::ImReply { busy: true, message, .. }) if message == remote(Remote::ListDeny))
            );
        }
        query(&mut world, "@list", false);
        let id = world
            .notifications
            .list
            .iter()
            .find(|n| matches!(n.data, Data::RlvaQuery { .. }))
            .unwrap()
            .id;
        world.respond_notification(id, Response::Accept);
        let text = world
            .rlva
            .out
            .iter()
            .filter_map(|command| match command {
                NetCommand::ImReply { message, busy: false, .. } => Some(message.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(text.contains("  -> fly\n") && text.starts_with(VERSION));
        assert!(!text.contains(&format!("recvchat:{target}")));
        world.rlva.out.clear();
        query(&mut world, "@except", false);
        let id = world
            .notifications
            .list
            .iter()
            .find(|n| matches!(n.data, Data::RlvaQuery { .. }))
            .unwrap()
            .id;
        world.respond_notification(id, Response::Button(1));
        assert!(world.rlva.preferences.im_queries_auto_share && world.rlva.im.remember);
        let text = world
            .rlva
            .out
            .iter()
            .filter_map(|command| match command {
                NetCommand::ImReply { message, busy: false, .. } => Some(message.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(text.contains(&format!("recvchat:{target}")) && text.contains("  [Test"));
        assert!(!text.contains("  -> fly\n"));
        world.rlva.out.clear();
        query(&mut world, "@list", false);
        assert!(!world.rlva.out.is_empty() && !world.notifications.list.iter().any(|n| matches!(n.data, Data::RlvaQuery { .. })));
    }

    #[test]
    fn rlva_stopim_closes_only_a_restricted_session_even_with_queries_disabled() {
        let mut world = world();
        world.rlva.preferences.im_queries = false;
        world.social.session_mut(PERSON);
        command(&mut world, "@startim=n");
        query(&mut world, "@stopim", false);
        assert!(!world.social.has_session(&PERSON));
        assert_eq!(world.rlva.im.close, [PERSON]);
        assert!(matches!(world.rlva.out.last(), Some(NetCommand::ImReply { busy: true, .. })));
        world.rlva.im.close.clear();
        world.rlva.out.clear();
        command(&mut world, "@clear");
        query(&mut world, "@stopim", false);
        assert!(
            world.social.has_session(&PERSON),
            "with no startim restriction a disabled query remains an ordinary message"
        );
        assert!(world.rlva.out.is_empty());
        world.rlva.preferences.im_queries = true;
        query(&mut world, "@stopim", false);
        assert!(world.social.has_session(&PERSON) && matches!(world.rlva.out.last(), Some(NetCommand::ImReply { busy: true, .. })));
        world.rlva.configure(false);
        assert!(world.rlva.out.is_empty() && world.rlva.im.close.is_empty());
    }

    #[test]
    fn rlva_im_queries_keep_friend_and_block_filters_and_disabling_cancels_a_pending_answer() {
        let mut world = world();
        world.status.cfg.friends_only = true;
        query(&mut world, "@version", false);
        assert!(world.rlva.out.is_empty());
        world.status.cfg.friends_only = false;
        query(&mut world, "@list", false);
        assert!(world.notifications.list.iter().any(|n| matches!(n.data, Data::RlvaQuery { .. })));
        world.block(PERSON, "Demo Resident", crate::world::mutes::MuteType::Agent).unwrap();
        assert!(!world.notifications.list.iter().any(|n| matches!(n.data, Data::RlvaQuery { .. })));
        query(&mut world, "@version", false);
        assert!(world.rlva.out.is_empty());
        world.unblock(PERSON, "Demo Resident", 0);
        query(&mut world, "@list", false);
        let id = world
            .notifications
            .list
            .iter()
            .find(|n| matches!(n.data, Data::RlvaQuery { .. }))
            .unwrap()
            .id;
        world.rlva.configure(false);
        world.respond_notification(id, Response::Accept);
        assert!(world.rlva.out.is_empty());
    }

    #[test]
    fn rlva_im_response_chunks_keep_unicode_and_line_boundaries_without_oversized_packets() {
        // utf8str_split (rlvcommon.cpp): a cut at a line end drops that '\n'
        let text = format!("{}\n{}\n{}", "a".repeat(800), "é".repeat(800), "z".repeat(800));
        let split = super::super::chat::utf8_split(&text, b'\n');
        assert!(
            split
                .iter()
                .all(|part| part.len() <= 1023 && !part.is_empty() && !part.ends_with('\n'))
        );
        assert_eq!(split, ["a".repeat(800), "é".repeat(511), "é".repeat(289), "z".repeat(800)]);
    }

    fn replies(world: &World) -> String {
        world
            .rlva
            .out
            .iter()
            .filter_map(|command| match command {
                NetCommand::ImReply { message, busy: false, .. } => Some(message.as_str()),
                _ => None,
            })
            .collect()
    }

    /// RlvFloaterBehaviours::getFormattedBehaviourString (rlvfloaters.cpp):
    /// each object's rules in the order it issued them, a resident key named
    /// by its account name, a blank line before the @list suffix.
    #[test]
    fn rlva_im_list_follows_the_legacy_layout() {
        let mut world = world();
        let target = Uuid::from_u128(3);
        world.social.avatar_names.on_legacy(target, "Test", "Smith");
        world.rlva.preferences.im_queries_auto_share = true;
        command(&mut world, &format!("@sendim=n,fly=n,recvchat:{target}=n"));
        query(&mut world, "@list", false);
        assert_eq!(
            replies(&world),
            format!("{VERSION}\n\n{ISSUER}:\n  -> sendim\n  -> fly\n\n{}", remote(Remote::ListSuffix))
        );
        world.rlva.out.clear();
        query(&mut world, "@except", false);
        assert_eq!(
            replies(&world),
            format!("{VERSION}\n\n{ISSUER}:\n  -> recvchat:{target}  [test.smith]\n")
        );
    }

    /// RlvHandler::processIMQuery asks once per query, however many wait,
    /// and onIMQueryListResponse answers even once the IM query setting is
    /// off.
    #[test]
    fn rlva_im_list_questions_are_never_merged_capped_or_cancelled_by_the_setting() {
        let mut world = world();
        for _ in 0..40 {
            query(&mut world, "@list", false);
        }
        let ids: Vec<_> = world
            .notifications
            .list
            .iter()
            .filter(|n| matches!(n.data, Data::RlvaQuery { .. }))
            .map(|n| n.id)
            .collect();
        assert_eq!(ids.len(), 40);
        assert!(world.rlva.out.is_empty(), "no automatic refusal");
        world.rlva.preferences.im_queries = false;
        world.respond_notification(ids[0], Response::Accept);
        assert!(replies(&world).starts_with(VERSION));
    }
}
