//! RlvHandler::processIMQuery/onIMQueryListResponse and RlvFloaterBehaviours
//! formatting (rlvhandler.cpp / rlvfloaters.cpp, originally LGPL 2.1).
use super::{
    super::{
        World,
        notifications::{Data, Kind, NameTarget, Response},
    },
    VERSION,
};
use aurora_net::{InstantMessage, NetCommand};
use uuid::Uuid;

#[derive(Default)]
pub struct Queries {
    pub close: Vec<Uuid>,
    pub remember: bool,
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

fn chunks(mut text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    while !text.is_empty() {
        let mut cut = text.len().min(1023);
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut < text.len()
            && let Some(newline) = text[..cut].rfind('\n').filter(|index| *index > 0)
        {
            cut = newline + 1;
        }
        result.push(&text[..cut]);
        text = &text[cut..];
    }
    result
}

impl World {
    fn rlva_im_reply(&mut self, to: Uuid, message: &str, busy: bool) {
        let offline = self.social.friends.iter().any(|friend| friend.id == to && !friend.online);
        for chunk in chunks(message) {
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
                self.rlva_im_reply(from, aurora_i18n::tr!("rlva-im-ended-remote"), true);
                if self.social.has_session(&from) {
                    self.conversation_closed(&from);
                    self.leave_session(from);
                    self.rlva.im.close.push(from);
                    let name = if self.rlva_can_name(from) {
                        im.from_name.as_str()
                    } else {
                        super::privacy::anonymous(from).display_name.as_str()
                    };
                    self.system_message(aurora_i18n::tr!("rlva-im-ended-local", name = name));
                }
                return true;
            }
            if !self.rlva.preferences.im_queries {
                return false;
            }
            self.rlva_im_reply(from, aurora_i18n::tr!("rlva-im-no-restriction"), true);
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
        if self.rlva.preferences.im_queries_auto_share {
            self.rlva_im_list(from, exceptions);
        } else if !self
            .notifications
            .list
            .iter()
            .any(|n| matches!(n.data, Data::RlvaQuery { from: sender, exceptions: kind } if sender == from && kind == exceptions))
        {
            if self
                .notifications
                .list
                .iter()
                .filter(|n| matches!(n.data, Data::RlvaQuery { .. }))
                .count()
                >= 32
            {
                self.rlva_im_reply(from, aurora_i18n::tr!("rlva-im-denied"), true);
                return true;
            }
            let name = if self.rlva_can_name(from) {
                im.from_name.as_str()
            } else {
                super::privacy::anonymous(from).display_name.as_str()
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

    fn rlva_im_list(&mut self, to: Uuid, exceptions: bool) {
        let mut message = format!("{VERSION}\n");
        'issuers: for (source, rules) in &self.rlva.rules {
            let name = self
                .rlva_attachment_item(*source)
                .and_then(|id| self.inventory.items.get(&id))
                .map(|item| item.name.clone())
                .unwrap_or_else(|| source.to_string());
            message.push_str(&format!("\n{name}:\n"));
            for rule in rules {
                if (!rule.option.is_empty() && exception(&rule.behaviour)) != exceptions {
                    continue;
                }
                if message.len() + rule.behaviour.len() + rule.option.len() + 16 > 250_000 {
                    break 'issuers;
                }
                message.push_str(&format!("  -> {}", super::rule_text(rule)));
                if let Some(id) = Uuid::parse_str(&rule.option).ok().filter(|id| !id.is_nil()) {
                    let name = self
                        .rlva_attachment_item(id)
                        .and_then(|item| self.inventory.items.get(&item))
                        .map(|item| item.name.clone())
                        .or_else(|| self.social.avatar_names.complete(&id))
                        .or_else(|| self.search.groups.get(&id).map(|group| group.name.clone()));
                    if let Some(name) = name.filter(|name| name != &rule.option) {
                        message.push_str(&format!("  [{name}]"));
                    }
                }
                message.push('\n');
            }
        }
        if !exceptions {
            message.push_str(aurora_i18n::tr!("rlva-im-list-suffix"));
        }
        self.rlva_im_reply(to, &message, false);
    }

    pub(crate) fn rlva_answer_im_query(&mut self, from: Uuid, exceptions: bool, response: &Response) {
        if !self.rlva.enabled() || !self.rlva.preferences.im_queries {
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
            _ => self.rlva_im_reply(from, aurora_i18n::tr!("rlva-im-denied"), true),
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
    fn rlva_im_list_requires_consent_deduplicates_prompts_and_separates_exceptions() {
        let mut world = world();
        let target = Uuid::from_u128(3);
        world.social.avatar_names.on_legacy(target, "Test", "Resident");
        command(&mut world, &format!("@fly=n,recvchat=n,recvchat:{target}=n,sendim=n"));
        query(&mut world, "@list", false);
        query(&mut world, "@list", false);
        assert!(world.rlva.out.is_empty());
        assert_eq!(
            world
                .notifications
                .list
                .iter()
                .filter(|n| matches!(n.data, Data::RlvaQuery { .. }))
                .count(),
            1
        );
        let id = world
            .notifications
            .list
            .iter()
            .find(|n| matches!(n.data, Data::RlvaQuery { .. }))
            .unwrap()
            .id;
        world.respond_notification(id, Response::Decline);
        assert!(matches!(world.rlva.out.pop(), Some(NetCommand::ImReply { busy: true, .. })));
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
        let text = format!("{}\n{}\n{}", "a".repeat(800), "é".repeat(800), "z".repeat(800));
        let split = chunks(&text);
        assert_eq!(split.concat(), text);
        assert!(split.iter().all(|part| part.len() <= 1023 && !part.is_empty()));
        assert!(split[0].ends_with('\n'));
    }
}
