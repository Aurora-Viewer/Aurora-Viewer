//! AURORA_DEMO_FREEZE_WORLD=1: what the simulator sends while the photo
//! mode holds the world still (#1154), with
//! `AURORA_DEMO_SNAPSHOT=viewfinder-frozen` (frozen at frame 600). A
//! violet box and a teal disc of another resident rez beside our avatar at
//! 400; at 650 the box is derezzed and a script starts a fountain of
//! particles on the disc. Frozen, the box stays and the disc emits nothing;
//! before, the box went and the disc piled a burst a frame at its centre.

use aurora_net::NetEvent;
use aurora_net::objects::ParticleUpdate;
use glam::{Quat, Vec3};

pub fn on() -> bool {
    std::env::var("AURORA_DEMO_FREEZE_WORLD").is_ok_and(|v| v.trim() == "1")
}

pub const REZ_AT: u64 = 400;
pub const CHANGE_AT: u64 = 650;

const BOX: u32 = 980;
const DISC: u32 = 981;
/// Another resident, not in the demo's mute list.
const OWNER: uuid::Uuid = uuid::Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0F2E);

/// The box and the disc, beside where our avatar stands in the plaza.
fn props() -> Option<[aurora_net::objects::ObjectUpdate; 2]> {
    let me = super::events().into_iter().find_map(|ev| match ev {
        NetEvent::ObjectUpdates { objects, .. } => objects.into_iter().find(|o| o.full_id == super::DEMO_AGENT),
        _ => None,
    })?;
    let ground = me.position - Vec3::new(0.0, 0.0, 0.9);
    let mut cube = super::prim(
        BOX,
        ground + Vec3::new(1.2, 1.4, 0.35),
        Quat::IDENTITY,
        Vec3::splat(0.7),
        super::shape(super::LL_PCODE_PATH_LINE, super::LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        super::te([0.55, 0.36, 0.96, 1.0], 0, false, 0.0),
        aurora_prim::ExtraParams::default(),
        "",
    );
    let mut disc = super::prim(
        DISC,
        ground + Vec3::new(2.6, 0.6, 1.0),
        Quat::IDENTITY,
        Vec3::new(0.4, 0.4, 0.05),
        super::shape(super::LL_PCODE_PATH_LINE, super::LL_PCODE_PROFILE_CIRCLE, 100, 0, 0),
        super::te([0.37, 0.92, 0.83, 1.0], 0, false, 0.0),
        aurora_prim::ExtraParams::default(),
        "",
    );
    for o in [&mut cube, &mut disc] {
        o.owner_id = OWNER;
        o.update_flags &= !crate::ui::context::flags::OBJECT_YOU_OWNER;
    }
    Some([cube, disc])
}

/// The events of `frame`.
pub fn events(frame: u64) -> Vec<NetEvent> {
    match frame {
        REZ_AT => props()
            .map(|p| {
                vec![NetEvent::ObjectUpdates {
                    handle: super::HANDLE,
                    objects: p.into(),
                }]
            })
            .unwrap_or_default(),
        CHANGE_AT => {
            let mut events = vec![NetEvent::ObjectsKilled {
                handle: super::HANDLE,
                local_ids: vec![BOX],
            }];
            if let Some([_, mut disc]) = props() {
                disc.particles = ParticleUpdate::Set(super::fountain_particles());
                events.push(NetEvent::ObjectUpdates {
                    handle: super::HANDLE,
                    objects: vec![disc],
                });
            }
            events
        }
        _ => Vec::new(),
    }
}
