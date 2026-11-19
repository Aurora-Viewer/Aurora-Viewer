//! AURORA_DEMO_AVATAR_CACHE=1: avatars drawn from the cache while the
//! region sends them (world/avatar_cache).
//!
//! The cache holds how Violet Wolf looked the last time it was seen (the
//! classic look, without its pigtails then) and what it wore: its hat,
//! still pink then, and its orb. The region sends its appearance 2 s after
//! the loading fade: until then the kept look is drawn (no cloud), then
//! the region's, whose pigtails grow. That appearance lists both objects,
//! but the region only sends them 6 s after the fade: until then their
//! copies are drawn, then the region's own, the hat violet now. With AURORA_DEMO_OUTFIT_RESTORE=warm, the cache also holds our demo
//! outfit's hair, earrings and boots (drawn visible for this scenario):
//! they show from the start instead of when the restore puts them back.
//! The log ends with `avatar cache: N copies drawn, N handed over to
//! the region's objects`.

use crate::world::World;
use crate::world::avatar_cache::file::{KeptAvatar, WornObject, WornPrim};
use aurora_net::objects::ObjectUpdate;
use aurora_net::{AttachRequest, NetEvent};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The demo region starts once the loading fade is over.
const FIRST_FRAME: u64 = 300;
/// How long after that the region sends Violet Wolf's appearance, then its
/// worn objects.
const APPEARANCE_LATE: Duration = Duration::from_secs(2);
const LATE: Duration = Duration::from_secs(6);
/// The demo outfit's objects kept in the cache: hair, earrings, boots
/// (world::appearance::seed_demo), with their points.
pub const KEPT_OUTFIT: [(u128, u8); 3] = [(717, 2), (718, 13), (720, 7)];

/// The scenario is on (read once).
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AURORA_DEMO_AVATAR_CACHE").is_some())
}

pub struct AvatarCacheDemo {
    start: Option<Instant>,
    /// Violet Wolf's appearance from the region, until it is sent.
    appearance: Option<aurora_net::AvatarAppearance>,
    sent: bool,
}

impl AvatarCacheDemo {
    pub fn from_env() -> Option<Self> {
        enabled().then_some(AvatarCacheDemo {
            start: None,
            appearance: None,
            sent: false,
        })
    }

    /// The kept sets, as if read from the store, and Violet Wolf's
    /// appearance listing its two objects, to send.
    pub fn setup(&mut self, world: &mut World) {
        let saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let wolf = crate::demo::wolf_attachments(true);
        let listed = wolf.iter().map(|o| (o.full_id, attach_point(o.state))).collect::<Vec<_>>();
        let shapes = &world.avatar_lib.shape_params;
        let Some(NetEvent::Appearance(mut look)) = crate::demo::classic_events(shapes).into_iter().nth(1) else {
            return;
        };
        // the classic look of crate::demo::classic_events, its pigtails (785)
        // not grown yet
        let mut kept_look = crate::world::avatar_cache::capture::look_of(&look);
        kept_look.visual_params = shapes.values_with(&[(637, 1.0), (157, 1.0), (105, 0.9), (763, 1.0), (133, 1.0), (134, 1.0)]);
        let mut kept = kept_set(&wolf, saved_at, |_| Uuid::nil());
        kept.look = Some(kept_look);
        world.avatar_cache.insert_set(crate::demo::DEMO_LOUP, kept);
        let outfit: Vec<ObjectUpdate> = KEPT_OUTFIT
            .iter()
            .map(|&(item, point)| crate::demo::outfit_attachment(&request(Uuid::from_u128(item), point)))
            .collect();
        // (the demo's outfit objects take their item's id)
        world
            .avatar_cache
            .insert_set(world.agent_id, kept_set(&outfit, saved_at, |o| o.full_id));
        look.attachments = listed;
        self.appearance = Some(look);
        log::info!("demo avatar cache: kept look and objects for Violet Wolf, kept objects for our outfit");
    }

    /// Violet Wolf's appearance `APPEARANCE_LATE` after `FIRST_FRAME`, its
    /// worn objects `LATE` after.
    pub fn tick(&mut self, frame: u64, world: &mut World, now: Instant) {
        if frame < FIRST_FRAME || self.sent {
            return;
        }
        let start = *self.start.get_or_insert(now);
        let waited = now.saturating_duration_since(start);
        if waited >= APPEARANCE_LATE
            && let Some(look) = self.appearance.take()
        {
            log::info!("demo avatar cache: the region sends Violet Wolf's appearance");
            let _ = world.apply(NetEvent::Appearance(look));
        }
        if waited < LATE {
            return;
        }
        self.sent = true;
        log::info!(
            "demo avatar cache: the region sends Violet Wolf's worn objects ({} copies drawn now)",
            world.avatar_cache.copy_count()
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
fn kept_set(objects: &[ObjectUpdate], saved_at: u64, item: impl Fn(&ObjectUpdate) -> Uuid) -> KeptAvatar {
    KeptAvatar {
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
        look: None,
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
