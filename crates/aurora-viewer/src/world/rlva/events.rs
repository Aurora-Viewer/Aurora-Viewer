//! Script permissions, teleport consent and restriction notifications.
//! See RlvUtil::filterScriptQuestions, RlvActions::canAcceptTpOffer and
//! RlvBehaviourNotifyHandler in the Legacy Viewer.
use super::{super::World, Rlva};
use uuid::Uuid;

/// rlvParseNotifyOption (rlvhandler.cpp): the channel, then an optional
/// filter; a third token makes the option invalid.
pub(super) fn notification_option(option: &str) -> Option<(i32, &str)> {
    let mut tokens = option.split(';');
    let channel = tokens.next()?.parse::<i32>().ok()?;
    let filter = tokens.next().unwrap_or_default();
    (tokens.next().is_none() && channel > 0 && channel != i32::MAX).then_some((channel, filter))
}

impl Rlva {
    /// An event (worn, attached, sat…): the whole text is tested against
    /// each subscription (RlvBehaviourNotifyHandler::sendNotification).
    pub fn notify(&mut self, text: &str) {
        self.notify_command(text, "");
    }

    /// The reference also notifies about rejected and duplicate commands.
    /// Only `subject`, the behaviour and its option, is tested against the
    /// subscription; `suffix` (`=n`, `=y`) follows it untested.
    pub(super) fn notify_command(&mut self, subject: &str, suffix: &str) {
        let tested = subject.to_ascii_lowercase();
        let channels: Vec<_> = self
            .rules_for("notify", Uuid::nil())
            .filter_map(|(_, option)| notification_option(option))
            .filter(|(_, filter)| tested.contains(&filter.to_ascii_lowercase()))
            .map(|(channel, _)| channel)
            .collect();
        for channel in channels {
            self.reply(channel, "notify", format!("/{subject}{suffix}"));
        }
    }

    pub fn auto_accept(&self, behaviour: &str, sender: Uuid) -> bool {
        self.has(behaviour) || (!sender.is_nil() && self.targets(behaviour, &sender.to_string()))
    }
}

impl World {
    /// After an OwnerSay stored rules: whether their issuers are known
    /// objects (`Rlva::resolve_sources`).
    pub(crate) fn rlva_resolve_sources(&mut self) {
        let objects = &self.objects;
        self.rlva.resolve_sources(|id| objects.index_of_uuid(&id).is_some());
    }

    /// RlvHandler::onDetach (rlvhandler.cpp), which LLVOAvatarSelf::
    /// detachObject (llvoavatarself.cpp) calls when one of our attachments is
    /// killed: the restrictions of every object rooted on it are cleared at
    /// once, and the listeners told. Run on a kill that is applied, before
    /// the objects leave the store: a kill ignored during a teleport or sent
    /// by a region we left (`World::kill_is_ignored`) keeps the attachment and
    /// its rules, and one dropped to the ground is not killed and keeps them
    /// too, as in the Legacy Viewer.
    pub(crate) fn rlva_attachment_killed(&mut self, idx: usize) {
        if !self.rlva.enabled() {
            return;
        }
        let Some(root) = self.objects.get(idx).filter(|o| !o.is_avatar()) else {
            return;
        };
        let ours = self
            .objects
            .parent_of(root)
            .and_then(|p| self.objects.get(p))
            .is_some_and(|p| p.is_avatar() && p.full_id == self.agent_id);
        if !ours {
            return;
        }
        let item = root.attachment_item_id().filter(|id| !id.is_nil()).unwrap_or(root.full_id);
        // The wear a forced batch asked for had arrived, since it leaves: it
        // is no longer on its way, and a new forced wear of the same object
        // is a new request (LLAttachmentsMgr::onAttachmentArrived forgets
        // the request at the arrival, llattachmentsmgr.cpp).
        self.rlva.requested_wears.remove(&item);
        let mut gone: std::collections::BTreeSet<Uuid> = self
            .objects
            .subtree(idx)
            .into_iter()
            .filter_map(|i| self.objects.get(i).map(|o| o.full_id))
            .collect();
        // A child prim can be killed before its root (the note in onDetach,
        // which matches on the remembered m_idRoot): the watchdog remembers
        // which attachment each issuer belonged to.
        gone.extend(
            self.rlva
                .watchdog
                .sources
                .iter()
                .filter(|(_, worn)| **worn == item)
                .map(|(source, _)| *source),
        );
        // (onDetach walks the objects that hold restrictions: a prim that
        // holds none keeps the commands it has queued)
        gone.retain(|source| self.rlva.rules.contains_key(source));
        for source in gone {
            self.rlva.release(source);
        }
    }

    pub fn rlva_can_accept_teleport(&self, sender: Uuid, request: bool) -> bool {
        self.rlva.allows(if request { "tprequest" } else { "tplure" }, &sender.to_string())
            && (request || !self.agent.is_sitting() || !self.rlva.has("unsit"))
    }

    /// RlvUtil::filterScriptQuestions (rlvcommon.cpp): the notice a question
    /// RLVa stripped of `requested` leaves for when it is answered yes
    /// (script_question_cb, llviewermessage.cpp); the teleport one replaces
    /// the attach one, as the payload keeps the last.
    pub(crate) fn rlva_permission_notice(&self, requested: i32, granted: i32, object_name: &str) -> Option<String> {
        const ATTACH: i32 = 1 << 5;
        const TELEPORT: i32 = 1 << 12;
        let stripped = requested & !granted;
        let object = object_name.to_owned();
        if stripped & TELEPORT != 0 {
            Some(aurora_i18n::tr!("rlva-blocked-permission-teleport", object = object))
        } else if stripped & ATTACH != 0 {
            Some(aurora_i18n::tr!("rlva-blocked-permission-attach", object = object))
        } else {
            None
        }
    }

