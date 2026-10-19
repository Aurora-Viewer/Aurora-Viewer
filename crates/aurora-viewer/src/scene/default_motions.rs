//! What the system avatar plays besides its animations: the facial
//! expressions and hand poses animations ask for, and the body noise and
//! breathing of every avatar.
//!
//! Port of LLEmote (indra/newview/llemote.cpp), LLHandMotion
//! (indra/llcharacter/llhandmotion.cpp), LLBodyNoiseMotion and
//! LLBreatheMotionRot (indra/newview/llvoavatar.cpp), with the triggers of
//! LLKeyframeMotion::onActivate / onUpdate
//! (indra/llcharacter/llkeyframemotion.cpp), originally LGPL 2.1.
//!
//! An expression is a motion of its own, keyed by its built-in UUID: the
//! simulator signals it (gestures) or an animation names it in its header
//! (`emote_name`), and it eases in and out like any motion while its weight
//! drives its `Express_*` morph. A hand pose is the `hand_pose` of the
//! strongest animation playing, its `Hands_*` morph blended in 0.2 s. Both
//! reach the GPU as live morph weights (live_morphs.rs).

use super::anim::{AnimStreamer, BoundAnim, Rig};
use aurora_assets::Animation;
use glam::Quat;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

/// The built-in expressions: (UUID, LLAnimationLibrary name, the visual
/// parameter of their morph in avatar_lad.xml).
const EMOTES: [(u128, &str, i32); 19] = [
    (0xaa2df84d_cf8f_7218_527b_424a52de766e, "express_afraid_emote", 310),
    (0x1a03b575_9634_b62a_5767_3a679e81f4de, "express_anger_emote", 314),
    (0x214aa6c1_ba6a_4578_f27c_ce7688f61d0d, "express_bored_emote", 307),
    (0xd535471b_85bf_3b4d_a542_93bea4f59d33, "express_cry_emote", 312),
    (0xd4416ff1_09d3_300f_4183_1b68a19b9fc1, "express_disdain", 309),
    (0x0b8c8211_d78c_33e8_fa28_c51a9594e424, "express_embarrassed_emote", 304),
    (0xfee3df48_fa3d_1015_1e26_a205810e3001, "express_frown", 315),
    (0x1e8d90cc_a84e_e135_884c_7c82c8b03a14, "express_kiss", 306),
    (0x62570842_0950_96f8_341c_809e65110823, "express_laugh_emote", 316),
    (0xd63bc1f9_fc81_9625_a0c6_007176d82eb7, "express_open_mouth", 632),
    (0xf76cda94_41d4_a229_2872_e0296e58afe1, "express_repulsed_emote", 308),
    (0xeb6ebfb2_a4b3_a19c_d388_4dd5c03823f7, "express_sad_emote", 313),
    (0xa351b1bc_cc94_aac2_7bea_a7e6ebad15ef, "express_shrug_emote", 305),
    (0xb7c7c833_e3d3_c4e3_9fc0_131237446312, "express_smile", 318),
    (0x728646d9_cc79_08b2_32d6_937f0a835c24, "express_surprise_emote", 302),
    (0x835965c6_7f2f_bda2_5deb_2478737f91bf, "express_tongue_out", 301),
    (0xb92ec1a5_e7ce_a76b_2b05_bcdb9311417e, "express_toothsmile", 317),
    (0xda020525_4d94_59d6_23d7_81fdebf33148, "express_wink_emote", 303),
    (0x9c05e5c7_6f07_6ca4_ed5a_b230390c3950, "express_worry_emote", 311),
];

/// Express_Closed_Mouth: weight 1 by default (baked with the shape), cross
/// faded against the expression playing.
pub const CLOSED_MOUTH: i32 = 300;

/// EMOTE_MORPH_FADEIN_TIME, EMOTE_MORPH_IN_TIME, EMOTE_MORPH_FADEOUT_TIME
/// (llemote.h).
const EMOTE_FADE_IN: f32 = 0.3;
const EMOTE_IN: f32 = 1.1;
const EMOTE_FADE_OUT: f32 = 1.4;

