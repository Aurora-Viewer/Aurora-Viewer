//! RlvStrings::getVersion/Num/ImplNum and RlvSettings::initCompatibilityMode
//! (rlvcommon.cpp, LGPL 2.1). Compatibility applies only to a worn original's
//! creator or full, case-insensitive name match. Regexes compile once per edit.
use super::{Command, Param};
use aurora_net::inventory::InvItem;
use std::collections::BTreeSet;
use uuid::Uuid;

pub const VERSION_NEW: &str = "RestrainedLove viewer v3.4.3 (RLVa 2.4.2; Aurora)";

#[derive(Default)]
pub struct Version {
    configuration: Option<String>,
    creators: BTreeSet<Uuid>,
    names: Vec<fancy_regex::Regex>,
    compatible: BTreeSet<Uuid>,
}

impl Version {
    pub fn prepare(&mut self, list: &str, source: Uuid, item: Option<&InvItem>) {
        if self.configuration.as_deref() != Some(list) {
            self.configuration = Some(list.to_owned());
            self.creators.clear();
            self.names.clear();
            self.compatible.clear();
            // Settings are local input, but malformed/expensive patterns must
            // not stall a frame or an object's version handshake.
            if list.len() <= 65536 {
                for entry in list.split(';').filter(|entry| !entry.is_empty()).take(128) {
                    if let Some(id) = entry.strip_prefix("creator:") {
                        if id.len() == 36
                            && let Ok(id) = Uuid::parse_str(id)
                        {
                            self.creators.insert(id);
                        }
                    } else if let Some(name) = entry.strip_prefix("name:").filter(|name| !name.is_empty() && name.len() <= 4096) {
                        let expression = format!(r"(?i)\A(?:{name})\z");
                        match fancy_regex::RegexBuilder::new(&expression).backtrack_limit(100_000).build() {
                            Ok(regex) => self.names.push(regex),
                            Err(_) => log::warn!("RLVa compatibility name expression is invalid"),
                        }
                    }
                }
            }
        }
        let compatible = item.is_some_and(|item| {
            self.creators.contains(&item.creator) || self.names.iter().any(|regex| regex.is_match(&item.name).unwrap_or(false))
        });
        if compatible {
            if self.compatible.len() < 1024 {
                self.compatible.insert(source);
            }
        } else {
            self.compatible.remove(&source);
        }
    }

    pub fn forget(&mut self, source: Uuid) {
        self.compatible.remove(&source);
    }

    pub fn query(&self, source: Uuid, command: &Command) -> Option<String> {
        if !matches!(command.param, Param::Reply(_)) {
            return None;
        }
        let compatible = self.compatible.contains(&source);
        match command.behaviour.as_str() {
            "version" | "versionnew" => Some(format!(
                "{} viewer v{} (RLVa 2.4.2; Aurora)",
                if command.behaviour == "version" {
                    "RestrainedLife"
                } else {
                    "RestrainedLove"
                },
                if compatible { "2.9.28" } else { "3.4.3" }
            )),
            "versionnum" => Some(
                match command.option.as_str() {
                    "" => {
                        if compatible {
                            "2092800"
                        } else {
                            "3040300"
                        }
                    }
                    "impl" => "2040213",
                    _ => "",
                }
                .into(),
            ),
            _ => None,
        }
    }
}

