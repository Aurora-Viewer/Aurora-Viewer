//! AURORA_DEMO_OUTFIT_RESTORE=warm|cold: the login restore of the outfit's
//! attachments against a region that puts part of it back by itself, as
//! the busy region of 2026-10-27 did.
//!
//! The Current Outfit of the Appearance demo links five objects. Like a
//! region at login, the demo region puts back by itself the mesh hair 3 s
//! after arriving and the boots 8 s after; the earrings, the ring and the
//! HUD are the viewer's to request (RezMultipleAttachmentsFromInv). For
//! this scenario the ring is a toe ring, on the boots' point. As the grid
//! logs suggest, a request for an object the region is still putting back
//! fails ("Attempt to rez an object failed"), and an object added on a
//! point before the region's own object arrives there comes off when it
//! does. With `cold`, the inventory knows none of the five items until
//! FetchItems asks for them (a cold inventory cache).
//!
//! The region's 5 s pause between its two objects is longer than the
//! viewer waits for it to be quiet, the unfavourable case: the log shows
//! `outfit: the region put back 1 of 5 objects by itself`, `outfit:
//! wearing 4 attachments` (the boots' request fails, the region is still
//! putting them back), then `'…' came off` for the ring when the boots
//! arrive, `outfit: wearing 1 taken-off attachments` once the region is
//! quiet again, and `outfit: 5 of 5 attachments worn` after the restore's
//! 30 s check. The Appearance window opens on its Worn tab then (opened
//! sooner, it would look the items up itself).

use crate::world::World;
use aurora_net::inventory::{FolderContents, InvItem};
use aurora_net::{AttachRequest, NetCommand, NetEvent};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The demo region starts once the loading fade is over.
const FIRST_FRAME: u64 = 300;
/// The objects the region puts back by itself, when, and on which point.
const REGION: [(Duration, u128, u8); 2] = [(Duration::from_secs(3), 717, 2), (Duration::from_secs(8), 720, 7)];
/// The ring of the demo outfit, worn here on the boots' point.
const RING: u128 = 719;
const OUTFIT: std::ops::RangeInclusive<u128> = 717..=721;

pub struct OutfitRestoreDemo {
    cold: bool,
    start: Option<Instant>,
    /// Region objects already put back.
    restored: usize,
    /// Items hidden from the inventory until FetchItems (`cold`).
    hidden: Vec<InvItem>,
    /// Objects the viewer added on a point the region has not filled yet:
    /// they come off when its object arrives there.
    added: HashMap<u8, Vec<Uuid>>,
    /// The Appearance window was opened at the end of the restore.
    shown: bool,
}

impl OutfitRestoreDemo {
    pub fn from_env() -> Option<Self> {
        let mode = std::env::var("AURORA_DEMO_OUTFIT_RESTORE").ok()?;
        Some(OutfitRestoreDemo {
            cold: mode == "cold",
            start: None,
            restored: 0,
            hidden: Vec::new(),
            added: HashMap::new(),
            shown: false,
        })
    }

    /// The demo outfit in the inventory, the ring moved to the boots' point.
    pub fn setup(&mut self, world: &mut World) {
        let agent = world.agent_id;
        crate::world::appearance::seed_demo(&mut world.inventory, agent);
        if let Some(ring) = world.inventory.items.get_mut(&Uuid::from_u128(RING)) {
            ring.flags = u32::from(REGION[1].2);
        }
    }

