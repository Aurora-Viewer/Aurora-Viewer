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

    /// MuteListUpdate / UseCachedMuteList answered.
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

    /// Nearby chat to hide (process_chat_from_simulator: the source by id or
    /// name, or its owner).
    pub(super) fn chat_blocked(&self, c: &ChatMessage) -> bool {
        if matches!(c.source_type, ChatSourceType::System) {
            return false;
        }
        self.mutes.text_muted(&c.source_id, &c.from_name) || (!c.owner_id.is_nil() && self.mutes.text_muted(&c.owner_id, ""))
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
        self.mutes.is_muted(object, name, 0) || (!owner.is_nil() && self.mutes.is_muted_id(&owner))
    }

    /// An IM from someone (or an object) we blocked: dropped; inventory
    /// offers are declined like closing the offer (LLOfferInfo IOR_DECLINE).
    pub(super) fn im_blocked(&mut self, im: &InstantMessage) -> bool {
        if im.from_agent_id.is_nil() || im.from_agent_id == self.agent_id {
            return false;
        }
        // object IMs carry the object id in the session id (STORM-1209)
        let muted = self.mutes.text_muted(&im.from_agent_id, &im.from_name) || (im.dialog == 19 && self.mutes.is_muted_id(&im.session_id));
        if !muted {
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
            self.notifications.list.retain(|n| match &n.data {
                Data::Lure { from, .. }
                | Data::TeleportRequest { from, .. }
                | Data::Inventory { from, .. }
                | Data::RlvaQuery { from, .. } => *from != id,
                Data::Dialog { object, .. } | Data::TextBox { object, .. } => *object != id,
                Data::Permissions { task, .. } => *task != id,
                _ => true,
            });
            self.close_orphan_inventory_offers();
        }
        if self.groups.report_blocks {
            self.system_notice(
                aurora_i18n::tr!("world-blocking-was-added-to-the-block-list", name = name.to_string()),
                crate::world::notice_topics::Topic::People,
            );
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
        let mut cmds = self.groups.take_commands();
        cmds.extend(self.group_data.take_commands());
        cmds.extend(self.group_money.take_commands());
        cmds
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::sync::Arc;

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
}
