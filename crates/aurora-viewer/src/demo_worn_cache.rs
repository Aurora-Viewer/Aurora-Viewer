//! AURORA_DEMO_WORN_CACHE=1: worn objects drawn from the cache while the
//! region sends them (world/worn_cache).
//!
//! The cache holds what Violet Wolf wore the last time it was seen: its
//! hat, still pink then, and its orb. Its appearance (the classic look)
//! lists both, but the region only sends them 6 s after the loading fade:
//! until then the copies are drawn, then the region's own, the hat violet
//! now. With AURORA_DEMO_OUTFIT_RESTORE=warm, the cache also holds our demo
//! outfit's hair, earrings and boots (drawn visible for this scenario):
//! they show from the start instead of when the restore puts them back.
//! The log ends with `worn objects cache: N copies drawn, N handed over to
//! the region's objects`.

use crate::world::World;
use crate::world::worn_cache::file::{WornObject, WornPrim, WornSet};
use aurora_net::objects::ObjectUpdate;
use aurora_net::{AttachRequest, NetEvent};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The demo region starts once the loading fade is over.
const FIRST_FRAME: u64 = 300;
/// How long after that the region sends Violet Wolf's worn objects.
const LATE: Duration = Duration::from_secs(6);
/// The demo outfit's objects kept in the cache: hair, earrings, boots
/// (world::appearance::seed_demo), with their points.
pub const KEPT_OUTFIT: [(u128, u8); 3] = [(717, 2), (718, 13), (720, 7)];

/// The scenario is on (read once).
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AURORA_DEMO_WORN_CACHE").is_some())
}

pub struct WornCacheDemo {
    start: Option<Instant>,
    sent: bool,
}

impl WornCacheDemo {
    pub fn from_env() -> Option<Self> {
        enabled().then_some(WornCacheDemo { start: None, sent: false })
    }

    /// The kept sets, as if read from the store, and Violet Wolf's
    /// appearance listing its two objects.
    pub fn setup(&mut self, world: &mut World) {
        let saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let wolf = crate::demo::wolf_attachments(true);
        let listed = wolf
            .iter()
            .map(|o| (o.full_id, attach_point(o.state)))
            .collect::<Vec<_>>();
        world.worn_cache.insert_set(crate::demo::DEMO_LOUP, kept_set(&wolf, saved_at, |_| Uuid::nil()));
        let outfit: Vec<ObjectUpdate> = KEPT_OUTFIT
            .iter()
            .map(|&(item, point)| crate::demo::outfit_attachment(&request(Uuid::from_u128(item), point)))
            .collect();
        // (the demo's outfit objects take their item's id)
        world.worn_cache.insert_set(world.agent_id, kept_set(&outfit, saved_at, |o| o.full_id));
        if let Some(NetEvent::Appearance(mut look)) =
            crate::demo::classic_events(&world.avatar_lib.shape_params).into_iter().nth(1)
        {
            look.attachments = listed;
            let _ = world.apply(NetEvent::Appearance(look));
        }
        log::info!("demo worn cache: kept sets for Violet Wolf and our outfit");
    }

    /// Violet Wolf's worn objects, `LATE` after `FIRST_FRAME`.
    pub fn tick(&mut self, frame: u64, world: &mut World, now: Instant) {
        if frame < FIRST_FRAME || self.sent {
            return;
        }
        let start = *self.start.get_or_insert(now);
        if now.saturating_duration_since(start) < LATE {
            return;
        }
        self.sent = true;
        log::info!(
            "demo worn cache: the region sends Violet Wolf's worn objects ({} copies drawn now)",
            world.worn_cache.copy_count()
        );
        let _ = world.apply(NetEvent::ObjectUpdates {
            handle: world.main_region.unwrap_or_default(),
            objects: crate::demo::wolf_attachments(false),
        });
    }
}

/// The attachment point of an object's `state` (its nibbles swapped).
fn attach_point(state: u8) -> u8 {
    state.rotate_left(4)
}

/// One worn object per update (each a single prim).
fn kept_set(objects: &[ObjectUpdate], saved_at: u64, item: impl Fn(&ObjectUpdate) -> Uuid) -> WornSet {
    WornSet {
        saved_at,
        objects: objects
            .iter()
            .map(|o| WornObject {
                root_id: o.full_id,
                item_id: item(o),
                point: attach_point(o.state),
                prims: vec![WornPrim {
                    update_flags: 0,
                    data: aurora_net::objects::encode_compressed(o),
                    overrides: Vec::new(),
                }],
            })
            .collect(),
    }
}

/// A request to wear `item` on `point`, as the outfit sends it.
pub fn request(item: Uuid, point: u8) -> AttachRequest {
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
