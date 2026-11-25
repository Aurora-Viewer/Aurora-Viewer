//! What the block list does to incoming traffic, after Firestorm
//! (llviewermessage.cpp, llimprocessing.cpp, originally LGPL 2.1): chat and IMs of blocked
//! residents / objects are dropped, their offers ignored (inventory offers
//! declined), script dialogs, permission requests and URLs of blocked objects
//! or owners skipped; and the actions to block / unblock.

use super::World;
use super::mutes::{LoadState, Mute, MuteType, flag};
use super::notifications::{Data, im as d};
use aurora_net::{ChatMessage, ChatSourceType, InstantMessage, NetCommand};
use uuid::Uuid;

impl World {
    /// Per frame: ask for the block list and our groups once in world, and
    /// fall back on the cached list when the server stays silent.
    pub fn tick_social(&mut self) {
        if self.agent_id.is_nil() || !self.movement_complete {
            return;
        }
        if self.mutes.state == LoadState::Initial {
            self.mutes.reset(self.agent_id);
            let cmd = self.mutes.request(self.cache_dir.as_deref());
            self.groups.out.push(cmd);
            self.request_groups_once();
        }
        if self.mutes.check_timeout(self.cache_dir.as_deref()) {
            self.restore_deferred_group_chats();
        }
        self.request_offline_messages_once();
    }

    /// LLIMProcessing::requestOfflineMessages, polled every frame: once our
    /// avatar is there, the region's capabilities came and the block list
    /// is loaded (or its request timed out), since offers from blocked
    /// residents are among what comes back.
    fn request_offline_messages_once(&mut self) {
        if self.offline_im_requested || !self.mutes.is_loaded() {
            return;
        }
        let caps = self
            .main_region
            .and_then(|h| self.regions.get(&h))
            .is_some_and(|r| !r.caps.is_empty());
        if !caps || self.objects.index_of_uuid(&self.agent_id).is_none() {
            return;
        }
        self.offline_im_requested = true;
        self.groups.out.push(NetCommand::RequestOfflineMessages);
    }

    /// MuteListUpdate / UseCachedMuteList / "emptymutelist" answered.
    pub(super) fn on_mute_list(&mut self, src: aurora_net::MuteListSource) {
        self.mutes.on_server(src, self.cache_dir.as_deref());
        self.restore_deferred_group_chats();
    }

    /// Save the block list at logout (LLMuteList::cache).
    pub fn save_mute_cache(&self) {
        self.mutes.save(self.cache_dir.as_deref());
    }

    /// Owner of an object in the scene (nil when unknown).
    fn owner_of(&self, object: &Uuid) -> Uuid {
        self.objects
            .index_of_uuid(object)
            .and_then(|i| self.objects.get(i))
            .map(|o| o.owner_id)
            .unwrap_or(Uuid::nil())
    }

    /// The prim an object's block is kept on: the root of its linkset, or
    /// `id` itself when it is not in the scene. Muting is done by root
    /// objects (LLMute::LLMute, get_object_to_mute_from_id in
    /// indra/newview/llmutelist.cpp, originally LGPL 2.1): a script says
    /// things from the prim it sits in, often a child, chiefly in a worn
    /// object. Aurora difference: the Legacy Viewer goes up one level, so a
    /// block taken on a child prim lands on the root there too, but one
    /// taken on the root prim of someone's attachment lands on its wearer.
    /// Here it stays on the attachment, like on ours.
    pub fn object_block_root(&self, id: &Uuid) -> Uuid {
        self.objects
            .index_of_uuid(id)
            .map(|i| self.objects.linkset_root(i))
            .and_then(|i| self.objects.get(i))
            .map_or(*id, |o| o.full_id)
    }

    /// The linkset of `id` is blocked, on its root: what `is_muted` cannot
    /// see from the id of a child (LLMuteList::isMuted looks at the parent).
    fn root_muted(&self, id: &Uuid, flags: u32) -> bool {
        if self.mutes.len() == 0 {
            return false;
        }
        let root = self.object_block_root(id);
        root != *id && self.mutes.is_muted(&root, "", flags)
    }

    /// The block entry that covers an object: its own id (a block taken
    /// while it was out of the scene) or its root.
    pub fn object_block_entry(&self, id: &Uuid) -> Option<Uuid> {
        if self.mutes.len() == 0 {
            return None;
        }
        [*id, self.object_block_root(id)].into_iter().find(|i| self.mutes.is_muted_id(i))
    }

