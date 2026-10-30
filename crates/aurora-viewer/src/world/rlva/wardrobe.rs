//! LLAppearanceMgr::updateAgentWearables notifications (llappearancemgr.cpp,
//! originally LGPL 2.1). Aurora's native appearance state is the confirmed
//! COF: report removals before additions, after the initial loaded baseline.
use super::super::{World, appearance};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Default)]
pub struct Wardrobe {
    generation: Option<u64>,
    worn: Option<BTreeMap<Uuid, u8>>,
}

impl World {
    pub fn rlva_wardrobe_tick(&mut self) {
        if !self.rlva.enabled() || self.rlva.wardrobe.generation == Some(self.inventory.generation) {
            return;
        }
        // Hold clothing worn/unworn notifies across a multi-step force wear so
        // collars and activity HUDs are not flooded mid-transaction.
        if self.rlva.appearance_busy || !self.rlva.appearance.is_empty() {
            return;
        }
        self.rlva.wardrobe.generation = Some(self.inventory.generation);
        let Some(cof) = appearance::cof(&self.inventory).filter(|cof| {
            appearance::complete(&self.inventory, *cof)
                && self
                    .inventory
                    .folders
                    .get(cof)
                    .is_some_and(|f| f.items.iter().all(|id| self.inventory.items.contains_key(id)))
        }) else {
            return;
        };
        let current: BTreeMap<_, _> = appearance::folder_links(&self.inventory, cof)
            .iter()
            .filter_map(|link| self.inventory.items.get(&link.target))
            .filter(|item| matches!(item.asset_type, 5 | 13))
            .filter_map(|item| {
                let kind = (item.flags & 0xff) as u8;
                ((kind as usize) < super::inventory::LAYERS.len()).then_some((item.id, kind))
            })
            .collect();
        if let Some(previous) = self.rlva.wardrobe.worn.replace(current.clone()) {
            let mut removed: Vec<_> = previous
                .iter()
                .filter(|(id, _)| !current.contains_key(id))
                .map(|(id, kind)| (*kind, *id))
                .collect();
            let mut added: Vec<_> = current
                .iter()
                .filter(|(id, _)| !previous.contains_key(id))
                .map(|(id, kind)| (*kind, *id))
                .collect();
            removed.sort_unstable();
            added.sort_unstable();
            // The native appearance manager reports these accepted outfit
            // deltas as legal, including a later lock arriving during loading.
            for (verb, items) in [("unworn", removed), ("worn", added)] {
                for (kind, _) in items {
                    self.rlva
                        .notify(&format!("{verb} legally {}", super::inventory::LAYERS[kind as usize]));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::{
        NetCommand,
        inventory::{InvFolder, InvItem},
    };
    const COF: Uuid = Uuid::from_u128(100);
    const A: Uuid = Uuid::from_u128(1);

    fn outfit(world: &mut World, items: &[(u128, u8)]) {
        let mut ids = Vec::new();
        for (item, kind) in items {
            let id = Uuid::from_u128(*item);
            ids.push(id);
            world.inventory.items.insert(
                id,
                InvItem {
                    id,
                    parent: COF,
                    asset_type: if *kind < 4 { 13 } else { 5 },
                    flags: *kind as u32,
                    name: "nostrip fixture".into(),
                    ..Default::default()
                },
            );
        }
        world.inventory.folders.insert(
            COF,
            Folder {
                info: InvFolder {
                    id: COF,
                    parent: world.inventory.root,
                    type_default: crate::world::outfit::FT_CURRENT_OUTFIT,
                    ..Default::default()
                },
                items: ids,
                children: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        world.inventory.generation += 1;
    }

    fn notices(world: &mut World) -> Vec<String> {
        std::mem::take(&mut world.rlva.out)
            .into_iter()
            .filter_map(|cmd| match cmd {
                NetCommand::ChatReply { message, .. } => Some(message),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn rlva_wear_notifications_skip_initial_load_and_report_every_removal_before_additions() {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.rlva.configure(true);
        w.rlva.process(A, super::super::Command::parse("notify:123;worn=n").unwrap());
        notices(&mut w);
        outfit(&mut w, &[(200, 0), (201, 4), (202, 5)]);
        w.rlva_wardrobe_tick();
        assert!(notices(&mut w).is_empty());
        outfit(&mut w, &[(203, 0), (204, 4), (205, 8)]);
        w.rlva_wardrobe_tick();
        assert_eq!(
            notices(&mut w),
            [
                "/unworn legally shape",
                "/unworn legally shirt",
                "/unworn legally pants",
                "/worn legally shape",
                "/worn legally shirt",
                "/worn legally jacket"
            ]
        );
        w.rlva_wardrobe_tick();
        assert!(notices(&mut w).is_empty(), "unchanged frames do no snapshot work or notifications");
        w.rlva.configure(false);
        outfit(&mut w, &[]);
        w.rlva_wardrobe_tick();
        assert!(notices(&mut w).is_empty());
    }

    #[test]
    fn rlva_wear_notifications_keep_previous_types_across_missing_data_and_reset_on_disable() {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.rlva.configure(true);
        w.rlva.process(A, super::super::Command::parse("notify:123=n").unwrap());
        notices(&mut w);
        outfit(&mut w, &[(200, 4)]);
        w.rlva_wardrobe_tick();
        w.inventory.items.remove(&Uuid::from_u128(200));
        w.inventory.generation += 1;
        w.rlva_wardrobe_tick();
        assert!(notices(&mut w).is_empty(), "missing originals are not removals");
        outfit(&mut w, &[(201, 5)]);
        w.rlva.process(A, super::super::Command::parse("addoutfit=n").unwrap());
        notices(&mut w);
        w.rlva_wardrobe_tick();
        assert_eq!(notices(&mut w), ["/unworn legally shirt", "/worn legally pants"]);
        w.rlva.configure(false);
        outfit(&mut w, &[(202, 8)]);
        w.rlva.configure(true);
        w.rlva.process(A, super::super::Command::parse("notify:123=n").unwrap());
        notices(&mut w);
        w.rlva_wardrobe_tick();
        assert!(notices(&mut w).is_empty(), "re-enabling starts a fresh appearance baseline");
    }
}
