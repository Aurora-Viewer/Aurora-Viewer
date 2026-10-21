//! Avatar body physics in the demo (AURORA_DEMO_PHYSICS, docs/TESTING.md):
//! our avatar and the one in front of it wear a physics layer and a rigged
//! "body", a ball on each breast, the belly and the butt weighted to the
//! LEFT_PEC, RIGHT_PEC, BELLY and BUTT collision volumes as mesh bodies are.
//! Both turn their side to the camera and hop in place twice every two
//! seconds while their chest rolls from side to side: the balls bounce after
//! each landing, then settle. `still`: the same pose without the hops, the
//! balls stay where they are; `classic`: the hops without the balls, the
//! system body's own breasts, belly and butt bouncing (its morphs of the
//! driven parameters);
//! `off`: they hop with Preferences › Graphics › Avatar physics off, the
//! balls follow the body without bouncing.

use super::{DEMO_AGENT, DEMO_LOUP, HANDLE, IDLE_ANIM, prim, te};
use aurora_assets::Animation;
use aurora_assets::anim::{JointMotion, PosKey, RotKey};
use aurora_net::NetEvent;
use aurora_prim::params::LL_SCULPT_TYPE_MESH;
use aurora_prim::{ExtraParams, SculptParams};
use glam::{Quat, Vec3};
use uuid::Uuid;

/// The hops.
pub const HOP_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0660);
/// Mesh asset of the balls.
pub const BODY_MESH: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0003_0000_0100);
/// Frame the hops start at, once the avatars are there.
const HOP_FRAME: u64 = 240;
/// Local ids of the worn balls (one per avatar).
const BODY_LOCAL: u32 = 9660;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Hop,
    Still,
    Off,
    Classic,
}

pub fn mode() -> Option<Mode> {
    static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        let value = std::env::var("AURORA_DEMO_PHYSICS").ok()?;
        Some(match value.trim() {
            "still" => Mode::Still,
            "off" => Mode::Off,
            "classic" => Mode::Classic,
            _ => Mode::Hop,
        })
    })
}

/// The avatars of the scenario, by local id.
const AVATARS: [(Uuid, u32); 2] = [(DEMO_AGENT, 9000), (DEMO_LOUP, 9001)];

/// A lively physics layer: (parameter, weight). Mass, gravity and drag of
/// the breasts, belly and butt, then max effect, spring, gain and damping of
/// each motion.
pub const LAYER: [(i32, f32); 33] = [
    (10000, 0.2),
    (10001, 4.0),
    (10002, 1.0),
    (10003, 1.5),
    (10004, 25.0),
    (10005, 40.0),
    (10006, 0.15),
    (10007, 1.0),
    (10008, 25.0),
    (10009, 30.0),
    (10010, 0.2),
    (10011, 0.2),
    (10012, 2.0),
    (10013, 1.0),
    (10014, 1.0),
    (10015, 20.0),
    (10016, 30.0),
    (10017, 0.2),
    (10018, 0.2),
    (10019, 2.0),
    (10020, 1.0),
    (10021, 1.5),
    (10022, 20.0),
    (10023, 40.0),
    (10024, 0.15),
    (10025, 1.0),
    (10026, 20.0),
    (10027, 30.0),
    (10028, 0.2),
    (10029, 1.0),
    (10030, 25.0),
    (10031, 30.0),
    (10032, 0.2),
];

/// Two hops in two seconds, the pelvis rising 9 cm and landing at once,
/// while the chest rolls one way then the other; the body turned a quarter
/// turn, its side to the camera behind the avatar. `Still`: the same turn
/// only.
pub fn animation(mode: Mode) -> Animation {
    let amplitude = if mode == Mode::Still { 0.0 } else { 1.0 };
    let hop = [
        (0.0, 0.0),
        (0.15, 0.06),
        (0.3, 0.09),
        (0.45, 0.06),
        (0.6, 0.0),
        (1.0, 0.0),
        (1.15, 0.06),
        (1.3, 0.09),
        (1.45, 0.06),
        (1.6, 0.0),
        (2.0, 0.0),
    ];
    Animation {
        base_priority: 4,
        duration: 2.0,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: 2.0,
        looping: true,
        ease_in: 0.2,
        ease_out: 0.2,
        hand_pose: 0,
        joints: vec![
            JointMotion {
                joint_name: "mPelvis".into(),
                priority: 4,
                rot_keys: vec![RotKey {
                    time: 0.0,
                    rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                }],
                pos_keys: hop
                    .into_iter()
                    .map(|(time, z)| PosKey {
                        time,
                        position: Vec3::Z * z * amplitude,
                    })
                    .collect(),
            },
            JointMotion {
                joint_name: "mChest".into(),
                priority: 4,
                rot_keys: [(0.0, 0.0), (0.5, 0.15), (1.0, 0.0), (1.5, -0.15), (2.0, 0.0)]
                    .into_iter()
                    .map(|(time, angle)| RotKey {
                        time,
                        rotation: Quat::from_rotation_x(angle * amplitude),
                    })
                    .collect(),
                pos_keys: Vec::new(),
            },
        ],
    }
}

