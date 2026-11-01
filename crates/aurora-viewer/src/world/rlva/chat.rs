//! RlvHandler::filterChat and process_chat_from_simulator
//! (rlvhandler.cpp, llviewermessage.cpp, LGPL 2.1).
use super::{Rlva, is_emote};
use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetCommand};
use std::collections::BTreeSet;
use uuid::Uuid;
impl Rlva {
    /// RlvActions::canSendTypingStart: gates animation, agent state and chat.
    pub fn can_send_typing_start(&self) -> bool {
        !self.enabled || !self.has_options("redirchat") || self.preferences.show_redirect_typing
    }
    pub fn receive_chat(&self, chat: &mut ChatMessage, own: Uuid) -> bool {
        if !self.enabled
            || chat.source_id == own
            || chat.source_type == ChatSourceType::System
            || (chat.source_type == ChatSourceType::Object && matches!(chat.chat_type, ChatType::OwnerSay | ChatType::Other(9)))
        {
            return true;
        }
        let emote = is_emote(&chat.message);
        let (all, specific) = if emote {
            ("recvemote", "recvemotefrom")
        } else {
            ("recvchat", "recvchatfrom")
        };
        let id = chat.source_id.to_string();
        if !self.allows(all, &id) || self.targets(specific, &id) {
            if emote {
                if !self.preferences.show_ellipsis {
                    return false;
                }
                chat.message = "/me ...".into();
            } else {
                let (message, filtered) = filtered_chat(
                    &chat.message,
                    false,
                    false,
                    self.preferences.allow_ooc,
                    self.preferences.show_ellipsis,
                );
                if filtered && !self.preferences.show_ellipsis {
                    return false;
                }
                chat.message = message;
            }
        }
        true
    }

    /// Transform one chat line into zero or more sends. Redirection is checked
    /// before sendchat, and redirected text never returns to public chat.
    pub fn outgoing_chat(&self, message: &str, channel: i32, kind: ChatType) -> Vec<NetCommand> {
        let make = |message: String, channel, chat_type| NetCommand::Chat {
            message,
            channel,
            chat_type,
        };
        if !self.enabled {
            return vec![make(message.to_owned(), channel, kind)];
        }
        if channel == i32::MAX {
            // CHAT_CHANNEL_DEBUG shows as public chat on viewers
            // (send_chat_from_viewer, llfloaterimnearbychat.cpp).
            let redirect = if is_emote(message) { "rediremote" } else { "redirchat" };
            if self.has("sendchat") || self.has_options(redirect) {
                return Vec::new();
            }
        }
        if channel != 0 {
            return if self.allows_channel(channel) {
                vec![make(message.to_owned(), channel, kind)]
            } else {
                Vec::new()
            };
        }
        if matches!(kind, ChatType::StartTyping | ChatType::StopTyping) {
            return if kind == ChatType::StartTyping && !self.can_send_typing_start() {
                Vec::new()
            } else {
                vec![make(message.to_owned(), channel, kind)]
            };
        }
        let emote = is_emote(message);
        let redirect = if emote { "rediremote" } else { "redirchat" };
        if self.has_options(redirect)
            && (emote || filtered_chat(message, false, false, self.preferences.allow_ooc, self.preferences.show_ellipsis).1)
        {
            let channels: BTreeSet<_> = self
                .rules
                .values()
                .flat_map(|rs| rs.iter())
                .filter(|r| r.behaviour == redirect)
                .filter_map(|r| r.option.parse::<i32>().ok())
                .filter(|c| self.allows_channel(*c))
                .collect();
            let parts = if self.preferences.split_redirect_chat {
                chunks(message)
            } else {
                vec![message]
            };
            return channels
                .into_iter()
                .flat_map(|channel| {
                    parts.iter().map(move |message| NetCommand::ChatReply {
                        channel,
                        message: (*message).into(),
                    })
                })
                .collect();
        }
        let text = if self.has("sendchat") {
            filtered_chat(
                message,
                true,
                self.has("emote"),
                self.preferences.allow_ooc,
                self.preferences.show_ellipsis,
            )
            .0
        } else {
            message.to_owned()
        };
        if text.is_empty() {
            Vec::new()
        } else {
            vec![make(text, 0, self.chat_volume(kind))]
        }
    }

