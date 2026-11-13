//! Conversations open at logout, kept per account following
//! FSFloaterIMContainer::saveOpenIMs / restoreOpenIMs (fsfloaterimcontainer.cpp,
//! originally LGPL 2.1). Aurora also restores group chats (#1683), using
//! their membership names and ordinary group-session start protocol.
//! Conferences cannot be rejoined by their old session id and are excluded.
//! The list is always written; the setting decides whether it is reopened.

use aurora_llsd::Llsd;
use std::path::Path;
use uuid::Uuid;

/// Per account file, extending the LLSD array of FSLastOpenIMs.
pub const FILE: &str = "open_ims.xml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversation {
    Resident(Uuid),
    Group(Uuid),
    /// Older Aurora files also put group ids in other_participant_id.
    Legacy(Uuid),
}

impl Conversation {
    fn id(self) -> Uuid {
        match self {
            Self::Resident(id) | Self::Group(id) | Self::Legacy(id) => id,
        }
    }
}

/// The conversations to reopen, in their saved order.
pub fn load(path: &Path) -> Vec<Conversation> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("open conversations not read: {e}");
            }
            return Vec::new();
        }
    };
    match aurora_llsd::from_xml(&bytes) {
        Ok(data) => parse(&data),
        Err(e) => {
            log::warn!("open conversations unreadable: {e:?}");
            Vec::new()
        }
    }
}

/// Unknown kinds, malformed entries, nil ids and duplicates are skipped.
fn parse(data: &Llsd) -> Vec<Conversation> {
    let mut out: Vec<Conversation> = Vec::new();
    for entry in data.as_array() {
        if !entry.is_map() {
            continue;
        }
        let conversation = match entry.get("kind") {
            Llsd::String(kind) if kind == "group" => Conversation::Group(entry.get("group_id").as_uuid()),
            Llsd::String(kind) if kind == "resident" => Conversation::Resident(entry.get("other_participant_id").as_uuid()),
            Llsd::Undef => Conversation::Legacy(entry.get("other_participant_id").as_uuid()),
            _ => continue,
        };
        if !conversation.id().is_nil() && !out.iter().any(|c| c.id() == conversation.id()) {
            out.push(conversation);
        }
    }
    out
}

pub fn save(path: &Path, conversations: &[Conversation]) {
    let list: Vec<Llsd> = conversations
        .iter()
        .map(|conversation| {
            let mut entry = aurora_llsd::Map::new();
            let field = match conversation {
                Conversation::Resident(_) => {
                    entry.insert("kind".into(), Llsd::from("resident"));
                    "other_participant_id"
                }
                Conversation::Group(_) => {
                    entry.insert("kind".into(), Llsd::from("group"));
                    "group_id"
                }
                Conversation::Legacy(_) => "other_participant_id",
            };
            entry.insert(field.into(), Llsd::from(conversation.id()));
            Llsd::Map(entry)
        })
        .collect();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, aurora_llsd::to_xml(&Llsd::Array(list))) {
        log::warn!("open conversations not saved: {e}");
    }
}

impl super::World {
    /// A deliberate close/snooze takes precedence over deferred login work.
    pub(super) fn cancel_saved_im(&mut self, session: Uuid) {
        self.reopen_ims.retain(|saved| saved.id() != session);
    }

    /// Save only rejoinable conversations, with the kind needed at login.
    pub fn save_open_ims(&self) {
        let Some(path) = &self.open_ims_path else { return };
        let mut conversations: Vec<Conversation> = self
            .social
            .ims
            .iter()
            .filter_map(|s| match self.groups.sessions.get(&s.other) {
                Some(info) if info.group => Some(Conversation::Group(s.other)),
                Some(_) => None,
                None if self.groups.is_member(&s.other) => Some(Conversation::Group(s.other)),
                None => Some(Conversation::Resident(s.other)),
            })
            .collect();
        // A quick logout must not lose conversations still waiting for
        // the login's group/mute replies.
        for pending in &self.reopen_ims {
            if !conversations.iter().any(|c| c.id() == pending.id()) {
                conversations.push(*pending);
            }
        }
        save(path, &conversations);
    }

