//! RlvHandler::onExperienceAttach/getAttachmentResourcesCoro (rlvhandler.cpp,
//! LGPL 2.1). Experience acceptance is independent from controller permission.
use super::{Param, preferences::Preferences};
use crate::world::World;
use aurora_net::{
    NetCommand,
    experience::ExperienceDetails,
    land::{LandCommand, ScriptAttachment},
};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A memory bound the Legacy Viewer does not have (RlvHandler::
/// m_BlockedObjects), far above the experiences an avatar attaches.
const LIMIT: usize = 4096;
struct Waiting {
    experience: Uuid,
    name: String,
    since: Instant,
}
struct Blocked {
    root: Option<Uuid>,
    name: String,
    since: Instant,
    seen: bool,
}

#[derive(Default)]
pub struct Controllers {
    waiting: Vec<Waiting>,
    blocked: Vec<Blocked>,
}

fn allowed(preferences: &Preferences, details: &ExperienceDetails) -> bool {
    let threshold = match preferences.experience_maturity {
        0 => 0,
        1 => 13,
        2 => 21,
        _ => 42,
    };
    let id = details.id.to_string();
    preferences.temporary_controllers
        && threshold != 0
        && details.maturity >= threshold
        && !preferences.blocked_experiences.split(';').any(|blocked| blocked == id)
}

impl Controllers {
    pub fn attachment(&mut self, experience: Uuid, name: String, now: Instant) {
        if !experience.is_nil() && !name.is_empty() && name.len() <= 1024 && self.waiting.len() < LIMIT {
            self.waiting.push(Waiting {
                experience,
                name,
                since: now,
            });
        }
    }
    /// RlvHandler::isBlockedObject (rlvhandler.cpp): the very object the
    /// region reported, not the other prims of its linkset.
    pub fn blocked(&self, source: Uuid) -> bool {
        self.blocked.iter().any(|entry| entry.root == Some(source))
    }
    pub fn accepts(&self, source: Uuid, param: Param) -> bool {
        matches!(param, Param::Remove | Param::Clear) || !self.blocked(source)
    }
}

impl World {
    /// process_chat_from_simulator + LLViewerObject::isTempAttachment:
    /// wire attachment state and AttachItemID identify temporary prims,
    /// independently of experience origin or a loaded inventory/avatar tree.
    pub(crate) fn rlva_controller_chat_allowed(&self, source: Uuid) -> bool {
        self.rlva.preferences.temporary_controllers
            || self
                .objects
                .index_of_uuid(&source)
                .and_then(|index| self.objects.get(index))
                .is_none_or(|object| {
                    object.attachment_point() == 0 || object.full_id.is_nil() || object.attachment_item_id() != Some(object.full_id)
                })
    }

    pub(crate) fn rlva_demo_experience(&mut self, maturity: u8, classify: bool) {
        let root = Uuid::from_u128(0x726c766165787072);
        if classify {
            self.rlva_controller_resources(&[ScriptAttachment {
                location: "chest".into(),
                objects: vec![aurora_net::land::ScriptObject {
                    id: root,
                    name: "Experience controller".into(),
                    owner: self.agent_id,
                    group_owned: false,
                    memory: 0,
                    urls: 0,
                    location: None,
                }],
            }]);
        } else if let Some(region) = self.main_region {
            let object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
                item_id: root,
                owner_id: self.agent_id,
                point: 1,
                add: true,
                flags: 0,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
                name: "Experience controller".into(),
                desc: String::new(),
            });
            self.apply(aurora_net::NetEvent::ObjectUpdates {
                handle: region,
                objects: vec![object],
            });
            self.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
                source_id: root,
                from_name: "Experience controller".into(),
                owner_id: self.agent_id,
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                region: None,
                position: glam::Vec3::ZERO,
                message: "@fly=n,jump=n".into(),
            }));
            let experience = Uuid::from_u128(0x726c766165787069);
            self.apply(aurora_net::NetEvent::ExperienceInfo(vec![ExperienceDetails {
                id: experience,
                maturity,
                ..Default::default()
            }]));
            self.apply(aurora_net::NetEvent::ExperienceAttachment {
                experience,
                object_name: "Experience controller".into(),
            });
        }
    }

    pub(crate) fn rlva_demo_experience_report(&self) {
        log::info!(
            "RLVa experience demo: blocked={}, fly={}, jump={}",
            self.rlva.controllers.blocked(Uuid::from_u128(0x726c766165787072)),
            self.rlva.has("fly"),
            self.rlva.has("jump")
        );
    }

    fn rlva_controller_root(&self, source: Uuid) -> Option<(Uuid, bool)> {
        let root = self.rlva_own_attachment_root(source)?;
        let object = self.objects.index_of_uuid(&root).and_then(|index| self.objects.get(index))?;
        Some((root, object.attachment_item_id() == Some(root)))
    }

    pub(crate) fn rlva_controllers_tick(&mut self, now: Instant) {
        if !self.rlva.enabled() || (self.rlva.controllers.waiting.is_empty() && self.rlva.controllers.blocked.is_empty()) {
            return;
        }
        let mut controllers = std::mem::take(&mut self.rlva.controllers);
        let waiting = std::mem::take(&mut controllers.waiting);
        let mut request = 0usize;
        for entry in waiting {
            if now.saturating_duration_since(entry.since) >= Duration::from_secs(300) {
                continue;
            }
            if let Some(details) = self.experiences.want(entry.experience) {
                if !allowed(&self.rlva.preferences, details) && controllers.blocked.len() < LIMIT {
                    controllers.blocked.push(Blocked {
                        root: None,
                        name: entry.name,
                        since: now,
                        seen: false,
                    });
                    request += 1;
                }
            } else {
                controllers.waiting.push(entry);
            }
        }
        controllers
            .blocked
            .retain(|entry| entry.root.is_some() || now.saturating_duration_since(entry.since) < Duration::from_secs(300));
        // Detach cleanup waits for an authoritative own-avatar tree. Entries
        // identified before their object arrives remain blocked in the meantime.
        if self.movement_complete && self.objects.index_of_uuid(&self.agent_id).is_some() {
            controllers.blocked.retain_mut(|entry| {
                let Some(root) = entry.root else {
                    return true;
                };
                if self.rlva_controller_root(root).is_some() {
                    entry.seen = true;
                    true
                } else {
                    !entry.seen
                }
            });
        }
        self.rlva.controllers = controllers;
        // RlvHandler::onExperienceAttach asks for the AttachmentResources
        // of every experience it blocks, one request each.
        for _ in 0..request {
            self.rlva.out.push(NetCommand::Land(LandCommand::AttachmentScripts));
        }
    }

    pub(crate) fn rlva_controller_resources(&mut self, attachments: &[ScriptAttachment]) {
        if !self.rlva.enabled() {
            return;
        }
        // RlvHandler::getAttachmentResourcesCoro (rlvhandler.cpp): the first
        // object, in the order of the reply, that is not known as anything
        // but a temporary attachment and not blocked yet, whose name an
        // unresolved entry waits for, is that entry's object; its rules are
        // cleared (with a "/clear" notice) and the reply's other objects are
        // left to the next request.
        for object in attachments.iter().flat_map(|attachment| &attachment.objects) {
            if object.id.is_nil() || self.rlva.controllers.blocked(object.id) {
                continue;
            }
            if self.objects.index_of_uuid(&object.id).is_some()
                && !self
                    .rlva_controller_root(object.id)
                    .is_some_and(|(root, temporary)| root == object.id && temporary)
            {
                continue;
            }
            let seen = self.rlva_controller_root(object.id).is_some();
            if let Some(entry) = self
                .rlva
                .controllers
                .blocked
                .iter_mut()
                .find(|entry| entry.root.is_none() && entry.name == object.name)
            {
                entry.root = Some(object.id);
                entry.seen = seen;
                self.rlva.clear_issuer(object.id);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rlva_experience_controller_policy_uses_reference_defaults_threshold_and_block_list() {
        let mut prefs = Preferences::default();
        let id = Uuid::from_u128(10);
        let mut details = ExperienceDetails {
            id,
            maturity: 13,
            ..Default::default()
        };
        assert!(!allowed(&prefs, &details));
        details.maturity = 21;
        assert!(allowed(&prefs, &details));
        details.maturity = 42;
        assert!(allowed(&prefs, &details));
        let blocked_default = ExperienceDetails {
            id: Uuid::parse_str("bfe25fb4-222c-11e5-85a2-fa4c4ccaa202").unwrap(),
            maturity: 42,
            ..Default::default()
        };
        assert!(
            !allowed(&prefs, &blocked_default),
            "the native blocked experience stays blocked at every rating"
        );
        prefs.experience_maturity = 0;
        assert!(!allowed(&prefs, &details));
        prefs.experience_maturity = 1;
        prefs.temporary_controllers = false;
        assert!(!allowed(&prefs, &details));
        prefs.temporary_controllers = true;
        prefs.blocked_experiences = format!("{};{id}", Uuid::from_u128(11));
        assert!(!allowed(&prefs, &details));
    }

    fn fixture() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.rlva_configure(true);
        world
    }

    fn attachment(world: &mut World, temporary: bool) -> Uuid {
        let id = Uuid::from_u128(0x726c766174656d70);
        let mut object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: id,
            owner_id: world.agent_id,
            point: 1,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: "Temporary controller".into(),
            desc: String::new(),
        });
        if !temporary {
            object.name_values = format!("AttachItemID STRING RW SV {}", Uuid::from_u128(99));
        }
        world.objects.upsert(world.main_region.unwrap(), object);
        id
    }

    fn say(world: &mut World, source: Uuid, message: &str) {
        world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            source_id: source,
            from_name: "Controller".into(),
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: message.into(),
            owner_id: world.agent_id,
        }));
    }

    fn resources(id: Uuid) -> Vec<ScriptAttachment> {
        vec![ScriptAttachment {
            location: "chest".into(),
            objects: vec![aurora_net::land::ScriptObject {
                id,
                name: "Temporary controller".into(),
                owner: Uuid::nil(),
                group_owned: false,
                memory: 0,
                urls: 0,
                location: None,
            }],
        }]
    }

    #[test]
    fn rlva_temporary_chat_gate_uses_native_metadata_without_an_experience_or_inventory_lookup() {
        let mut world = fixture();
        let source = attachment(&mut world, true);
        say(&mut world, source, "@fly=n");
        assert!(world.rlva.has("fly"));
        world.rlva.preferences.temporary_controllers = false;
        let rows = world.chat.len();
        say(&mut world, source, "@jump=n,version=123,clear");
        assert!(!world.rlva.has("jump") && world.rlva.has("fly"));
        assert!(
            world.rlva.out.is_empty(),
            "disallowed temporary OwnerSay is ordinary chat, without protocol replies"
        );
        assert_eq!(world.chat.len(), rows + 1);
        assert_eq!(world.chat.back().unwrap().text, "@jump=n,version=123,clear");
        world.rlva.preferences.temporary_controllers = true;
        say(&mut world, source, "@clear");
        assert!(!world.rlva.has("fly"));
        world.rlva.preferences.temporary_controllers = false;
        attachment(&mut world, false);
        say(&mut world, source, "@jump=n");
        assert!(world.rlva.has("jump"), "ordinary inventory attachments are unaffected");
        let unknown = Uuid::from_u128(1001);
        say(&mut world, unknown, "@sendchat=n");
        assert!(world.rlva.has("sendchat"), "unresolved sources retain native acceptance");
    }

    #[test]
    fn rlva_disallowed_experience_clears_late_identified_rules_and_blocks_new_commands_until_detach() {
        let mut world = fixture();
        let source = attachment(&mut world, true);
        let experience = Uuid::from_u128(42);
        let now = Instant::now();
        say(&mut world, source, "@fly=n");
        assert!(world.rlva.has("fly"));
        world.apply(aurora_net::NetEvent::ExperienceAttachment {
            experience,
            object_name: "Temporary controller".into(),
        });
        world.rlva_controllers_tick(now);
        assert!(
            world
                .experiences
                .take_commands()
                .iter()
                .any(|command| matches!(command, NetCommand::ExperienceInfo(ids) if ids.contains(&experience)))
        );
        assert!(world.rlva.has("fly"), "classification waits for experience details");
        world.apply(aurora_net::NetEvent::ExperienceInfo(vec![ExperienceDetails {
            id: experience,
            maturity: 13,
            ..Default::default()
        }]));
        world.rlva_controllers_tick(now);
        assert!(
            world
                .rlva
                .out
                .iter()
                .any(|command| matches!(command, NetCommand::Land(LandCommand::AttachmentScripts)))
        );
        world.rlva_controller_resources(&resources(source));
        assert!(!world.rlva.has("fly"));
        say(&mut world, source, "@fly=n,version=123,tpto:1/2/3=force");
        assert!(!world.rlva.has("fly"));
        assert!(world.rlva.pending.is_empty());
        assert!(world.rlva.controllers.accepts(source, Param::Clear));
        assert!(world.rlva.controllers.accepts(source, Param::Remove));
        assert!(!world.rlva.controllers.accepts(source, Param::Reply(123)));
        let key = world
            .objects
            .index_of_uuid(&source)
            .and_then(|index| world.objects.get(index))
            .unwrap()
            .key;
        world.objects.remove(&key);
        world.rlva_controllers_tick(now + Duration::from_secs(1));
        assert!(!world.rlva.controllers.blocked(source), "confirmed detach retires the block");
    }

    #[test]
    fn rlva_experience_resolution_ignores_regular_attachments_expires_unknowns_and_clears_on_disable() {
        let mut world = fixture();
        let regular = attachment(&mut world, false);
        let now = Instant::now();
        world.rlva.controllers.blocked.push(Blocked {
            root: None,
            name: "Temporary controller".into(),
            since: now,
            seen: false,
        });
        world.rlva_controller_resources(&resources(regular));
        assert!(!world.rlva.controllers.blocked(regular));
        world.rlva_controllers_tick(now + Duration::from_secs(301));
        assert!(world.rlva.controllers.blocked.is_empty());
        let unknown = Uuid::from_u128(888);
        world.rlva.controllers.blocked.push(Blocked {
            root: None,
            name: "Temporary controller".into(),
            since: now,
            seen: false,
        });
        world.rlva_controller_resources(&resources(unknown));
        assert!(
            world.rlva.controllers.blocked(unknown),
            "resources can identify an attachment before object arrival"
        );
        world.rlva_configure(false);
        assert!(!world.rlva.controllers.blocked(unknown));
    }

    #[test]
    fn rlva_experience_controller_classification_clears_only_the_reported_object() {
        let mut world = fixture();
        let root = attachment(&mut world, true);
        let parent = world
            .objects
            .index_of_uuid(&root)
            .and_then(|index| world.objects.get(index))
            .unwrap()
            .key
            .local_id;
        let child = Uuid::from_u128(0x726c76616368696c64);
        let mut object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: child,
            owner_id: world.agent_id,
            point: 1,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: "Child".into(),
            desc: String::new(),
        });
        object.state = 0;
        object.name_values.clear();
        object.parent_id = parent;
        world.objects.upsert(world.main_region.unwrap(), object);
        say(&mut world, child, "@jump=n");
        say(&mut world, root, "@fly=n,tpto:1/2/3=force,notify:7=add");
        assert!(world.rlva.has("jump") && world.rlva.has("fly") && !world.rlva.pending.is_empty());
        world.rlva.out.clear();
        world.rlva.controllers.blocked.push(Blocked {
            root: None,
            name: "Temporary controller".into(),
            since: Instant::now(),
            seen: false,
        });
        world.rlva_controller_resources(&resources(root));
        // RlvHandler::isBlockedObject is the reported object alone: its
        // child prim's rules stay and it may still add some.
        assert!(!world.rlva.has("fly") && world.rlva.pending.is_empty());
        assert!(world.rlva.has("jump") && !world.rlva.controllers.blocked(child));
        say(&mut world, child, "@unsit=n");
        assert!(world.rlva.has("unsit"));
        say(&mut world, root, "@fly=n");
        assert!(!world.rlva.has("fly"));
    }

    #[test]
    fn rlva_blocked_experience_object_is_cleared_with_a_notice_one_match_per_reply_and_one_request_each() {
        let mut world = fixture();
        let first = attachment(&mut world, true);
        let listener = Uuid::from_u128(0x6c69_7374);
        say(&mut world, listener, "@notify:7=add");
        world.rlva.out.clear();
        let now = Instant::now();
        for (n, name) in ["Temporary controller", "Temporary controller"].into_iter().enumerate() {
            let experience = Uuid::from_u128(500 + n as u128);
            world.apply(aurora_net::NetEvent::ExperienceInfo(vec![ExperienceDetails {
                id: experience,
                maturity: 13,
                ..Default::default()
            }]));
            world.rlva.controllers.attachment(experience, name.into(), now);
        }
        world.rlva_controllers_tick(now);
        let requests = world
            .rlva
            .out
            .iter()
            .filter(|command| matches!(command, NetCommand::Land(LandCommand::AttachmentScripts)))
            .count();
        assert_eq!(
            requests, 2,
            "RlvHandler::onExperienceAttach asks once per blocked experience, unthrottled"
        );
        world.rlva.out.clear();
        let second = Uuid::from_u128(0x726c_7661_7365_636f);
        let mut reply = resources(first);
        let copy = aurora_net::land::ScriptObject {
            id: second,
            ..reply[0].objects[0].clone()
        };
        reply[0].objects.push(copy);
        world.rlva_controller_resources(&reply);
        assert!(world.rlva.controllers.blocked(first) && !world.rlva.controllers.blocked(second));
        assert!(
            world
                .rlva
                .out
                .iter()
                .any(|command| matches!(command, NetCommand::ChatReply { channel: 7, message } if message == "/clear")),
            "the clear of a blocked object is heard even when it held nothing"
        );
        world.rlva_controller_resources(&reply);
        assert!(world.rlva.controllers.blocked(second), "the next reply resolves the next entry");
    }
}