    /// RlvActions::checkChatVolume (rlvactions.cpp): one step, never a drop.
    /// `@chatnormal` turns shout and normal into a whisper, whatever else is
    /// set; otherwise `@chatshout` and `@chatwhisper` fall back to normal.
    fn chat_volume(&self, kind: ChatType) -> ChatType {
        match kind {
            ChatType::Shout | ChatType::Normal if self.has("chatnormal") => ChatType::Whisper,
            ChatType::Shout if self.has("chatshout") => ChatType::Normal,
            ChatType::Whisper if self.has("chatwhisper") => ChatType::Normal,
            kind => kind,
        }
    }

    fn has_options(&self, name: &str) -> bool {
        self.enabled && self.rules.values().any(|rs| rs.iter().any(|r| r.behaviour == name))
    }
}

fn chunks(mut text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    while !text.is_empty() {
        let mut cut = text.len().min(1023);
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut < text.len()
            && text.as_bytes().get(cut) != Some(&b' ')
            && let Some(space) = text[..cut].rfind(' ').filter(|index| *index > 0)
        {
            cut = space;
        }
        parts.push(&text[..cut]);
        text = &text[cut..];
        if let Some(rest) = text.strip_prefix(' ') {
            text = rest;
        }
    }
    parts
}

fn filtered_chat(text: &str, filter_emote: bool, allow_long_emote: bool, allow_ooc: bool, show_ellipsis: bool) -> (String, bool) {
    if text.is_empty() {
        return (String::new(), false);
    }
    let filtered = || (if show_ellipsis { "..." } else { "" }.to_owned(), true);
    if is_emote(text) {
        if !filter_emote {
            return (text.to_owned(), false);
        }
        if text.contains(['"', '(', ')', '*', '=', '^', '_', '?', '~']) || text.contains(" -") || text.contains("- ") || text.contains("''")
        {
            return filtered();
        }
        if !allow_long_emote {
            return (
                text.chars()
                    .take(
                        text.chars()
                            .position(|c| c == '.')
                            .filter(|i| *i > 0 && *i < 20)
                            .map_or(20, |i| i + 1),
                    )
                    .collect(),
                false,
            );
        }
        return (text.to_owned(), false);
    }
    if (text.starts_with('/') && text.chars().count() <= 7)
        || (allow_ooc && text.len() >= 4 && text.starts_with("((") && text.ends_with("))"))
    {
        (text.to_owned(), false)
    } else {
        filtered()
    }
}