    /// Called each frame: group membership and mutes may arrive after login.
    pub fn restore_open_ims(&mut self, enabled: bool) {
        if !enabled {
            let previews: Vec<Uuid> = self.groups.restoring.iter().copied().collect();
            for preview in previews {
                self.discard_restoring_group(preview);
            }
            self.reopen_ims.clear();
            return;
        }
        let mut pending = std::mem::take(&mut self.reopen_ims);
        pending.retain(|&saved| {
            let id = saved.id();
            let group = match saved {
                Conversation::Resident(_) => false,
                Conversation::Group(_) => {
                    if !self.groups.is_member(&id) {
                        return true;
                    }
                    true
                }
                Conversation::Legacy(_) => {
                    if !self.groups.list_received {
                        return true;
                    }
                    self.groups.is_member(&id)
                }
            };
            if group {
                if !self.mutes.is_loaded() {
                    self.prepare_saved_group_chat(id);
                    return true;
                }
                self.restore_saved_group_chat(id);
            } else {
                self.social.session_mut(id);
            }
            false
        });
        self.reopen_ims = pending;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{World, groups::dialog};
    use aurora_net::{GroupMembership, NetCommand, NetEvent};

    fn world() -> World {
        World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()))
    }

    fn membership(id: Uuid) -> GroupMembership {
        GroupMembership {
            id,
            name: format!("Group {id}"),
            insignia: Uuid::nil(),
            powers: 1 << 16,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }
    }

    #[test]
    fn the_list_comes_back_in_its_order_without_bad_entries() {
        let dir = std::env::temp_dir().join(format!("aurora-open-ims-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        assert!(load(&path).is_empty(), "no file yet");
        let (a, b) = (Uuid::from_u128(2), Uuid::from_u128(1));
        let conversations = [Conversation::Resident(a), Conversation::Group(b)];
        save(&path, &conversations);
        assert_eq!(load(&path), conversations);
        // a nil resident or a stray value is skipped
        let mut nil = aurora_llsd::Map::new();
        nil.insert("other_participant_id".into(), Llsd::from(Uuid::nil()));
        let mut good = aurora_llsd::Map::new();
        good.insert("other_participant_id".into(), Llsd::from(a));
        let data = Llsd::Array(vec![
            Llsd::Map(nil),
            Llsd::from(3),
            Llsd::Map(good.clone()),
            Llsd::Map(good.clone()),
        ]);
        assert_eq!(parse(&data), vec![Conversation::Legacy(a)]);
        good.insert("kind".into(), Llsd::from("conference"));
        assert!(parse(&Llsd::Array(vec![Llsd::Map(good.clone())])).is_empty());
        good.insert("kind".into(), Llsd::from(42));
        assert!(parse(&Llsd::Array(vec![Llsd::Map(good)])).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn open_ims_save_distinguishes_groups_and_excludes_conferences() {
        let mut w = world();
        let (resident, group, conference, pending) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        let dir = std::env::temp_dir().join(format!("aurora-open-ims-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        w.open_ims_path = Some(path.clone());
        w.social.session_mut(resident);
        w.on_group_event(NetEvent::Groups(vec![membership(group)]));
        w.info_mut(group, "");
        w.session_lines(group);
        w.info_mut(conference, "Conference");
        w.session_lines(conference);
        w.reopen_ims = vec![Conversation::Group(group), Conversation::Group(pending)];
        w.save_open_ims();
        assert_eq!(
            load(&path),
            [
                Conversation::Resident(resident),
                Conversation::Group(group),
                Conversation::Group(pending)
            ]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn open_ims_groups_appear_before_the_mute_reply_and_only_then_join() {
        let mut w = world();
        let (resident, group, old_group) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        w.social.focus_im = Some(resident);
        w.reopen_ims = vec![
            Conversation::Resident(resident),
            Conversation::Group(group),
            Conversation::Legacy(old_group),
        ];
        w.restore_open_ims(true);
        assert_eq!(w.social.ims.iter().map(|s| s.other).collect::<Vec<_>>(), [resident]);
        assert_eq!(w.social.avatar_names.take_asks(), [resident]);
        w.on_group_event(NetEvent::Groups(vec![membership(group), membership(old_group)]));
        w.restore_open_ims(true);
        for id in [group, old_group] {
            assert!(w.is_group_session(&id), "show groups without waiting for the block list");
            assert_eq!(w.session_title(&id), membership(id).name);
            assert!(w.groups.restoring.contains(&id));
            assert!(!w.groups.sessions[&id].started_by_us);
        }
        assert!(w.groups.take_commands().is_empty(), "displaying a group never joins it early");
        w.mutes.on_server(aurora_net::MuteListSource::UseCache, None);
        w.restore_open_ims(true);
        assert!(w.reopen_ims.is_empty());
        assert!(w.social.avatar_names.take_asks().is_empty(), "group ids are not avatar names");
        assert_eq!(w.social.focus_im, Some(resident));
        assert!(w.im_messages.is_empty(), "restoration is quiet");
        for id in [group, old_group] {
            assert!(w.is_group_session(&id));
            assert_eq!(w.session_title(&id), membership(id).name);
            assert!(w.social.ims.iter().find(|s| s.other == id).unwrap().lines.is_empty());
            assert!(w.groups.sessions[&id].started_by_us);
            assert!(!w.groups.restoring.contains(&id));
        }
        let commands = w.groups.take_commands();
        assert_eq!(commands.len(), 2);
        for (command, id) in commands.iter().zip([group, old_group]) {
            assert!(
                matches!(command, NetCommand::SendImDialog { to, dialog: d, id: session, message, .. }
                if *to == id && *d == dialog::SESSION_GROUP_START && *session == id && message.is_empty())
            );
        }
        // The ordinary acknowledgement still fetches history, once.
        for _ in 0..2 {
            w.on_group_event(NetEvent::SessionStarted {
                temp_session: group,
                session: group,
                success: true,
                error: String::new(),
                agents: Vec::new(),
            });
        }
        assert!(
            matches!(w.groups.take_commands()[..], [NetCommand::ChatSession { method: aurora_net::SessionMethod::FetchHistory, session }] if session == group)
        );
        w.restore_open_ims(true);
        assert!(w.groups.take_commands().is_empty());
        let mut renamed = membership(group);
        renamed.name = "Renamed group".into();
        w.on_group_event(NetEvent::Groups(vec![renamed]));
        assert_eq!(w.session_title(&group), "Renamed group");
    }

    #[test]
    fn open_ims_legacy_groups_are_repaired_when_a_later_membership_packet_arrives() {
        let mut w = world();
        let id = Uuid::from_u128(42);
        w.reopen_ims = vec![Conversation::Legacy(id)];
        w.mutes.on_server(aurora_net::MuteListSource::UseCache, None);
        w.on_group_event(NetEvent::Groups(Vec::new()));
        w.restore_open_ims(true);
        assert!(w.social.has_session(&id));
        w.on_group_event(NetEvent::Groups(vec![membership(id)]));
        w.restore_open_ims(true);
        assert!(w.is_group_session(&id));
        assert_eq!(w.session_title(&id), membership(id).name);
        assert_eq!(w.social.ims.len(), 1);
        assert_eq!(w.groups.take_commands().len(), 1);
    }

    #[test]
    fn open_ims_late_membership_also_retires_a_blocked_legacy_placeholder() {
        let mut w = world();
        let id = Uuid::from_u128(42);
        w.reopen_ims = vec![Conversation::Legacy(id)];
        w.mutes.on_server(aurora_net::MuteListSource::UseCache, None);
        w.groups.mute_all_groups = true;
        w.on_group_event(NetEvent::Groups(Vec::new()));
        w.restore_open_ims(true);
        assert!(w.social.has_session(&id));
        w.on_group_event(NetEvent::Groups(vec![membership(id)]));
        w.restore_open_ims(true);
        assert!(!w.social.has_session(&id));
        assert!(w.groups.take_commands().is_empty());
    }

    #[test]
    fn open_ims_restore_respects_blocks_disabled_setting_and_already_open_sessions() {
        for filter in 0..4 {
            let mut w = world();
            let id = Uuid::from_u128(42);
            let mut group = membership(id);
            group.accept_notices = false;
            w.on_group_event(NetEvent::Groups(vec![group]));
            w.mutes.on_server(aurora_net::MuteListSource::UseCache, None);
            match filter {
                0 => w.groups.mute_all_groups = true,
                1 => w.groups.mute_when_notices_off = true,
                2 => {
                    w.set_group_chat_blocked(id, true);
                    w.groups.take_commands();
                }
                _ => {}
            }
            w.reopen_ims = vec![Conversation::Group(id)];
            w.restore_open_ims(filter != 3);
            assert!(w.social.ims.is_empty());
            assert!(w.groups.take_commands().is_empty());
            assert!(w.reopen_ims.is_empty());
        }
        let mut w = world();
        let id = Uuid::from_u128(42);
        w.on_group_event(NetEvent::Groups(vec![membership(id)]));
        w.mutes.on_server(aurora_net::MuteListSource::UseCache, None);
        w.start_group_chat(id, false);
        w.groups.take_commands();
        w.reopen_ims = vec![Conversation::Group(id)];
        w.restore_open_ims(true);
        assert_eq!(w.social.ims.len(), 1);
        assert!(w.groups.take_commands().is_empty(), "an incoming/manual session already opened it");
        w.reopen_ims = vec![Conversation::Group(Uuid::from_u128(99))];
        w.restore_open_ims(true);
        assert_eq!(w.social.ims.len(), 1, "no resident placeholder for a group no longer joined");
        assert!(w.social.avatar_names.take_asks().is_empty());
        w.restore_open_ims(false);
        assert!(w.reopen_ims.is_empty());
        // A user can open and close/snooze a group before the mute reply
        // arrives. Its still-pending login entry must not reopen it later.
        for snooze in [false, true] {
            let mut w = world();
            w.on_group_event(NetEvent::Groups(vec![membership(id)]));
            w.reopen_ims = vec![Conversation::Group(id)];
            w.restore_open_ims(true);
            w.on_group_event(NetEvent::SessionInvite(aurora_net::SessionInvite {
                session: id,
                from: Uuid::from_u128(7),
                from_name: "Sender".into(),
                message: "Deferred".into(),
                session_name: membership(id).name,
                offline: false,
                timestamp: 0,
            }));
            if snooze {
                assert!(w.snooze_group_chat(id, 60));
            } else {
                w.leave_session(id);
            }
            assert!(w.reopen_ims.is_empty());
            w.groups.take_commands();
            w.on_mute_list(aurora_net::MuteListSource::UseCache);
            w.restore_open_ims(true);
            assert!(!w.social.has_session(&id));
            assert!(w.groups.take_commands().is_empty());
        }
        let mut w = world();
        w.on_group_event(NetEvent::Groups(vec![membership(id)]));
        w.reopen_ims = vec![Conversation::Group(id)];
        w.restore_open_ims(true);
        assert!(w.social.has_session(&id));
        w.restore_open_ims(false);
        w.on_mute_list(aurora_net::MuteListSource::UseCache);
        assert!(!w.social.has_session(&id));
        assert!(w.groups.restoring.is_empty());
        assert!(w.reopen_ims.is_empty());
        assert!(w.groups.take_commands().is_empty());
    }

    #[test]
    fn open_ims_pending_groups_drop_live_traffic_and_disappear_if_the_mute_reply_blocks_them() {
        let mut w = world();
        let group = Uuid::from_u128(42);
        let sender = Uuid::from_u128(7);
        w.on_group_event(NetEvent::Groups(vec![membership(group)]));
        w.reopen_ims = vec![Conversation::Group(group)];
        w.restore_open_ims(true);
        assert_eq!(w.session_title(&group), membership(group).name);
        w.on_group_event(NetEvent::SessionInvite(aurora_net::SessionInvite {
            session: group,
            from: sender,
            from_name: "Sender".into(),
            message: "Not accepted yet".into(),
            session_name: membership(group).name,
            offline: false,
            timestamp: 0,
        }));
        let message = aurora_net::InstantMessage {
            from_agent_id: sender,
            from_name: "Sender".into(),
            to_agent_id: w.agent_id,
            dialog: dialog::SESSION_SEND,
            session_id: group,
            message: "Not displayed yet".into(),
            offline: false,
            from_group: true,
            binary_bucket: Vec::new(),
            timestamp: 0,
        };
        assert!(w.on_session_im(&message));
        assert!(w.social.ims[0].lines.is_empty());
        assert!(
            w.groups.take_commands().is_empty(),
            "no join, accept or history request before mutes"
        );
        assert!(w.im_messages.is_empty());
        w.on_mute_list(aurora_net::MuteListSource::File(format!("0 {} Group:{group}|\n", Uuid::nil())));
        w.restore_open_ims(true);
        assert!(!w.social.has_session(&group));
        assert!(!w.is_group_session(&group));
        assert!(w.groups.restoring.is_empty());
        assert!(w.reopen_ims.is_empty());
        assert!(w.groups.take_commands().is_empty());
        assert!(w.on_session_im(&message));
        assert!(!w.social.has_session(&group));
    }

    #[test]
    fn open_ims_an_incoming_invitation_can_finish_restoration_without_a_duplicate_join() {
        let mut w = world();
        let group = Uuid::from_u128(42);
        w.on_group_event(NetEvent::Groups(vec![membership(group)]));
        w.reopen_ims = vec![Conversation::Group(group)];
        w.restore_open_ims(true);
        w.mutes.on_server(aurora_net::MuteListSource::UseCache, None);
        w.on_group_event(NetEvent::SessionInvite(aurora_net::SessionInvite {
            session: group,
            from: Uuid::from_u128(7),
            from_name: "Sender".into(),
            message: "Hello".into(),
            session_name: membership(group).name,
            offline: false,
            timestamp: 0,
        }));
        assert!(!w.groups.restoring.contains(&group));
        w.restore_open_ims(true);
        assert_eq!(w.social.ims.len(), 1);
        assert_eq!(w.social.ims[0].lines.len(), 1);
        assert!(w.groups.take_commands().iter().all(|cmd| !matches!(
            cmd,
            NetCommand::SendImDialog {
                dialog: dialog::SESSION_GROUP_START,
                ..
            }
        )));
    }

    #[test]
    fn open_ims_sending_from_a_preview_starts_the_group_first_and_respects_new_rlva_rules() {
        let group = Uuid::from_u128(42);
        let mut w = world();
        w.on_group_event(NetEvent::Groups(vec![membership(group)]));
        w.reopen_ims = vec![Conversation::Group(group)];
        w.restore_open_ims(true);
        w.send_session_im(group, "Hello");
        assert!(w.reopen_ims.is_empty());
        assert!(!w.groups.restoring.contains(&group));
        let out = w.groups.take_commands();
        assert!(matches!(
            out[..],
            [
                NetCommand::SendImDialog {
                    dialog: dialog::SESSION_GROUP_START,
                    ..
                },
                NetCommand::SendImDialog {
                    dialog: dialog::SESSION_SEND,
                    ..
                }
            ]
        ));

        let mut w = world();
        w.agent_id = Uuid::from_u128(99);
        w.on_group_event(NetEvent::Groups(vec![membership(group)]));
        w.reopen_ims = vec![Conversation::Group(group)];
        w.restore_open_ims(true);
        w.rlva.configure(true);
        w.apply(NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Controller".into(),
            source_id: Uuid::from_u128(88),
            owner_id: w.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: "@startim=n".into(),
        }));
        assert!(
            !w.rlva_can_start_im(group),
            "a displayed preview is not an authorized existing session"
        );
        w.send_session_im(group, "Blocked");
        assert!(w.groups.take_commands().is_empty());
        assert!(w.social.ims[0].lines.is_empty());
        w.restore_open_ims(true);
        assert!(!w.social.has_session(&group));
    }
}