/// The morph parameter an expression motion drives.
pub fn emote_param(id: &Uuid) -> Option<i32> {
    let id = id.as_u128();
    EMOTES.iter().find(|e| e.0 == id).map(|e| e.2)
}

/// The expression an animation header names (LLKeyframeMotion::deserialize:
/// LLAnimationLibrary::stringToAnimState, lower case; "Closed_Mouth" and
/// unknown names play nothing). Only the expressions are looked up: another
/// built-in motion named there would not move a morph.
pub fn emote_named(name: &str) -> Option<Uuid> {
    let name = name.trim();
    (!name.is_empty())
        .then(|| EMOTES.iter().find(|e| e.1.eq_ignore_ascii_case(name)))
        .flatten()
        .map(|e| Uuid::from_u128(e.0))
}

/// The LLAnimationLibrary name of an expression (demo).
pub fn emote_name(id: &Uuid) -> Option<&'static str> {
    let id = id.as_u128();
    EMOTES.iter().find(|e| e.0 == id).map(|e| e.1)
}

/// Every expression parameter, Express_Closed_Mouth included.
pub fn emote_params() -> impl Iterator<Item = i32> {
    std::iter::once(CLOSED_MOUTH).chain(EMOTES.iter().map(|e| e.2))
}

/// An expression as a motion without joints: it eases in for 0.3 s, holds,
/// then stops itself and eases out (LLEmote::getDuration and its ease
/// durations, MEDIUM_PRIORITY).
fn emote_animation() -> Animation {
    Animation {
        base_priority: 1,
        duration: EMOTE_FADE_IN + EMOTE_IN + EMOTE_FADE_OUT,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: 0.0,
        looping: false,
        ease_in: EMOTE_FADE_IN,
        ease_out: EMOTE_FADE_OUT,
        hand_pose: 0,
        joints: Vec::new(),
    }
}

/// Register the expressions as built-in motions: they are never fetched as
/// assets (LLVOAvatar registers them with LLEmote::create).
pub fn register(anims: &mut AnimStreamer, rig: &Rig) {
    let anim = Arc::new(BoundAnim::bind(emote_animation(), rig));
    for e in &EMOTES {
        anims.insert(Uuid::from_u128(e.0), anim.clone());
    }
}

// ------------------------------------------------------------------- hands

/// gHandPoseNames (llhandmotion.cpp) as visual parameters: 0 is
/// HAND_POSE_SPREAD, the bare mesh.
pub const HAND_PARAMS: [i32; 14] = [0, 101, 102, 103, 666, 667, 668, 669, 670, 671, 766, 672, 791, 792];
const HAND_POSE_SPREAD: usize = 0;
const HAND_POSE_RELAXED: usize = 1;
/// Hands_Relaxed, baked into the shape of system bodies at full weight.
pub const RELAXED_HANDS: i32 = HAND_PARAMS[HAND_POSE_RELAXED];
/// HAND_MORPH_BLEND_TIME.
const HAND_MORPH_BLEND_TIME: f32 = 0.2;

/// LLHandMotion: the requested pose blends in while the current one blends
/// out; relaxed hands when no animation asks for a pose.
pub struct HandMotion {
    current: usize,
    new: usize,
    weights: [f32; HAND_PARAMS.len()],
    last: Option<Instant>,
}

impl Default for HandMotion {
    /// LLHandMotion::onActivate: relaxed.
    fn default() -> Self {
        let mut weights = [0.0; HAND_PARAMS.len()];
        weights[HAND_POSE_RELAXED] = 1.0;
        HandMotion {
            current: HAND_POSE_RELAXED,
            new: HAND_POSE_RELAXED,
            weights,
            last: None,
        }
    }
}

