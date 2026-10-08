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
                Data::Lure { from, .. } | Data::Inventory { from, .. } => *from != id,
                Data::Dialog { object, .. } | Data::TextBox { object, .. } => *object != id,
                Data::Permissions { task, .. } => *task != id,
                _ => true,
            });
        }
        if self.groups.report_blocks {
            self.system_message(format!("{name} a été ajouté(e) à la liste de blocage."));
        }
        Ok(())
    }

    /// Remove a block (all of it, or some properties with `flags`).
    pub fn unblock(&mut self, id: Uuid, name: &str, flags: u32) {
        let shown = self.mutes.get(&id).map(|m| m.name.clone()).unwrap_or_else(|| name.to_owned());
        if let Some(cmd) = self.mutes.remove(&id, name, flags) {
            self.groups.out.push(cmd);
            if self.groups.report_blocks && !self.mutes.is_muted(&id, name, 0) {
                self.system_message(format!("{shown} a été retiré(e) de la liste de blocage."));
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
        self.mutes.is_muted_id(object) || (!owner.is_nil() && self.mutes.sounds_muted(owner))
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
        self.groups.take_commands()
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
