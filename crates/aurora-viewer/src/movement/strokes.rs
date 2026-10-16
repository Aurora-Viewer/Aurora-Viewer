//! Contributed QAvimator swimming pack retained for local/demo poses.
//! AO inventory assets, rather than this pack, provide shared swimming poses.
use super::Style;
use aurora_assets::anim::Animation;
use glam::Quat;

pub const WATER_CLIPS: [(&str, &str); 3] = [
    ("surface-idle.bvh", "Aurora Swim Surface Idle"),
    ("underwater-idle.bvh", "Aurora Swim Underwater Idle"),
    ("dive.bvh", "Aurora Swim Dive"),
];

/// QAvimator BVH reference poses and translations are converted by the same
/// loader as the normal importer. Swimming must not animate the root position:
/// the simulator, rather than the clip, owns the avatar's depth and collisions.
pub fn from_bvh(bytes: &[u8], skeleton: &aurora_assets::Skeleton) -> Result<Animation, aurora_assets::AssetError> {
    let mut animation = aurora_assets::import_animation::parse_bvh(bytes, skeleton)?.animation;
    animation.base_priority = 4;
    animation.looping = true;
    animation.loop_in = 0.0;
    animation.loop_out = animation.duration;
    animation.ease_in = 0.2;
    animation.ease_out = 0.2;
    for joint in &mut animation.joints {
        joint.priority = 4;
        if joint.joint_name == "mPelvis" {
            joint.pos_keys.clear();
        }
        if let Some(first) = joint.rot_keys.first().cloned()
            && let Some(last) = joint.rot_keys.last_mut()
        {
            last.rotation = first.rotation;
        }
    }
    Ok(animation)
}

static PACK: std::sync::OnceLock<[Animation; 6]> = std::sync::OnceLock::new();
const SOURCES: [&[u8]; 6] = [
    include_bytes!("../../assets/animations/swimming/breaststroke.bvh"),
    include_bytes!("../../assets/animations/swimming/backstroke.bvh"),
    include_bytes!("../../assets/animations/swimming/crawl.bvh"),
    include_bytes!("../../assets/animations/swimming/surface-idle.bvh"),
    include_bytes!("../../assets/animations/swimming/underwater-idle.bvh"),
    include_bytes!("../../assets/animations/swimming/dive.bvh"),
];

/// Prewarmed by App's background workers. These are embedded, validated build
/// inputs; a broken bundled asset is an internal invariant, covered by tests.
pub fn pack() -> &'static [Animation; 6] {
    PACK.get_or_init(|| {
        let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml"))
            .expect("validated bundled skeleton");
        let mut animations = SOURCES.map(|bytes| from_bvh(bytes, &skeleton).expect("validated bundled swimming BVH"));
        // The contributed backwards stroke points its head behind the avatar.
        // Turn the whole pose so backstroke travels head-first like the others.
        for joint in &mut animations[1].joints {
            if joint.joint_name == "mPelvis" {
                for key in &mut joint.rot_keys {
                    key.rotation = Quat::from_rotation_z(std::f32::consts::PI) * key.rotation;
                }
            }
        }
        animations
    })
}
pub fn ready() -> Option<&'static [Animation; 6]> {
    PACK.get()
}
pub fn animation(style: Style) -> Animation {
    pack()[style as usize].clone()
}
pub fn local_id(index: usize) -> uuid::Uuid {
    uuid::Uuid::from_u128(0xA0_5600 + index as u128)
}