impl HandMotion {
    /// LLHandMotion::onUpdate with the pose the animations asked for this
    /// frame (None: none did).
    pub fn update(&mut self, requested: Option<u32>, now: Instant) {
        let dt = self.last.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        self.last = Some(now);
        let requested = match requested {
            None => Some(HAND_POSE_RELAXED),
            Some(p) if (p as usize) < HAND_PARAMS.len() => Some(p as usize),
            // ("Requested hand pose out of range. Ignoring requested pose.")
            Some(_) => None,
        };
        if let Some(pose) = requested {
            // a new pose while still blending: start the blend again from
            // the current pose at full weight
            if pose != self.new && self.new != self.current {
                if self.new != HAND_POSE_SPREAD {
                    self.weights[self.new] = 0.0;
                }
                if self.current != HAND_POSE_SPREAD {
                    self.weights[self.current] = 1.0;
                }
            }
            self.new = pose;
        }
        if self.current != self.new {
            let step = dt / HAND_MORPH_BLEND_TIME;
            let (mut incoming, mut outgoing) = (1.0, 0.0);
            if self.new != HAND_POSE_SPREAD {
                incoming = (self.weights[self.new] + step).clamp(0.0, 1.0);
                self.weights[self.new] = incoming;
            }
            if self.current != HAND_POSE_SPREAD {
                outgoing = (self.weights[self.current] - step).clamp(0.0, 1.0);
                self.weights[self.current] = outgoing;
            }
            if incoming == 1.0 && outgoing == 0.0 {
                self.current = self.new;
            }
        }
    }

    /// The hand morph weights that differ from the shape's: the relaxed
    /// hands are baked into it at full weight (RELAXED_HANDS).
    pub fn weights(&self) -> impl Iterator<Item = (i32, f32)> + '_ {
        HAND_PARAMS
            .iter()
            .zip(self.weights)
            .skip(1)
            .map(|(&p, w)| (p, if p == RELAXED_HANDS { w - 1.0 } else { w }))
            .filter(|(_, w)| *w != 0.0)
    }
}

// --------------------------------------------------- body noise, breathing

/// TORSO_NOISE_AMOUNT (degrees) and TORSO_NOISE_SPEED (llvoavatar.cpp).
const TORSO_NOISE_AMOUNT: f32 = 1.0;
const TORSO_NOISE_SPEED: f32 = 0.2;
/// BREATHE_ROT_MOTION_STRENGTH (radians at the top of a breath).
const BREATHE_ROT_MOTION_STRENGTH: f32 = 0.05;

/// The animations that stop the body noise while they are signalled
/// (AGENT_GUN_AIM_ANIMS, llvoavatar.cpp).
pub const GUN_AIM_ANIMS: [Uuid; 4] = [
    Uuid::from_u128(0xea633413_8006_180a_c3ba_96dd1d756720),
    Uuid::from_u128(0x3147d815_6338_b932_f011_16b56d9ac18b),
    Uuid::from_u128(0xb5b4a67d_0aee_30d2_72cd_77b333e932ef),
    Uuid::from_u128(0x46bb4359_de38_4ed8_6a22_f1f52fe8f506),
];

/// LLBodyNoiseMotion::onUpdate: the torso sways by about a degree, `t`
/// seconds after the motion started (an additive rotation of mTorso).
pub fn body_noise(t: f32) -> Quat {
    let noise_x = aurora_prim::noise::noise2(t * TORSO_NOISE_SPEED, 0.0);
    let noise_y = aurora_prim::noise::noise2(0.0, t * TORSO_NOISE_SPEED);
    let rx = TORSO_NOISE_AMOUNT.to_radians() * noise_x / 0.42;
    let ry = TORSO_NOISE_AMOUNT.to_radians() * noise_y / 0.42;
    // LLQuaternion::setQuat(roll, pitch, 0)
    let (sx, cx) = (rx * 0.5).sin_cos();
    let (sy, cy) = (ry * 0.5).sin_cos();
    Quat::from_xyzw(sx * cy, cx * sy, sx * sy, cx * cy)
}