    /// The region's own restore, from `FIRST_FRAME` on.
    pub fn tick(&mut self, frame: u64, world: &mut World, now: Instant) {
        if frame < FIRST_FRAME {
            return;
        }
        if self.start.is_none() {
            // the demo's other seeds run until then: the items are hidden
            // on arrival
            if self.cold {
                self.hidden = OUTFIT.filter_map(|id| world.inventory.items.remove(&Uuid::from_u128(id))).collect();
            }
            log::info!(
                "demo outfit restore: arrived in the demo region ({} items to look up)",
                self.hidden.len()
            );
            world.demo_outfit_region = true;
            self.start = Some(now);
        }
        let Some(start) = self.start else {
            return;
        };
        while let Some(&(after, id, point)) = REGION.get(self.restored)
            && now.saturating_duration_since(start) >= after
        {
            self.restored += 1;
            // the region's object replaces what the viewer added there
            let off: Vec<u32> = self
                .added
                .remove(&point)
                .unwrap_or_default()
                .iter()
                .map(|id| local_id(*id))
                .collect();
            if !off.is_empty() {
                world.apply(NetEvent::ObjectsKilled {
                    handle: world.main_region.unwrap_or_default(),
                    local_ids: off,
                });
            }
            log::info!("demo outfit restore: the region puts back item {id} on point {point}");
            wear(world, &[request(Uuid::from_u128(id), point)]);
        }
    }

    /// True once, when the login restore is over: time to show the Worn tab.
    pub fn restore_over(&mut self, world: &World) -> bool {
        let over = !self.shown && self.start.is_some() && world.outfit.is_done();
        self.shown |= over;
        over
    }

    /// Answers the outfit's network commands; false for the others.
    pub fn handle(&mut self, cmd: &NetCommand, world: &mut World, now: Instant) -> bool {
        match cmd {
            NetCommand::FetchInventory { folders, .. } => {
                let Some(cof) = crate::world::appearance::cof(&world.inventory).filter(|cof| folders.contains(cof)) else {
                    return false;
                };
                let (version, items) = world
                    .inventory
                    .folders
                    .get(&cof)
                    .map(|f| {
                        (
                            f.info.version,
                            f.items.iter().filter_map(|id| world.inventory.items.get(id).cloned()).collect(),
                        )
                    })
                    .unwrap_or_default();
                world.inventory.apply(vec![FolderContents {
                    folder_id: cof,
                    owner_id: world.agent_id,
                    version,
                    folders: Vec::new(),
                    items,
                }]);
                true
            }
            NetCommand::FetchItems { items, .. } if !self.hidden.is_empty() => {
                let (found, rest) = std::mem::take(&mut self.hidden).into_iter().partition(|it| items.contains(&it.id));
                self.hidden = rest;
                world.inventory.add_items(found);
                true
            }
            NetCommand::RezAttachments(list) => {
                let elapsed = self.start.map(|s| now.saturating_duration_since(s)).unwrap_or_default();
                let mut worn = Vec::new();
                let mut failed = false;
                for a in list {
                    let region = REGION.iter().position(|(_, id, _)| Uuid::from_u128(*id) == a.item_id);
                    match region {
                        // the region is still putting it back: the request races with it
                        Some(i) if i >= self.restored => failed = true,
                        Some(_) => {}
                        None => {
                            let point = if a.point == 0 { (a.flags & 0xff) as u8 } else { a.point };
                            let pending = REGION.iter().skip(self.restored).any(|(_, _, p)| *p == point);
                            if pending {
                                self.added.entry(point).or_default().push(a.item_id);
                            }
                            worn.push(a.clone());
                        }
                    }
                }
                log::info!(
                    "demo outfit restore: {} requested {:.1} s after arriving, {} worn",
                    list.len(),
                    elapsed.as_secs_f32(),
                    worn.len()
                );
                if failed {
                    world.alerts.push_back(aurora_i18n::tr!("demo-outfit-rez-failed").into());
                }
                wear(world, &worn);
                true
            }
            _ => false,
        }
    }
}

fn local_id(item: Uuid) -> u32 {
    // as crate::demo::outfit_attachment numbers them
    30_000 + (item.as_u128() & 0xffff) as u32
}

fn request(item: Uuid, point: u8) -> AttachRequest {
    AttachRequest {
        item_id: item,
        owner_id: Uuid::nil(),
        point,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    }
}

fn wear(world: &mut World, list: &[AttachRequest]) {
    if list.is_empty() {
        return;
    }
    world.apply(NetEvent::ObjectUpdates {
        handle: world.main_region.unwrap_or_default(),
        objects: list.iter().map(crate::demo::outfit_attachment).collect(),
    });
}