impl crate::world::World {
    pub(crate) fn rlva_demo_chat(&mut self) {
        let source = Uuid::from_u128(0x726c766163686174);
        for (source_type, chat_type, text) in [
            (ChatSourceType::Agent, ChatType::Normal, "((RLVa OOC demo))"),
            (ChatSourceType::Agent, ChatType::Normal, "RLVa ordinary demo"),
            (ChatSourceType::Agent, ChatType::Normal, "/me waves in the RLVa demo"),
            (ChatSourceType::Object, ChatType::Other(9), "RLVa directed demo"),
        ] {
            self.apply(aurora_net::NetEvent::Chat(ChatMessage {
                source_id: source,
                from_name: "RLVa Chat Fixture".into(),
                owner_id: Uuid::nil(),
                source_type,
                chat_type,
                position: glam::Vec3::ZERO,
                message: text.into(),
            }));
        }
        log::info!(
            "RLVa chat demo: ooc={}, direct={}, filtered_rows={}",
            self.chat.iter().any(|line| line.text == "((RLVa OOC demo))"),
            self.chat.iter().any(|line| line.text == "RLVa directed demo"),
            self.chat
                .iter()
                .filter(|line| line.source == source && matches!(line.text.as_str(), "..." | "/me ..."))
                .count()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(message: &str) -> ChatMessage {
        ChatMessage {
            from_name: "Controller".into(),
            source_id: Uuid::from_u128(1),
            owner_id: Uuid::from_u128(2),
            source_type: ChatSourceType::Agent,
            chat_type: ChatType::Normal,
            position: glam::Vec3::ZERO,
            message: message.into(),
        }
    }
    fn restricted(text: &str) -> Rlva {
        let mut r = Rlva::default();
        r.configure(true);
        let mut command = chat(text);
        command.source_type = ChatSourceType::Object;
        command.chat_type = ChatType::OwnerSay;
        r.receive(&command);
        r
    }

    #[test]
    fn rlva_received_chat_keeps_native_ooc_commands_direct_messages_and_ellipsis_options() {
        let mut r = restricted("@recvchat=n,recvemote=n");
        for text in ["((OOC))", "/test"] {
            let mut message = chat(text);
            assert!(r.receive_chat(&mut message, Uuid::nil()));
            assert_eq!(message.message, text);
        }
        let mut ordinary = chat("ordinary chat");
        assert!(r.receive_chat(&mut ordinary, Uuid::nil()));
        assert_eq!(ordinary.message, "...");
        let mut emote = chat("/me waves");
        r.receive_chat(&mut emote, Uuid::nil());
        assert_eq!(emote.message, "/me ...");
        r.preferences.allow_ooc = false;
        let mut ooc = chat("((OOC))");
        r.receive_chat(&mut ooc, Uuid::nil());
        assert_eq!(ooc.message, "...");
        r.preferences.show_ellipsis = false;
        assert!(!r.receive_chat(&mut chat("ordinary chat"), Uuid::nil()));
        assert!(!r.receive_chat(&mut chat("/me waves"), Uuid::nil()));
        let mut direct = chat("directed object message");
        direct.source_type = ChatSourceType::Object;
        direct.chat_type = ChatType::Other(9);
        assert!(r.receive_chat(&mut direct, Uuid::nil()));
        assert_eq!(direct.message, "directed object message");
        direct.chat_type = ChatType::OwnerSay;
        assert!(r.receive_chat(&mut direct, Uuid::nil()));
    }

    #[test]
    fn rlva_chat_redirection_uses_protocol_packets_and_splits_only_when_requested() {
        let mut r = restricted("@redirchat:123=n,sendchat=n");
        assert!(!r.can_send_typing_start());
        assert!(r.outgoing_chat("", 0, ChatType::StartTyping).is_empty());
        assert!(matches!(
            &r.outgoing_chat("", 0, ChatType::StopTyping)[..],
            [NetCommand::Chat {
                chat_type: ChatType::StopTyping,
                ..
            }]
        ));
        r.preferences.show_redirect_typing = true;
        assert!(r.can_send_typing_start());
        assert!(matches!(
            &r.outgoing_chat("", 0, ChatType::StartTyping)[..],
            [NetCommand::Chat {
                chat_type: ChatType::StartTyping,
                ..
            }]
        ));
        assert!(
            matches!(&r.outgoing_chat("...", 0, ChatType::Normal)[..], [NetCommand::ChatReply { channel: 123, message }] if message == "..."),
            "redirect eligibility is the filter result, not string inequality"
        );
        let text = format!("{} {}", "é".repeat(500), "word ".repeat(300));
        assert!(
            matches!(
                &r.outgoing_chat(&text, 0, ChatType::Normal)[..],
                [NetCommand::ChatReply { channel: 123, .. }]
            ),
            "the default uses one bounded reply packet"
        );
        r.preferences.split_redirect_chat = true;
        let commands = r.outgoing_chat(&text, 0, ChatType::Normal);
        assert!(commands.len() > 1);
        assert!(commands.iter().all(
            |command| matches!(command, NetCommand::ChatReply { channel: 123, message } if message.len() <= 1023 && !message.is_empty())
        ));
        assert_eq!(chunks(&"é".repeat(1200)).concat(), "é".repeat(1200));
        assert_eq!(chunks(&format!("{} next", "x".repeat(1023))), ["x".repeat(1023), "next".into()]);
        assert!(matches!(
            &r.outgoing_chat("((OOC))", 0, ChatType::Normal)[..],
            [NetCommand::Chat { channel: 0, .. }]
        ));
        r.preferences.allow_ooc = false;
        assert!(matches!(
            &r.outgoing_chat("((OOC))", 0, ChatType::Normal)[..],
            [NetCommand::ChatReply { channel: 123, .. }]
        ));
        let mut r = restricted("@sendchat=n");
        r.preferences.show_ellipsis = false;
        assert!(r.outgoing_chat("ordinary chat", 0, ChatType::Normal).is_empty());
        assert!(r.outgoing_chat("", 0, ChatType::Normal).is_empty());
    }

    #[test]
    fn rlva_chat_volume_follows_the_legacy_table_and_never_drops_a_line() {
        use ChatType::{Normal, Shout, Whisper};
        let sent = |r: &Rlva, kind| match &r.outgoing_chat("hello", 0, kind)[..] {
            [NetCommand::Chat { chat_type, .. }] => *chat_type,
            other => panic!("one chat line expected, got {other:?}"),
        };
        // RlvActions::checkChatVolume: input × restrictions → sent volume
        for (restrictions, whisper, normal, shout) in [
            ("@chatwhisper=n", Normal, Normal, Shout),
            ("@chatnormal=n", Whisper, Whisper, Whisper),
            ("@chatshout=n", Whisper, Normal, Normal),
            ("@chatnormal=n,chatwhisper=n", Normal, Whisper, Whisper),
            ("@chatnormal=n,chatshout=n", Whisper, Whisper, Whisper),
            ("@chatshout=n,chatwhisper=n", Normal, Normal, Normal),
            ("@chatshout=n,chatnormal=n,chatwhisper=n", Normal, Whisper, Whisper),
        ] {
            let r = restricted(restrictions);
            assert_eq!(
                [sent(&r, Whisper), sent(&r, Normal), sent(&r, Shout)],
                [whisper, normal, shout],
                "{restrictions}"
            );
        }
    }

    #[test]
    fn rlva_debug_channel_closes_under_sendchat_and_redirection() {
        const DEBUG: i32 = i32::MAX;
        let open = |r: &Rlva, text| !r.outgoing_chat(text, DEBUG, ChatType::Normal).is_empty();
        assert!(open(&restricted("@fly=n"), "debug"));
        assert!(!open(&restricted("@sendchat=n"), "debug"));
        assert!(!open(&restricted("@sendchat=n"), "/me debug"));
        let r = restricted("@redirchat:123=n");
        assert!(!open(&r, "debug"));
        assert!(open(&r, "/me debug"), "an emote is only closed by @rediremote");
        let r = restricted("@rediremote:123=n");
        assert!(open(&r, "debug"));
        assert!(!open(&r, "/me debug"));
        assert!(
            !restricted("@sendchat=n").outgoing_chat("ordinary", 5, ChatType::Normal).is_empty(),
            "other channels keep @sendchannel's rule"
        );
    }

    #[test]
    fn rlva_native_chat_path_exempts_own_attachments_and_drops_hidden_messages_before_history() {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.rlva = restricted("@recvchat=n,recvemote=n");
        world.rlva.preferences.show_ellipsis = false;
        let before = world.chat.len();
        world.apply(aurora_net::NetEvent::Chat(chat("hidden ordinary chat")));
        world.apply(aurora_net::NetEvent::Chat(chat("/me hidden emote")));
        assert_eq!(world.chat.len(), before);
        world.apply(aurora_net::NetEvent::Chat(chat("((visible OOC))")));
        assert_eq!(world.chat.back().unwrap().text, "((visible OOC))");
        let attachment = Uuid::from_u128(101);
        let update = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: attachment,
            owner_id: world.agent_id,
            point: 1,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: "Owned attachment".into(),
            desc: String::new(),
        });
        world.objects.upsert(world.main_region.unwrap(), update);
        let mut message = chat("owned attachment chat");
        message.source_type = ChatSourceType::Object;
        message.source_id = attachment;
        message.owner_id = world.agent_id;
        world.apply(aurora_net::NetEvent::Chat(message.clone()));
        assert_eq!(world.chat.back().unwrap().text, "owned attachment chat");
        message.source_id = Uuid::from_u128(102);
        message.chat_type = ChatType::Other(9);
        message.message = "directed object chat".into();
        world.apply(aurora_net::NetEvent::Chat(message));
        assert_eq!(world.chat.back().unwrap().text, "directed object chat");
    }
}