    /// Block or unblock an object from a menu: the whole of it, by its root,
    /// whichever prim was named.
    pub fn toggle_block_object(&mut self, id: Uuid, name: &str) -> Result<(), String> {
        let root = self.object_block_root(&id);
        let fallback;
        let name = if name.is_empty() {
            fallback = aurora_i18n::tr!("common-object-2", value0 = root.to_string()[..8].to_string());
            fallback.as_str()
        } else {
            name
        };
        match self.object_block_entry(&id) {
            Some(entry) => {
                self.unblock(entry, name, 0);
                Ok(())
            }
            None => self.block(root, name, MuteType::Object),
        }
    }

    /// Nearby chat to hide (process_chat_from_simulator: the source by id or
    /// name, its root, or its owner).
    pub(super) fn chat_blocked(&self, c: &ChatMessage) -> bool {
        if matches!(c.source_type, ChatSourceType::System) {
            return false;
        }
        self.mutes.text_muted(&c.source_id, &c.from_name)
            || self.root_muted(&c.source_id, flag::TEXT_CHAT)
            || (!c.owner_id.is_nil() && self.mutes.text_muted(&c.owner_id, ""))
    }

    /// A blocked source's nearby line (`chat_blocked`). An object's, or one
    /// of a blocked owner's objects, is dropped as process_chat_from_simulator
    /// drops it before anything. A resident's is marked mMuted there, and
    /// FSFloaterNearbyChat::addMessage keeps it for the muted history alone,
    /// shown with FSShowMutedChatHistory: no console line, no flash, no
    /// transcript, no sound, no bubble, no keyword alert, no unread count;
    /// Aurora keeps it the same way when Preferences › Chat asks for it,
    /// and translates nothing of it. Ours is never blocked.
    pub(super) fn blocked_chat_line(&mut self, c: ChatMessage) {
        if self.blocked_chat == crate::settings::BlockedChat::Hidden
            || c.source_type != ChatSourceType::Agent
            || c.source_id.is_nil()
            || c.source_id == self.agent_id
        {
            return;
        }
        let from = self.social.avatar_names.complete(&c.source_id).unwrap_or(c.from_name);
        self.nearby_push(super::ChatLine {
            time: std::time::SystemTime::now(),
            from,
            text: c.message,
            kind: super::ChatKind::Local(c.chat_type),
            source: c.source_id,
            origin: super::Origin::Live,
            blocked: true,
            // already greyed as blocked: not dimmed a second time
            beyond_range: false,
            speaker: Default::default(),
        });
    }

    /// A resident whose nearby chat is hidden (`chat_blocked`) is not named
    /// in the chat's "is typing" line either (#1395): their words never
    /// show, so the line would only point at them. The Legacy Viewer has no
    /// such line; the signs on the avatar (the "..." bubble, the name tag,
    /// the radar's badge) stay as there.
    pub fn nearby_typing_hidden(&self, avatar: &Uuid) -> bool {
        self.mutes.text_muted(avatar, &self.legacy_name(avatar).unwrap_or_default())
    }

    /// Script dialogs, permission requests and URLs from a blocked object
    /// or owner (LLScriptFloater, process_script_question, process_load_url).
    pub(super) fn object_blocked(&self, object: &Uuid, name: &str) -> bool {
        let owner = self.owner_of(object);
        self.mutes.is_muted(object, name, 0) || self.root_muted(object, 0) || (!owner.is_nil() && self.mutes.is_muted_id(&owner))
    }

    /// A script menu (process_script_dialog): also dropped when the owner
    /// the simulator names (OwnerData) is blocked, the object out of view
    /// or not cached yet. A nil owner (older simulators) leaves the scene's.
    pub(super) fn script_dialog_blocked(&self, object: &Uuid, name: &str, owner: &Uuid) -> bool {
        self.object_blocked(object, name) || (!owner.is_nil() && self.mutes.is_muted_id(owner))
    }