    /// Keeps `notice` for the answer to the question of `task`'s `item`.
    pub(crate) fn rlva_hold_permission_notice(&mut self, task: Uuid, item: Uuid, notice: Option<String>) {
        let held = &mut self.rlva.permission_notices;
        held.retain(|(t, i, _)| (*t, *i) != (task, item));
        if let Some(text) = notice {
            if held.len() >= 16 {
                held.remove(0);
            }
            held.push((task, item, text));
        }
    }

    /// The answer to the question of `task`'s `item`: a yes shows what RLVa
    /// stripped from it (RlvUtil::notifyBlocked), any answer forgets it.
    pub(crate) fn rlva_answer_permission_notice(&mut self, task: Uuid, item: Uuid, granted: bool) {
        let held = &mut self.rlva.permission_notices;
        let Some(at) = held.iter().position(|(t, i, _)| (*t, *i) == (task, item)) else {
            return;
        };
        let (_, _, text) = held.remove(at);
        if granted {
            self.system_notice(text, super::super::notice_topics::Topic::Warning);
        }
    }

    /// Returns granted bits and whether the remaining question is eligible
    /// for automatic acceptance. Experiences and caution bits keep their UI.
    pub fn rlva_script_permissions(&self, task: Uuid, requested: i32) -> (i32, bool) {
        const TAKE_CONTROLS: i32 = 1 << 2;
        const ATTACH: i32 = 1 << 5;
        const TELEPORT: i32 = 1 << 12;
        let mut granted = requested;
        // RlvAttachmentLocks::canAttach asks whether *any* actual attachment
        // point is addable, including point-specific addattach/detach locks.
        if self.rlva.enabled()
            && (self.objects.index_of_uuid(&self.agent_id).is_none()
                || !self.avatar_lib.attach_points.keys().any(|point| self.rlva_point_addable(*point)))
        {
            granted &= !ATTACH;
        }
        if self.rlva.has("tploc") {
            granted &= !TELEPORT;
        }
        let mut unanswered = granted;
        if self.rlva.has("acceptpermission")
            && let Some(object) = self.objects.index_of_uuid(&task).and_then(|i| self.objects.get(i))
        {
            unanswered &= !TAKE_CONTROLS;
            let mut root = object;
            for _ in 0..64 {
                let Some(parent) = self.objects.parent_of(root).and_then(|index| self.objects.get(index)) else {
                    break;
                };
                if parent.is_avatar() {
                    break;
                }
                root = parent;
            }
            // permYouOwner uses the root's simulator ownership flag, not
            // inventory metadata; temporary/unresolved attachments stay worn.
            if root.update_flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0
                && root.attachment_point() == 0
                && object.attachment_point() == 0
            {
                unanswered &= !ATTACH;
            }
        }
        (
            granted,
            requested != 0 && unanswered == 0 && !super::super::experiences::caution(requested),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetEvent};

    fn command(world: &mut World, message: String) {
        world.apply(NetEvent::Chat(ChatMessage {
            source_id: Uuid::from_u128(123),
            from_name: "Controller".into(),
            owner_id: world.agent_id,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message,
        }));
    }

    #[test]
    fn rlva_permission_attach_requires_an_addable_point_and_auto_grants_only_an_owned_rezzed_object() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.rlva_configure(true);
        command(&mut world, "@acceptpermission=n".into());
        let task = Uuid::from_u128(789);
        let mut object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: task,
            owner_id: world.agent_id,
            point: 1,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: String::new(),
            desc: String::new(),
        });
        let handle = world.main_region.unwrap();
        world.objects.upsert(handle, object.clone());
        assert_eq!(
            world.rlva_script_permissions(task, 32),
            (32, false),
            "no inventory record does not make an attachment rezzed"
        );
        assert_eq!(world.rlva_script_permissions(task, 4), (4, true));
        object.parent_id = 0;
        object.state = 0;
        object.owner_id = Uuid::nil();
        world.objects.upsert(handle, object.clone());
        assert_eq!(
            world.rlva_script_permissions(task, 32),
            (32, true),
            "the simulator owner flag is sufficient"
        );
        object.update_flags &= !crate::ui::context::flags::OBJECT_YOU_OWNER;
        object.owner_id = world.agent_id;
        world.objects.upsert(handle, object);
        assert_eq!(world.rlva_script_permissions(task, 32), (32, false));
        let points: Vec<_> = world.avatar_lib.attach_points.keys().copied().collect();
        for point in points {
            let name = super::super::inventory::POINTS[point as usize];
            command(
                &mut world,
                format!("@{}:{name}=n", if point % 2 == 0 { "addattach" } else { "detach" }),
            );
        }
        assert_eq!(
            world.rlva_script_permissions(task, 32),
            (0, true),
            "point-specific locks can cover every point"
        );
        command(&mut world, "@addattach:skull=y".into());
        assert_eq!(
            world.rlva_script_permissions(task, 32),
            (32, false),
            "one free point permits asking to attach"
        );
        world.rlva_configure(false);
        assert_eq!(world.rlva_script_permissions(task, 32), (32, false));
    }
}