/// The balls (`crate::scene::Scene::install_demo_physics_body`): joint, ball
/// centre relative to the joint at rest, radii.
pub const BALLS: [(&str, Vec3, Vec3); 4] = [
    ("LEFT_PEC", Vec3::new(0.05, 0.01, 0.0), Vec3::splat(0.07)),
    ("RIGHT_PEC", Vec3::new(0.05, -0.01, 0.0), Vec3::splat(0.07)),
    ("BELLY", Vec3::new(0.09, 0.0, -0.03), Vec3::new(0.07, 0.1, 0.08)),
    ("BUTT", Vec3::new(-0.08, 0.0, 0.0), Vec3::new(0.1, 0.15, 0.1)),
];

/// The appearances with the physics layer, and the balls worn on the chest
/// (not in `Classic`).
pub fn events(shapes: &crate::scene::shape::ShapeParams, mode: Mode) -> Vec<NetEvent> {
    let mesh = SculptParams {
        texture: BODY_MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    };
    let mut events: Vec<NetEvent> = AVATARS
        .iter()
        .map(|&(avatar_id, _)| {
            NetEvent::Appearance(aurora_net::AvatarAppearance {
                avatar_id,
                texture_entry: None,
                visual_params: shapes.values_with(&LAYER),
                cof_version: 1,
                appearance_version: 1,
                hover_height: 0.0,
                attachments: 0,
            })
        })
        .collect();
    let objects = AVATARS
        .iter()
        .enumerate()
        .map(|(k, &(_, local))| {
            let mut worn = prim(
                BODY_LOCAL + k as u32,
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3::splat(0.1),
                aurora_prim::VolumeParams {
                    sculpt: Some(mesh),
                    ..Default::default()
                },
                te([0.95, 0.45, 0.55, 1.0], 0, false, 0.0),
                ExtraParams {
                    sculpt: Some(mesh),
                    ..Default::default()
                },
                "",
            );
            worn.parent_id = local;
            worn.state = 0x10; // chest
            worn
        })
        .collect();
    if mode != Mode::Classic {
        events.push(NetEvent::ObjectUpdates { handle: HANDLE, objects });
    }
    events
}

/// The animation lists at `frame`: the hops join the idle motion.
pub fn animation_events(frame: u64) -> Vec<NetEvent> {
    if frame != HOP_FRAME {
        return Vec::new();
    }
    log::info!("demo physics: hops start");
    AVATARS
        .iter()
        .map(|&(avatar, _)| NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar,
            anims: vec![(IDLE_ANIM, 1), (HOP_ANIM, 1)],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layer_gives_both_avatars_physics() {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let rig = crate::scene::body_physics::PhysicsRig::new(&lib.shape_params, &lib.rig).expect("physics rig");
        assert_eq!(events(&lib.shape_params, Mode::Classic).len(), 2, "no balls");
        let NetEvent::Appearance(a) = &events(&lib.shape_params, Mode::Hop)[0] else {
            panic!("appearance first");
        };
        let weights = lib.shape_params.effective_weights(&a.visual_params);
        assert!(crate::scene::body_physics::Params::from_weights(&rig, &weights, false).is_some());
        for (name, _, _) in BALLS {
            assert!(lib.rig.names.contains_key(name), "{name}");
        }
        assert_eq!(animation_events(HOP_FRAME).len(), 2);
        assert!(animation_events(HOP_FRAME - 1).is_empty());
    }
}