    /// An IM from someone (or an object) we blocked: dropped; inventory
    /// offers are declined like closing the offer (LLOfferInfo IOR_DECLINE).
    pub(super) fn im_blocked(&mut self, im: &InstantMessage) -> bool {
        if im.from_agent_id.is_nil() {
            return false;
        }
        // an IM from our own worn object carries our id: only the object's
        // block applies to it, as to its chat (LLIMProcessing::
        // processNewMessage has no exception for our own id)
        let own = im.from_agent_id == self.agent_id;
        // object IMs carry the object id in the session id (STORM-1209)
        let muted = (!own && self.mutes.text_muted(&im.from_agent_id, &im.from_name))
            || (im.dialog == 19 && (self.mutes.is_muted_id(&im.session_id) || self.root_muted(&im.session_id, 0)));
        // inventory_offer_handler: an offer is declined on any entry, one
        // that blocks only sounds, particles or voice too (isMuted with no
        // flags); chat and IMs keep the text chat flag
        let offer = im.dialog == d::INVENTORY_OFFERED || im.dialog == d::TASK_INVENTORY_OFFERED;
        let offer_muted = offer && !own && self.mutes.is_muted(&im.from_agent_id, &im.from_name, 0);
        if !muted && !offer_muted {
            return false;
        }
        // FSSendMutedAvatarResponse; friendship offers are declined
        // (forceResponse "OfferFriendship" 1)
        self.answer_blocked_im(im);
        if im.dialog == d::FRIENDSHIP_OFFERED {
            self.groups.out.push(NetCommand::DeclineFriendship {
                transaction: im.session_id,
            });
        }
        if im.dialog == d::INVENTORY_OFFERED || im.dialog == d::TASK_INVENTORY_OFFERED {
            let task = im.dialog == d::TASK_INVENTORY_OFFERED;
            self.groups.out.push(super::notifications::inventory_answer(
                im.from_agent_id,
                im.session_id,
                task,
                false,
                Uuid::nil(),
            ));
            // a blocked resident's offer is in our inventory all the same
            // (IOR_MUTE falls through to IOR_DECLINE: LLDiscardAgentOffer)
            if !task && let Some((asset_type, object)) = super::notifications::offered_object(&im.binary_bucket) {
                self.discard_offer(asset_type, object);
            }
        }
        log::debug!("IM (dialog {}) from blocked {} dropped", im.dialog, im.from_agent_id);
        true
    }

    /// Block a resident or an object entirely (LLMuteList::add with no flags).
    pub fn block(&mut self, id: Uuid, name: &str, kind: MuteType) -> Result<(), String> {
        self.block_flags(id, name, kind, 0)
    }

    /// Block some properties only (`flags`: flag::TEXT_CHAT, VOICE_CHAT...).
    pub fn block_flags(&mut self, id: Uuid, name: &str, kind: MuteType, flags: u32) -> Result<(), String> {
        let name = clean_name(name);
        let cmd = self
            .mutes
            .add(
                Mute {
                    id,
                    name: name.clone(),
                    kind,
                    flags: 0,
                },
                flags,
            )
            .map_err(|e| e.message())?;
        self.groups.out.push(cmd);
        if flags == 0 || flags & flag::TEXT_CHAT != 0 {
            // LLNotifications::cancelByOwner (agents), scripts of the object
            // and of the other prims of its linkset
            let mut list = std::mem::take(&mut self.notifications.list);
            list.retain(|n| match &n.data {
                Data::Lure { from, .. }
                | Data::TeleportRequest { from, .. }
                | Data::Inventory { from, .. }
                | Data::RlvaQuery { from, .. } => *from != id,
                Data::Dialog { object, .. } | Data::TextBox { object, .. } => *object != id && self.object_block_root(object) != id,
                Data::Permissions { task, .. } => *task != id && self.object_block_root(task) != id,
                _ => true,
            });
            self.notifications.list = list;
            self.close_orphan_inventory_offers();
        }
        if self.groups.report_blocks {
            let shown_name = if kind == MuteType::Agent && !self.rlva_can_name(id) {
                self.rlva_anonymous(id).display_name.clone()
            } else {
                name.clone()
            };
            self.system_notice(
                aurora_i18n::tr!("world-blocking-was-added-to-the-block-list", name = shown_name),
                crate::world::notice_topics::Topic::People,
            );
        }
        Ok(())
    }

    /// Canonical name and mute type for a remote object's owner. Like
    /// LLAvatarActions::toggleBlock / LLMuteList::add (llavataractions.cpp,
    /// llmutelist.cpp), resident blocks enforce self/Linden protections.
    /// Group owners follow inventory_offer_mute_callback (llviewermessage.cpp).
    /// Wait for a real name: a display name or RLVa alias cannot establish
    /// Linden status or be persisted as the owner's identity.
    pub fn object_owner_block(&self, owner: Uuid, group_owned: bool) -> Option<(String, MuteType)> {
        if owner.is_nil() || owner == self.agent_id || self.mutes.get(&owner).is_some_and(|entry| entry.flags == 0) {
            return None;
        }
        let (name, kind) = if group_owned {
            let name = self
                .groups
                .groups
                .iter()
                .find(|g| g.id == owner)
                .map(|g| g.name.clone())
                .or_else(|| self.land.group_names.get(&owner).cloned())?;
            (name, MuteType::Group)
        } else {
            let name = self
                .social
                .avatar_names
                .raw(&owner)
                .and_then(|n| {
                    if n.legacy_first.trim().is_empty() {
                        (!n.username.trim().is_empty()).then(|| n.username.clone())
                    } else {
                        Some(n.user_name())
                    }
                })
                .or_else(|| self.social.names.get(&owner).cloned())?;
            if super::mutes::is_linden(&name) {
                return None;
            }
            (name, MuteType::Agent)
        };
        (!name.trim().is_empty()).then_some((name, kind))
    }

