//! AURORA_DEMO_SEAT_KILL=1: a seat deleted under the resident sitting on it
//! (#746, a carry or pose object that dies before the simulator stands its
//! sitter up). Violet Wolf sits on a cushion at frame 700; the cushion is
//! killed at 800 and the simulator stands Wolf up at 900. Wolf must stay
//! where it sat the whole time, hat and orb on: before, it left the world
//! with the cushion at 800 and came back at 900 without them, as the
//! simulator does not send again what it wears.

use aurora_net::NetEvent;
use aurora_net::objects::ObjectUpdate;
use glam::{Quat, Vec3};

pub fn on() -> bool {
    std::env::var("AURORA_DEMO_SEAT_KILL").is_ok_and(|v| v.trim() == "1")
}

pub const SIT_AT: u64 = 700;
pub const KILL_AT: u64 = 800;
pub const STAND_AT: u64 = 900;

const CUSHION: u32 = 960;

/// Wolf as the plaza sends it.
fn wolf() -> Option<ObjectUpdate> {
    super::events().into_iter().find_map(|ev| match ev {
        NetEvent::ObjectUpdates { objects, .. } => objects.into_iter().find(|o| o.full_id == super::DEMO_LOUP),
        _ => None,
    })
}

/// The events of `frame`.
pub fn events(frame: u64) -> Vec<NetEvent> {
    let update = |objects| NetEvent::ObjectUpdates {
        handle: super::HANDLE,
        objects,
    };
    match frame {
        SIT_AT => {
            let Some(mut wolf) = wolf() else {
                return Vec::new();
            };
            let cushion = super::prim(
                CUSHION,
                wolf.position - Vec3::new(0.0, 0.0, 0.7),
                Quat::IDENTITY,
                Vec3::new(0.8, 0.8, 0.2),
                super::shape(super::LL_PCODE_PATH_LINE, super::LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
                super::te([0.48, 0.3, 0.75, 1.0], 0, false, 0.0),
                aurora_prim::ExtraParams::default(),
                aurora_i18n::tr!("demo-seat-kill-cushion"),
            );
            wolf.parent_id = CUSHION;
            wolf.position = Vec3::new(0.0, 0.0, 0.7);
            vec![update(vec![cushion, wolf])]
        }
        KILL_AT => vec![NetEvent::ObjectsKilled {
            handle: super::HANDLE,
            local_ids: vec![CUSHION],
        }],
        STAND_AT => wolf().map(|w| vec![update(vec![w])]).unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wolf_keeps_what_it_wears_through_the_scenario() {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = super::super::DEMO_AGENT;
        world.main_region = Some(super::super::HANDLE);
        for ev in super::super::events() {
            if matches!(ev, NetEvent::ObjectUpdates { .. }) {
                let _ = world.apply(ev);
            }
        }
        let worn = |w: &crate::world::World| {
            let o = w.objects.get(w.objects.index_of_uuid(&super::super::DEMO_LOUP)?)?;
            Some((o.parent_id, w.objects.children_of(&o.key).len()))
        };
        assert_eq!(worn(&world), Some((0, 2)));
        for (frame, parent) in [(SIT_AT, CUSHION), (KILL_AT, 0), (STAND_AT, 0)] {
            for ev in events(frame) {
                let _ = world.apply(ev);
            }
            assert_eq!(worn(&world), Some((parent, 2)), "frame {frame}");
        }
    }
}
