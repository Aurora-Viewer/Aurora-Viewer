//! A deformed avatar to reset (AURORA_DEMO_DEFORM, docs/TESTING.md, #272):
//! a short pose that moves joints plays once on the avatar in front of the
//! start position, then stops. As in Second Life, the joints keep the
//! positions it gave them: the neck stays stretched and the shoulders wide
//! until "Reset skeleton". `reset` / `reset-anim` choose that entry of the
//! avatar's menu at frame 700; `self` deforms our own avatar instead (and
//! `self-reset` resets it the same way, ViewerEffect included).

use super::{DEMO_AGENT, DEMO_LOUP, IDLE_ANIM};
use aurora_assets::Animation;
use aurora_assets::anim::{JointMotion, PosKey};
use aurora_net::NetEvent;
use glam::Vec3;
use uuid::Uuid;

/// The pose that deforms.
pub const DEFORM_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0272);
/// Frame the pose starts at, once the avatars are there.
const DEFORM_FRAME: u64 = 240;
/// Frame the menu entry is chosen at.
const RESET_FRAME: u64 = 700;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    /// Our avatar instead of the other one.
    pub own: bool,
    /// The reset at RESET_FRAME: None, or with the animations or not.
    pub reset: Option<bool>,
}

pub fn mode() -> Option<Mode> {
    static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        let value = std::env::var("AURORA_DEMO_DEFORM").ok()?;
        let (own, reset) = match value.trim() {
            "reset" => (false, Some(false)),
            "reset-anim" => (false, Some(true)),
            "self" => (true, None),
            "self-reset" => (true, Some(false)),
            _ => (false, None),
        };
        Some(Mode { own, reset })
    })
}

impl Mode {
    pub fn avatar(self) -> Uuid {
        if self.own { DEMO_AGENT } else { DEMO_LOUP }
    }
}

/// A one-second pose, not looping, that moves the neck up and the
/// shoulders out (positions in the parent joint's frame, like any
/// animation's position keys).
pub fn animation(rig: &crate::scene::anim::Rig) -> Animation {
    let moved = |joint: &str, by: Vec3| JointMotion {
        joint_name: joint.into(),
        priority: 4,
        rot_keys: Vec::new(),
        pos_keys: vec![PosKey {
            time: 0.0,
            position: rig.local_pos[rig.names[joint]] + by,
        }],
    };
    Animation {
        base_priority: 4,
        duration: 1.0,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: 1.0,
        looping: false,
        ease_in: 0.2,
        ease_out: 0.2,
        hand_pose: 0,
        joints: vec![
            moved("mNeck", Vec3::Z * 0.3),
            moved("mShoulderLeft", Vec3::Y * 0.15),
            moved("mShoulderRight", -Vec3::Y * 0.15),
        ],
    }
}

/// The animation list of the deformed avatar at `frame`: the pose starts
/// with the idle one, and stays listed (a one-shot motion stops by itself).
pub fn events(frame: u64, mode: Mode) -> Vec<NetEvent> {
    if frame != DEFORM_FRAME {
        return Vec::new();
    }
    log::info!("demo deform: pose on {}", mode.avatar());
    vec![NetEvent::AvatarAnimations {
        sources: Vec::new(),
        avatar: mode.avatar(),
        anims: vec![(IDLE_ANIM, 1), (DEFORM_ANIM, 1)],
    }]
}

/// The menu entry chosen at `frame`: (avatar, with the animations).
pub fn reset_at(frame: u64, mode: Mode) -> Option<(Uuid, bool)> {
    (frame == RESET_FRAME).then_some(())?;
    mode.reset.map(|animations| (mode.avatar(), animations))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pose_starts_once_and_the_reset_comes_later() {
        let other = Mode {
            own: false,
            reset: Some(true),
        };
        assert!(events(DEFORM_FRAME - 1, other).is_empty());
        assert_eq!(events(DEFORM_FRAME, other).len(), 1);
        assert_eq!(reset_at(RESET_FRAME, other), Some((DEMO_LOUP, true)));
        assert_eq!(reset_at(RESET_FRAME + 1, other), None);
        let own = Mode { own: true, reset: None };
        assert_eq!(reset_at(RESET_FRAME, own), None);
        assert_eq!(own.avatar(), DEMO_AGENT);
    }
}
