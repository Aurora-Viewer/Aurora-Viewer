//! A facial expression and a hand pose on the avatar in front of the start
//! position (AURORA_DEMO_EMOTE, docs/TESTING.md, #661). From frame 240 it
//! plays a looping animation whose header names an expression and a hand
//! pose, as uploaded animations do; the animation
//! restarts every 240 frames, which starts its expression again (2.8 s
//! each time). `signal` signals the expression's built-in UUID next to the
//! animation instead, as a gesture does; `self` plays it on our own avatar
//! (framed from the front by AURORA_DEMO_SHAPE=mouth). Combine with
//! AURORA_DEMO_CLASSIC=1: the morphs move the system body only.

use super::{DEMO_AGENT, DEMO_LOUP, IDLE_ANIM};
use aurora_assets::Animation;
use aurora_net::NetEvent;
use uuid::Uuid;

/// The animation that carries the expression and the hand pose.
pub const EMOTE_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0661);
/// First frame, once the avatars are there, then the period of a restart.
const START_FRAME: u64 = 240;
const PERIOD: u64 = 240;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mode {
    /// The expression's built-in UUID.
    pub emote: Uuid,
    /// gHandPoseNames index (0 spread … 13 Hands_Spread_R).
    pub hand: u32,
    /// Signalled by the simulator instead of named by the animation.
    pub signal: bool,
    /// Our own avatar instead of the one in front.
    pub own: bool,
}

impl Mode {
    pub fn avatar(&self) -> Uuid {
        if self.own { DEMO_AGENT } else { DEMO_LOUP }
    }
}

/// `<expression>[,<hand pose>][,signal][,self]`: the expression without its
/// `express_` prefix (`smile`, `toothsmile`, `wink_emote`… `_emote` may be
/// left out), the hand pose as its index (3, a fist, by default).
pub fn mode() -> Option<&'static Mode> {
    static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| parse(&std::env::var("AURORA_DEMO_EMOTE").ok()?)).as_ref()
}

fn parse(value: &str) -> Option<Mode> {
    let mut parts = value.split(',').map(str::trim);
    let name = parts.next().filter(|n| !n.is_empty()).unwrap_or("toothsmile");
    let emote = crate::scene::default_motions::emote_named(&format!("express_{name}"))
        .or_else(|| crate::scene::default_motions::emote_named(&format!("express_{name}_emote")))?;
    let mut mode = Mode {
        emote,
        hand: 3,
        signal: false,
        own: false,
    };
    for p in parts {
        match p {
            "signal" => mode.signal = true,
            "self" => mode.own = true,
            p => mode.hand = p.parse().ok().filter(|h| *h < 14)?,
        }
    }
    Some(mode)
}

/// The looping animation: priority 4 (above the demo idle), the hand pose,
/// and the expression unless the simulator signals it. It holds mFaceRoot at
/// rest, which moves nothing on the system body: without a joint the motion
/// would not be updated once eased in, and would leave the hands
/// (LLMotionController::updateMotionsByType).
pub fn animation(mode: &Mode) -> Animation {
    let emote_name = if mode.signal {
        String::new()
    } else {
        crate::scene::default_motions::emote_name(&mode.emote)
            .unwrap_or_default()
            .to_owned()
    };
    Animation {
        base_priority: 4,
        duration: 1.0,
        emote_name,
        loop_in: 0.0,
        loop_out: 1.0,
        looping: true,
        ease_in: 0.2,
        // (still easing out while it restarts: the hands keep their pose)
        ease_out: 0.5,
        hand_pose: mode.hand,
        joints: vec![aurora_assets::JointMotion {
            joint_name: "mFaceRoot".into(),
            priority: -1,
            rot_keys: vec![aurora_assets::anim::RotKey {
                time: 0.0,
                rotation: glam::Quat::IDENTITY,
            }],
            pos_keys: Vec::new(),
        }],
    }
}

/// The animation list of the avatar at `frame`: off for one frame before
/// each restart, so that the loop starts over.
pub fn events(frame: u64, mode: &Mode) -> Vec<NetEvent> {
    if frame + 1 < START_FRAME || (frame + 1 - START_FRAME) % PERIOD > 1 {
        return Vec::new();
    }
    let round = ((frame + 1 - START_FRAME) / PERIOD + 1) as i32;
    let on = (frame + 1 - START_FRAME) % PERIOD == 1;
    let mut anims = vec![(IDLE_ANIM, 1)];
    if on {
        anims.push((EMOTE_ANIM, round));
        if mode.signal {
            anims.push((mode.emote, round));
        }
        log::info!("demo emote: round {round} on {}", mode.avatar());
    }
    vec![NetEvent::AvatarAnimations {
        sources: Vec::new(),
        avatar: mode.avatar(),
        anims,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scenario_reads_its_expression_and_hand_pose() {
        let smile = parse("smile").expect("smile");
        assert_eq!(crate::scene::default_motions::emote_param(&smile.emote), Some(318));
        assert_eq!((smile.hand, smile.signal), (3, false));
        let wink = parse("wink,9,signal,self").expect("wink");
        assert_eq!(crate::scene::default_motions::emote_param(&wink.emote), Some(303));
        assert_eq!((wink.hand, wink.signal, wink.avatar()), (9, true, DEMO_AGENT));
        assert!(parse("nothing").is_none());
        assert!(parse("smile,99").is_none());
        assert_eq!(animation(&smile).emote_name, "express_smile");
        assert!(animation(&wink).emote_name.is_empty());
    }

    #[test]
    fn the_animation_restarts_with_a_frame_off() {
        let mode = parse("smile").expect("smile");
        let listed = |frame: u64| -> Option<Vec<(Uuid, i32)>> {
            match events(frame, &mode).pop()? {
                NetEvent::AvatarAnimations { anims, .. } => Some(anims),
                _ => None,
            }
        };
        assert!(listed(100).is_none());
        assert_eq!(listed(START_FRAME - 1), Some(vec![(IDLE_ANIM, 1)]));
        assert_eq!(listed(START_FRAME), Some(vec![(IDLE_ANIM, 1), (EMOTE_ANIM, 1)]));
        assert!(listed(START_FRAME + 1).is_none());
        assert_eq!(listed(START_FRAME + PERIOD), Some(vec![(IDLE_ANIM, 1), (EMOTE_ANIM, 2)]));
    }
}