/// LLBreatheMotionRot::onUpdate: the chest turns about its Y axis, one
/// breath every 2π seconds (mBreatheRate 1).
pub fn breathe(t: f32) -> Quat {
    Quat::from_axis_angle(glam::Vec3::Y, t.sin() * BREATHE_ROT_MOTION_STRENGTH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::time::Duration;

    #[test]
    fn every_expression_and_hand_pose_has_its_morph_on_the_system_body() {
        let lib = AvatarLibrary::load();
        let part = |name: &str| lib.parts.iter().find(|p| p.name == name).expect("body part");
        let has = |name: &str, param: i32| part(name).morphs.iter().any(|m| m.param == param);
        for param in emote_params() {
            assert!(has("head", param), "head {param}");
        }
        for param in &HAND_PARAMS[1..] {
            assert!(has("upper", *param), "upper {param}");
        }
        // the names an animation header carries
        let lad = AvatarLibrary::file("avatar_lad.xml").expect("avatar_lad.xml");
        let params = super::super::morph::mesh_params(lad);
        for (_, name, param) in &EMOTES {
            let morph = &params["avatar_head.llm"].iter().find(|p| p.0 == *param).expect("express param").1;
            assert!(morph.eq_ignore_ascii_case(name), "{name} {morph}");
        }
    }

    #[test]
    fn an_animation_header_names_its_expression_in_any_case() {
        assert_eq!(emote_named("Express_Smile"), Some(Uuid::from_u128(EMOTES[13].0)));
        assert_eq!(emote_named("express_toothsmile").and_then(|id| emote_param(&id)), Some(317));
        assert_eq!(emote_named(""), None);
        assert_eq!(emote_named("Closed_Mouth"), None);
        assert_eq!(emote_named("stand"), None);
        assert!((emote_animation().duration - 2.8).abs() < 1e-6);
    }

    #[test]
    fn hands_relax_by_default_and_blend_to_the_requested_pose_in_a_fifth_of_a_second() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut hands = HandMotion::default();
        hands.update(None, at(0));
        // (relaxed is baked into the shape: nothing differs from it)
        assert_eq!(hands.weights().count(), 0);
        // a fist (3): halfway after 0.1 s, there at 0.2 s
        hands.update(Some(3), at(0));
        hands.update(Some(3), at(100));
        let w: Vec<_> = hands.weights().collect();
        assert!(w.iter().any(|&(p, w)| p == 103 && (w - 0.5).abs() < 1e-3), "{w:?}");
        assert!(w.iter().any(|&(p, w)| p == 101 && (w + 0.5).abs() < 1e-3), "{w:?}");
        hands.update(Some(3), at(200));
        assert_eq!(hands.weights().collect::<Vec<_>>(), vec![(101, -1.0), (103, 1.0)]);
        // spread (0) is the bare mesh: the fist blends out alone
        hands.update(Some(0), at(300));
        hands.update(Some(0), at(400));
        assert_eq!(hands.weights().collect::<Vec<_>>(), vec![(101, -1.0)]);
        // an out of range pose is ignored; back to relaxed without animation
        hands.update(Some(40), at(500));
        assert_eq!(hands.weights().collect::<Vec<_>>(), vec![(101, -1.0)]);
        hands.update(None, at(600));
        hands.update(None, at(800));
        assert_eq!(hands.weights().count(), 0);
    }

    #[test]
    fn the_body_sways_by_about_a_degree_and_the_chest_breathes() {
        assert_eq!(body_noise(0.0), Quat::IDENTITY);
        let mut largest: f32 = 0.0;
        for i in 0..2000 {
            let q = body_noise(i as f32 * 0.05);
            assert!((q.length() - 1.0).abs() < 1e-4);
            largest = largest.max(q.angle_between(Quat::IDENTITY));
        }
        assert!(
            largest > 0.2f32.to_radians() && largest < 3f32.to_radians(),
            "{}",
            largest.to_degrees()
        );
        let top = breathe(std::f32::consts::FRAC_PI_2);
        assert!((top.angle_between(Quat::IDENTITY) - 0.05).abs() < 1e-4);
        assert!((top * glam::Vec3::Y - glam::Vec3::Y).length() < 1e-5);
        assert!(breathe(std::f32::consts::TAU).angle_between(Quat::IDENTITY) < 1e-4);
    }
}