impl crate::world::World {
    pub(crate) fn rlva_demo_version_seed(&mut self) {
        let item = Uuid::from_u128(0x726c7661);
        self.inventory.add_items(vec![InvItem {
            id: item,
            parent: self.inventory.root,
            owner: self.agent_id,
            creator: Uuid::from_u128(0x726c766163726561),
            name: "Legacy controller".into(),
            asset_type: 6,
            inv_type: 6,
            ..Default::default()
        }]);
        if let Some(handle) = self.main_region {
            let object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
                item_id: item,
                owner_id: self.agent_id,
                point: 1,
                add: true,
                flags: 0,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
                name: "Legacy controller".into(),
                desc: String::new(),
            });
            self.apply(aurora_net::NetEvent::ObjectUpdates {
                handle,
                objects: vec![object],
            });
        }
    }

    pub(crate) fn rlva_demo_version_report(&self) {
        let reply = |channel| {
            self.rlva
                .out
                .iter()
                .find_map(|command| match command {
                    aurora_net::NetCommand::ChatReply { channel: actual, message } if *actual == channel => Some(message.as_str()),
                    _ => None,
                })
                .unwrap_or("missing")
        };
        log::info!(
            "RLVa version demo: legacy={}, modern={}, numeric={}, implementation={}",
            reply(123),
            reply(124),
            reply(125),
            reply(126)
        );
    }

    pub(crate) fn rlva_prepare_version(&mut self, source: Uuid, message: &str) {
        if !self.rlva.enabled()
            || !message.strip_prefix('@').is_some_and(|message| {
                message.split(',').any(|command| {
                    let name = command.split([':', '=']).next().unwrap_or_default();
                    ["version", "versionnew", "versionnum"]
                        .iter()
                        .any(|candidate| name.eq_ignore_ascii_case(candidate))
                })
            })
        {
            return;
        }
        let item = if self.rlva.preferences.compatibility_list.is_empty() {
            None
        } else {
            self.rlva_own_attachment_root(source)
                .and_then(|root| self.rlva_attachment_item(root))
                .and_then(|item| self.inventory.items.get(&item))
        };
        self.rlva.version.prepare(&self.rlva.preferences.compatibility_list, source, item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rlva_version_negotiation_keeps_legacy_new_implementation_and_invalid_option_semantics() {
        let version = Version::default();
        let source = Uuid::from_u128(1);
        assert!(
            version
                .query(source, &Command::parse("version:any=123").unwrap())
                .unwrap()
                .starts_with("RestrainedLife viewer v3.4.3")
        );
        assert_eq!(
            version.query(source, &Command::parse("versionnew:any=123").unwrap()),
            Some(VERSION_NEW.into())
        );
        assert_eq!(
            version.query(source, &Command::parse("versionnum=123").unwrap()),
            Some("3040300".into())
        );
        assert_eq!(
            version.query(source, &Command::parse("versionnum:impl=123").unwrap()),
            Some("2040213".into())
        );
        assert_eq!(
            version.query(source, &Command::parse("versionnum:unknown=123").unwrap()),
            Some(String::new())
        );
    }

    #[test]
    fn rlva_compatibility_matches_creator_or_complete_name_and_refreshes_changed_settings() {
        let mut version = Version::default();
        let source = Uuid::from_u128(1);
        let creator = Uuid::from_u128(2);
        let mut item = InvItem {
            creator,
            name: "Legacy Controller".into(),
            ..Default::default()
        };
        let query = Command::parse("versionnum=123").unwrap();
        version.prepare(&format!("creator:{creator}"), source, Some(&item));
        assert_eq!(version.query(source, &query), Some("2092800".into()));
        version.prepare("name:legacy", source, Some(&item));
        assert_eq!(
            version.query(source, &query),
            Some("3040300".into()),
            "native regex matching covers the whole name"
        );
        version.prepare("name:legacy.*;name:[invalid", source, Some(&item));
        assert_eq!(version.query(source, &query), Some("2092800".into()));
        version.prepare("name:legacy.*;name:[invalid", source, None);
        assert_eq!(
            version.query(source, &query),
            Some("3040300".into()),
            "detached/missing originals cannot keep compatibility"
        );
        item.name = "Controller Controller".into();
        version.prepare(r"name:(\w+) \1", source, Some(&item));
        assert_eq!(
            version.query(source, &query),
            Some("2092800".into()),
            "backreferences retain the native Perl use case"
        );
        version.forget(source);
        assert_eq!(version.query(source, &query), Some("3040300".into()));
    }

    #[test]
    fn rlva_version_handshake_resolves_the_worn_original_before_the_immediate_reply() {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.rlva_configure(true);
        let item = Uuid::from_u128(101);
        let creator = Uuid::from_u128(2);
        world.inventory.items.insert(
            item,
            InvItem {
                id: item,
                creator,
                name: "Old controller".into(),
                asset_type: 6,
                ..Default::default()
            },
        );
        let object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: item,
            owner_id: world.agent_id,
            point: 1,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: "Old controller".into(),
            desc: String::new(),
        });
        world.objects.upsert(world.main_region.unwrap(), object);
        world.rlva.preferences.compatibility_list = format!("creator:{creator}");
        world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            source_id: item,
            from_name: "Old controller".into(),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: "@versionnum=123,versionnum:impl=124,version:ignored=125".into(),
        }));
        assert!(world.rlva.pending.is_empty());
        assert!(
            world
                .rlva
                .out
                .iter()
                .any(|command| matches!(command, aurora_net::NetCommand::ChatReply { channel: 123, message } if message == "2092800"))
        );
        assert!(
            world
                .rlva
                .out
                .iter()
                .any(|command| matches!(command, aurora_net::NetCommand::ChatReply { channel: 124, message } if message == "2040213"))
        );
        assert!(world.rlva.out.iter().any(|command| matches!(command, aurora_net::NetCommand::ChatReply { channel: 125, message } if message.starts_with("RestrainedLife viewer v2.9.28"))));
    }
}