    /// Recheck at dispatch, so stale inspector actions never unblock an owner.
    pub fn block_object_owner(&mut self, owner: Uuid, group_owned: bool) -> Result<(), String> {
        if let Some((name, kind)) = self.object_owner_block(owner, group_owned) {
            self.block(owner, &name, kind)?;
        }
        Ok(())
    }

    /// Remove a block (all of it, or some properties with `flags`).
    pub fn unblock(&mut self, id: Uuid, name: &str, flags: u32) {
        let shown = self.mutes.get(&id).map(|m| m.name.clone()).unwrap_or_else(|| name.to_owned());
        if let Some(cmd) = self.mutes.remove(&id, name, flags) {
            self.groups.out.push(cmd);
            if self.groups.report_blocks && !self.mutes.is_muted(&id, name, 0) {
                self.system_notice(
                    aurora_i18n::tr!("world-blocking-was-removed-from-the-block-list", shown = shown.to_string()),
                    crate::world::notice_topics::Topic::People,
                );
            }
        }
    }

    /// Block or unblock an avatar from a menu.
    pub fn toggle_block_avatar(&mut self, id: Uuid, name: &str) -> Result<(), String> {
        if self.mutes.is_muted_id(&id) {
            self.unblock(id, name, 0);
            Ok(())
        } else {
            self.block(id, name, MuteType::Agent)
        }
    }

    /// Blocked residents (never rendered in full: grey silhouettes,
    /// FIRE-11783 / LLVOAvatar::isInMuteList).
    pub fn is_avatar_blocked(&self, id: &Uuid) -> bool {
        *id != self.agent_id && self.mutes.is_muted_id(id)
    }

    /// Object sounds of a blocked owner / object (process_sound_trigger,
    /// process_attached_sound).
    pub fn sound_blocked(&self, object: &Uuid, owner: &Uuid) -> bool {
        self.mutes.is_muted_id(object)
            || self.root_muted(object, 0)
            || (!owner.is_nil() && self.mutes.sounds_muted(owner))
            || self.anti_spam.is_blocked(super::anti_spam::Queue::Sound, *object)
            || self.anti_spam.is_blocked(super::anti_spam::Queue::Sound, *owner)
    }

    /// is_sound_blacklisted (indra/newview/llviewermessage.cpp): flags distinguish
    /// gestures, attachments (including linked children), and rezzed objects.
    pub fn sound_blacklisted(&self, asset: Uuid, object: Uuid, owner: Uuid) -> bool {
        if self.derender.is_empty() {
            return false;
        }
        let flag = if object == owner && !owner.is_nil() {
            4
        } else {
            let mut idx = self.objects.index_of_uuid(&object);
            if idx.is_none() {
                return self.derender.sound_blocked(asset, owner, 0);
            }
            let mut worn = false;
            for _ in 0..64 {
                let Some(o) = idx.and_then(|i| self.objects.get(i)) else {
                    break;
                };
                if o.is_avatar() {
                    worn = true;
                    break;
                }
                idx = self.objects.parent_of(o);
            }
            if worn { 1 } else { 2 }
        };
        self.derender.sound_blocked(asset, owner, flag)
    }

    /// Particles of a blocked owner (LLViewerPartSourceScript).
    pub fn particles_blocked(&self, owner: &Uuid) -> bool {
        !owner.is_nil() && self.mutes.is_muted(owner, "", flag::PARTICLES)
    }

    /// Voice of a blocked resident.
    pub fn voice_blocked(&self, id: &Uuid) -> bool {
        self.mutes.is_muted(id, "", flag::VOICE_CHAT)
    }

    /// Commands queued by groups, sessions and the block list.
    pub fn take_social_commands(&mut self) -> Vec<NetCommand> {
        self.expire_rights_grants(std::time::Instant::now());
        let mut cmds = self.groups.take_commands();
        cmds.extend(self.group_data.take_commands());
        cmds.extend(self.group_money.take_commands());
        cmds
    }
}

impl World {
    /// Grants of rights whose answer never came (friend_rights): the
    /// friend's newest choice leaves, or what the server said is shown.
    pub(super) fn expire_rights_grants(&mut self, now: std::time::Instant) {
        use super::friend_rights::Answer;
        for (friend, answer) in self.social.rights_sync.expired(now) {
            match answer {
                Answer::Send(rights) => self.groups.out.push(NetCommand::GrantUserRights { friend, rights }),
                Answer::Show(rights) => {
                    if let Some(f) = self.social.friends.iter_mut().find(|f| f.id == friend) {
                        f.rights_given = rights;
                    }
                }
                Answer::Keep => {}
            }
        }
    }
}

/// Mute names leave out the "Resident" last name (LLMute::mName).
fn clean_name(name: &str) -> String {
    let n = name.trim();
    n.strip_suffix(" Resident").unwrap_or(n).to_owned()
}

impl World {
    /// Legacy "First Last" name of an avatar (what the block list stores).
    pub fn legacy_name(&self, id: &Uuid) -> Option<String> {
        self.social.names.get(id).cloned().or_else(|| self.avatar_name(id))
    }

    /// `legacy_name` of a name shown in `scope`: a place that names its
    /// resident (a profile opened from a conversation, `NameScope::Roster`)
    /// gets the real name while `@shownames` hides it in the world, never
    /// the stand-in.
    pub fn legacy_name_in(&self, scope: super::names::NameScope, id: &Uuid) -> Option<String> {
        if !id.is_nil() && !self.rlva_shows_name(scope, *id) {
            return Some(self.rlva_anonymous(*id).display_name.clone());
        }
        self.social.names.get(id).cloned().or_else(|| self.scene_name(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::sync::Arc;

    #[test]
    fn remote_object_owner_blocks_use_canonical_names_and_never_toggle_an_existing_block() {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        w.agent_id = Uuid::from_u128(99);
        w.mutes.reset(w.agent_id);
        let owner = Uuid::from_u128(1);
        // No guessed identity while the name is still loading.
        w.block_object_owner(owner, false).unwrap();
        assert!(w.take_social_commands().is_empty());
        w.social.names.insert(owner, "Demo Owner".into());
        w.block_flags(owner, "Demo Owner", MuteType::Agent, flag::VOICE_CHAT).unwrap();
        w.take_social_commands();
        // A voice-only block can be promoted to a complete owner block.
        w.block_object_owner(owner, false).unwrap();
        assert!(
            matches!(&w.take_social_commands()[..], [NetCommand::UpdateMute { id, name, kind: 1, flags: 0 }]
            if *id == owner && name == "Demo Owner")
        );
        assert_eq!(w.mutes.get(&owner).unwrap().flags, 0);
        w.block_object_owner(owner, false).unwrap();
        assert!(w.take_social_commands().is_empty());
        assert!(w.mutes.is_muted_id(&owner));
        let group = Uuid::from_u128(2);
        w.land.group_names.insert(group, "Demo Group".into());
        w.block_object_owner(group, true).unwrap();
        assert!(matches!(&w.take_social_commands()[..], [NetCommand::UpdateMute { id, kind: 3, flags: 0, .. }] if *id == group));
    }

    #[test]
    fn remote_object_owner_blocks_preserve_self_and_linden_protections_under_rlva() {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        w.agent_id = Uuid::from_u128(99);
        w.mutes.reset(w.agent_id);
        let staff = Uuid::from_u128(1);
        w.apply(aurora_net::NetEvent::DisplayNames {
            names: vec![aurora_net::AvatarNameData {
                id: staff,
                username: "test.linden".into(),
                display_name: "An unrelated display name".into(),
                ..Default::default()
            }],
            bad_ids: Vec::new(),
            max_age: None,
        });
        w.rlva_configure(true);
        assert!(w.rlva.receive(&ChatMessage {
            from_name: "RLVa test controller".into(),
            source_id: Uuid::from_u128(4),
            owner_id: w.agent_id,
            source_type: ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: "@shownames=n".into(),
        }));
        for (owner, group) in [
            (Uuid::nil(), false),
            (w.agent_id, false),
            (w.agent_id, true),
            (staff, false),
            (staff, true),
        ] {
            w.block_object_owner(owner, group).unwrap();
            assert!(w.mutes.get(&owner).is_none());
        }
        assert!(w.take_social_commands().is_empty());
    }

    #[test]
    fn remote_object_owner_block_keeps_canonical_identity_out_of_rlva_notices() {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        w.agent_id = Uuid::from_u128(99);
        w.mutes.reset(w.agent_id);
        w.groups.report_blocks = true;
        let owner = Uuid::from_u128(1);
        w.apply(aurora_net::NetEvent::DisplayNames {
            names: vec![aurora_net::AvatarNameData {
                id: owner,
                username: "demo.owner".into(),
                display_name: "A custom display name".into(),
                legacy_first: "Demo".into(),
                legacy_last: "Owner".into(),
                ..Default::default()
            }],
            bad_ids: Vec::new(),
            max_age: None,
        });
        w.rlva_configure(true);
        assert!(w.rlva.receive(&ChatMessage {
            from_name: "RLVa test controller".into(),
            source_id: Uuid::from_u128(4),
            owner_id: w.agent_id,
            source_type: ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: "@shownames=n".into(),
        }));
        w.refresh_rlva_privacy();
        w.block_object_owner(owner, false).unwrap();
        assert_eq!(w.mutes.get(&owner).unwrap().name, "Demo Owner");
        let notice = &w.chat.back().expect("block notice").text;
        assert!(!notice.contains("Demo Owner"));
        assert!(!notice.contains("A custom display name"));
        assert!(notice.contains(&w.rlva_anonymous(owner).display_name));
    }

    #[test]
    fn mute_particle_flag_remains_independent_of_other_mute_flags() {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        w.agent_id = Uuid::from_u128(99);
        w.mutes.reset(w.agent_id);
        let jane = Uuid::from_u128(1);
        // a new entry mutes only the particles
        w.block_flags(jane, "Jane Doe", MuteType::Agent, flag::PARTICLES).expect("blocked");
        assert!(w.particles_blocked(&jane));
        assert!(!w.mutes.text_muted(&jane, ""));
        assert!(!w.voice_blocked(&jane));
        assert!(matches!(
            w.take_social_commands().last(),
            Some(NetCommand::UpdateMute { flags, .. }) if *flags == flag::ALL & !flag::PARTICLES
        ));
        // again: the entry goes (nothing else was muted)
        w.unblock(jane, "Jane Doe", flag::PARTICLES);
        assert!(!w.particles_blocked(&jane));
        assert!(w.mutes.get(&jane).is_none());
        // a resident blocked in full keeps the rest when their particles
        // are let through
        w.block(jane, "Jane Doe", MuteType::Agent).expect("blocked");
        assert!(w.particles_blocked(&jane));
        w.unblock(jane, "Jane Doe", flag::PARTICLES);
        assert!(!w.particles_blocked(&jane));
        assert!(w.mutes.text_muted(&jane, "") && w.voice_blocked(&jane));
    }

    /// The offline HUD: a linkset worn by our avatar, and a stranger's world.
    fn worn_world() -> (World, Uuid, Uuid) {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            w.apply(event);
        }
        let agent = w.agent_id;
        w.mutes.reset(agent);
        let id = |w: &World, local| w.objects.iter().find(|(_, o)| o.key.local_id == local).expect("object").1.full_id;
        let child = id(&w, crate::demo::hud::CHILD);
        let root = w.object_block_root(&child);
        (w, child, root)
    }

    fn said_by(source: Uuid, owner: Uuid) -> ChatMessage {
        ChatMessage {
            source_id: source,
            owner_id: owner,
            from_name: "Worn Thing".into(),
            source_type: ChatSourceType::Object,
            chat_type: aurora_net::ChatType::Normal,
            region: None,
            position: glam::Vec3::ZERO,
            message: String::new(),
        }
    }

    #[test]
    fn a_block_taken_from_a_child_prim_holds_its_whole_linkset() {
        let (mut w, child, root) = worn_world();
        assert_ne!(child, root, "the worn linkset has a child");
        assert!(w.objects.index_of_uuid(&root).is_some());
        assert!(!w.chat_blocked(&said_by(child, w.agent_id)));
        w.toggle_block_object(child, "Worn Thing").expect("blocked");
        // kept on the root, as an object: not on the wearer, us
        let entry = w.mutes.get(&root).expect("the root's entry");
        assert_eq!(entry.kind, MuteType::Object);
        assert!(w.mutes.get(&child).is_none());
        assert!(w.mutes.get(&w.agent_id).is_none());
        // whichever prim speaks or plays
        for prim in [child, root] {
            assert!(w.chat_blocked(&said_by(prim, w.agent_id)), "chat");
            assert!(w.object_blocked(&prim, ""), "dialogs");
            assert!(w.sound_blocked(&prim, &Uuid::nil()), "sounds");
            assert_eq!(w.object_block_entry(&prim), Some(root), "the menu reads it as blocked");
        }
    }

    fn im_from(prim: Uuid, from: Uuid, dialog: u8) -> InstantMessage {
        InstantMessage {
            from_agent_id: from,
            from_name: "Worn Thing".into(),
            to_agent_id: from,
            dialog,
            session_id: prim,
            message: "hello".into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        }
    }

    #[test]
    fn a_blocked_linkset_of_ours_sends_no_im_from_a_child_prim() {
        let (mut w, child, root) = worn_world();
        let agent = w.agent_id;
        assert!(!w.im_blocked(&im_from(child, agent, 19)));
        w.toggle_block_object(child, "Worn Thing").expect("blocked");
        for prim in [child, root] {
            assert!(w.im_blocked(&im_from(prim, agent, 19)), "object IM");
        }
        // our own IMs and offers are never blocked
        assert!(!w.im_blocked(&im_from(Uuid::from_u128(0x5e55), agent, 0)));
    }

    #[test]
    fn the_second_click_on_a_child_prim_unblocks_the_linkset() {
        let (mut w, child, root) = worn_world();
        w.toggle_block_object(child, "Worn Thing").expect("blocked");
        w.toggle_block_object(child, "Worn Thing").expect("unblocked");
        assert!(w.mutes.get(&root).is_none());
        assert!(!w.chat_blocked(&said_by(child, w.agent_id)));
    }

    fn question(task: Uuid) -> aurora_net::NetEvent {
        aurora_net::NetEvent::ScriptQuestion {
            task_id: task,
            item_id: Uuid::from_u128(0x17e),
            object_name: "Worn Thing".into(),
            owner_name: "Someone".into(),
            questions: 4,
            experience: Uuid::nil(),
            region: 0,
        }
    }

    fn dialog(object: Uuid) -> aurora_net::NetEvent {
        aurora_net::NetEvent::ScriptDialog {
            sender: ([127, 0, 0, 1], 13000).into(),
            object_id: object,
            object_name: "Worn Thing".into(),
            owner_name: "Someone".into(),
            owner_id: Uuid::nil(),
            message: "Choose".into(),
            channel: -42,
            buttons: vec!["Yes".into()],
        }
    }

    fn asks(w: &World, prim: Uuid) -> bool {
        w.notifications.list.iter().any(|n| match &n.data {
            Data::Permissions { task, .. } => *task == prim,
            Data::Dialog { object, .. } => *object == prim,
            _ => false,
        })
    }

    #[test]
    fn a_blocked_linkset_asks_nothing_from_a_child_prim() {
        let (mut w, child, root) = worn_world();
        w.block(root, "Worn Thing", MuteType::Object).expect("blocked");
        w.apply(question(child));
        assert!(!asks(&w, child), "process_script_question: isMuted looks at the parent");
    }

    #[test]
    fn blocking_a_linkset_closes_what_its_child_prims_asked() {
        for event in [question as fn(Uuid) -> aurora_net::NetEvent, dialog] {
            let (mut w, child, _) = worn_world();
            w.apply(event(child));
            assert!(asks(&w, child), "the question is open");
            w.toggle_block_object(child, "Worn Thing").expect("blocked");
            assert!(!asks(&w, child), "and goes with the block");
        }
    }

    #[test]
    fn a_blocked_linkset_preloads_nothing_from_a_child_prim() {
        let (mut w, child, root) = worn_world();
        let preload = || {
            aurora_net::NetEvent::PreloadSounds(vec![aurora_net::SoundPreload {
                sound: Uuid::from_u128(0x50),
                object: child,
                owner: Uuid::from_u128(0x0e),
            }])
        };
        assert!(w.apply(preload()).is_some());
        w.block(root, "Worn Thing", MuteType::Object).expect("blocked");
        assert!(w.apply(preload()).is_none());
    }

    #[test]
    fn a_block_taken_out_of_the_scene_still_ends_from_the_menu() {
        let (mut w, child, root) = worn_world();
        // the object was not loaded then: the entry is on the id chat gave
        w.block(child, "Worn Thing", MuteType::Object).expect("blocked");
        assert_eq!(w.object_block_entry(&child), Some(child));
        assert!(w.chat_blocked(&said_by(child, w.agent_id)));
        assert!(!w.chat_blocked(&said_by(root, w.agent_id)), "the root was never named");
        w.toggle_block_object(child, "Worn Thing").expect("unblocked");
        assert!(w.mutes.get(&child).is_none());
    }

    #[test]
    fn an_object_out_of_the_scene_is_its_own_root() {
        let (mut w, _, _) = worn_world();
        let far = Uuid::from_u128(0xfa2);
        assert_eq!(w.object_block_root(&far), far);
        w.toggle_block_object(far, "").expect("blocked");
        let entry = w.mutes.get(&far).expect("entry");
        assert!(!entry.name.is_empty(), "a name for the list");
        assert!(w.chat_blocked(&said_by(far, Uuid::nil())));
    }

    #[test]
    fn a_text_only_exception_on_the_root_lets_its_child_prims_speak() {
        let (mut w, child, root) = worn_world();
        w.block(root, "Worn Thing", MuteType::Object).expect("blocked");
        w.unblock(root, "Worn Thing", flag::TEXT_CHAT);
        assert!(!w.chat_blocked(&said_by(child, w.agent_id)));
        assert!(w.sound_blocked(&child, &Uuid::nil()), "the rest stays blocked");
    }

    #[test]
    fn blocked_chat_leaves_the_nearby_typing_line() {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        w.agent_id = Uuid::from_u128(99);
        w.mutes.reset(w.agent_id);
        let jane = Uuid::from_u128(1);
        assert!(!w.nearby_typing_hidden(&jane));
        // her voice alone blocked: her chat shows, and so does her typing
        w.block_flags(jane, "Jane Doe", MuteType::Agent, flag::VOICE_CHAT).expect("blocked");
        assert!(!w.nearby_typing_hidden(&jane));
        // her text chat blocked too: no "Jane is typing"
        w.block_flags(jane, "Jane Doe", MuteType::Agent, flag::TEXT_CHAT).expect("blocked");
        assert!(w.nearby_typing_hidden(&jane));
        w.unblock(jane, "Jane Doe", flag::ALL);
        assert!(!w.nearby_typing_hidden(&jane));
        // blocked in full
        w.block(jane, "Jane Doe", MuteType::Agent).expect("blocked");
        assert!(w.nearby_typing_hidden(&jane));
        // an old block by name only, matched through her known name
        let bob = Uuid::from_u128(2);
        w.block(Uuid::nil(), "Bob Smith", MuteType::ByName).expect("blocked");
        assert!(!w.nearby_typing_hidden(&bob));
        w.social.names.insert(bob, "Bob Smith".into());
        assert!(w.nearby_typing_hidden(&bob));
    }

    /// A world with Jane blocked, her spam box blocked, and Bob free.
    fn blocked_world(mode: crate::settings::BlockedChat) -> World {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        w.agent_id = Uuid::from_u128(99);
        w.mutes.reset(w.agent_id);
        w.block(JANE, "Jane Doe", MuteType::Agent).expect("blocked");
        w.block(SPAM_BOX, "Spam box", MuteType::Object).expect("blocked");
        w.take_social_commands();
        w.blocked_chat = mode;
        w
    }

    const JANE: Uuid = Uuid::from_u128(1);
    const BOB: Uuid = Uuid::from_u128(2);
    const SPAM_BOX: Uuid = Uuid::from_u128(3);
    const JANES_BOX: Uuid = Uuid::from_u128(4);

    fn said(w: &mut World, from: Uuid, owner: Uuid, source_type: ChatSourceType, text: &str) {
        w.apply(aurora_net::NetEvent::Chat(ChatMessage {
            from_name: "Speaker".into(),
            source_id: from,
            owner_id: owner,
            source_type,
            chat_type: aurora_net::ChatType::Normal,
            region: None,
            position: glam::Vec3::ZERO,
            message: text.into(),
        }));
    }

    #[test]
    fn a_blocked_residents_line_is_dropped_while_hidden_as_by_default() {
        assert_eq!(crate::settings::BlockedChat::default(), crate::settings::BlockedChat::Hidden);
        let mut w = blocked_world(crate::settings::BlockedChat::Hidden);
        said(&mut w, JANE, Uuid::nil(), ChatSourceType::Agent, "hello");
        assert!(w.chat.is_empty());
        assert_eq!((w.chat_unread, w.chat_badge), (0, 0));
        assert!(w.ui_sounds.is_empty() && w.attention.is_empty());
    }

    #[test]
    fn a_blocked_residents_line_goes_to_the_history_alone_when_collapsed_or_greyed() {
        for mode in [crate::settings::BlockedChat::Collapsed, crate::settings::BlockedChat::Greyed] {
            let mut w = blocked_world(mode);
            // even a line that names us or holds a keyword alerts nobody
            let mention = format!("hey secondlife:///app/agent/{}/mention", w.agent_id);
            said(&mut w, JANE, Uuid::nil(), ChatSourceType::Agent, &mention);
            let line = w.chat.back().expect("kept for the history");
            assert!(line.blocked, "{mode:?}");
            assert_eq!(line.source, JANE);
            assert_eq!(line.kind, crate::world::ChatKind::Local(aurora_net::ChatType::Normal));
            // no unread count, sound, flash, notification, bubble, last
            // speaker
            assert_eq!((w.chat_unread, w.chat_badge), (0, 0), "{mode:?}");
            assert!(w.ui_sounds.is_empty() && w.attention.is_empty(), "{mode:?}");
            assert!(!w.nearby_news && w.last_speaker.is_none(), "{mode:?}");
            assert!(!w.bubble_chat.has_lines(&JANE, std::time::Instant::now()), "{mode:?}");
            // Bob's line is his own as ever
            said(&mut w, BOB, Uuid::nil(), ChatSourceType::Agent, "hi");
            assert!(!w.chat.back().expect("Bob's line").blocked);
            assert_eq!(w.chat_unread, 1);
        }
    }

    #[test]
    fn a_blocked_objects_line_is_dropped_in_every_mode() {
        use crate::settings::BlockedChat;
        for mode in [BlockedChat::Hidden, BlockedChat::Collapsed, BlockedChat::Greyed] {
            let mut w = blocked_world(mode);
            // blocked itself, or owned by a blocked resident
            said(&mut w, SPAM_BOX, BOB, ChatSourceType::Object, "free money");
            said(&mut w, JANES_BOX, JANE, ChatSourceType::Object, "buy now");
            assert!(w.chat.is_empty(), "{mode:?}");
        }
    }
}
