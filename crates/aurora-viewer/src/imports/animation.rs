//! Independent animation preview clock. Joint palettes use the existing
//! Legacy Viewer motion blender; vertex skinning is performed by the preview GPU.

use super::*;
use crate::scene::anim::{AnimStreamer, BoundAnim, Controller, Rig};
use std::time::{Duration, Instant};

pub struct Player {
    pub bound: Option<Arc<BoundAnim>>,
    pub request: Option<Uuid>,
    pub started: bool,
    pub playing: bool,
    pub position: f32,
    pub speed: f32,
    pub base: usize,
    pub pose: Option<aurora_render::preview::Pose>,
    epoch: Instant,
    last: Instant,
    controller: Controller,
}
impl Default for Player {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            bound: None,
            request: None,
            started: false,
            playing: false,
            position: 0.0,
            speed: 1.0,
            base: 0,
            pose: None,
            epoch: now,
            last: now,
            controller: Controller::for_control_avatar(),
        }
    }
}
impl Player {
    pub fn seek(&mut self, position: f32) {
        self.position = position;
        self.epoch = Instant::now()
            .checked_sub(Duration::from_secs_f32(position.max(0.0)))
            .unwrap_or_else(Instant::now);
        self.controller = Controller::for_control_avatar();
        self.pose = None;
    }
    pub fn clock(&self) -> (Instant, Instant) {
        (self.epoch, self.epoch + Duration::from_secs_f32(self.position.max(0.0)))
    }
    pub fn tick(&mut self, id: Uuid, rig: &Rig, streamer: &mut AnimStreamer, skeleton: Option<&crate::scene::anim::SkeletonBase>) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        let Some(bound) = &self.bound else {
            return;
        };
        let duration = bound.anim.duration;
        if self.playing {
            self.position += dt * self.speed;
        }
        if !bound.anim.looping && self.position > duration {
            self.position = duration;
            self.playing = false;
        }
        if self.pose.is_some() && !self.playing {
            return;
        }
        let sample = self.epoch + Duration::from_secs_f32(self.position.max(0.0));
        self.controller.set_preview_clock(Some((None, self.epoch)));
        let base = match self.base {
            1 => Uuid::from_u128(0x6ed24bd8_91aa_4b12_ccc7_c97c857ab4e0),
            2 => Uuid::from_u128(0x1a5fe8ac_a804_8a5d_7cbd_56bd83184568),
            3 => Uuid::from_u128(0xaec4610c_757f_bc4e_c092_c6e9caf18daf),
            _ => Uuid::from_u128(0x2408fe9e_df1d_1d7d_f4ff_1384fa7b350f),
        };
        // (the expression the header names starts with it, in the controller)
        let playing = [
            crate::world::PlayingAnimation::from_signal(base, 0, self.epoch, None),
            crate::world::PlayingAnimation::from_signal(id, 1, self.epoch, None),
        ];
        self.controller.sync(&playing, sample, |motion| {
            if *motion == id { Some(bound.clone()) } else { streamer.get(motion) }
        });
        let mut pose = vec![glam::Mat4::IDENTITY.to_cols_array_2d(); rig.len() + 1];
        self.controller.evaluate(rig, sample, skeleton, &mut pose);
        self.pose = Some(pose.into());
    }
}

/// Copy editable metadata without cloning thousands of motion keys per frame.
pub fn metadata(animation: &Animation) -> Animation {
    Animation {
        base_priority: animation.base_priority,
        duration: animation.duration,
        emote_name: animation.emote_name.clone(),
        loop_in: animation.loop_in,
        loop_out: animation.loop_out,
        looping: animation.looping,
        ease_in: animation.ease_in,
        ease_out: animation.ease_out,
        hand_pose: animation.hand_pose,
        joints: Vec::new(),
    }
}

/// LLAnimStateLibrary names used by LLKeyframeMotion::setEmote.
pub fn emotes() -> [(&'static str, Uuid, &'static str); 20] {
    [
        ("", Uuid::nil(), aurora_i18n::tr!("import-emote-none")),
        (
            "express_open_mouth",
            Uuid::from_u128(0xd63bc1f9_fc81_9625_a0c6_007176d82eb7),
            aurora_i18n::tr!("import-emote-open-mouth"),
        ),
        (
            "express_afraid_emote",
            Uuid::from_u128(0xaa2df84d_cf8f_7218_527b_424a52de766e),
            aurora_i18n::tr!("import-emote-afraid"),
        ),
        (
            "express_anger_emote",
            Uuid::from_u128(0x1a03b575_9634_b62a_5767_3a679e81f4de),
            aurora_i18n::tr!("import-emote-angry"),
        ),
        (
            "express_toothsmile",
            Uuid::from_u128(0xb92ec1a5_e7ce_a76b_2b05_bcdb9311417e),
            aurora_i18n::tr!("import-emote-big-smile"),
        ),
        (
            "express_bored_emote",
            Uuid::from_u128(0x214aa6c1_ba6a_4578_f27c_ce7688f61d0d),
            aurora_i18n::tr!("import-emote-bored"),
        ),
        (
            "express_cry_emote",
            Uuid::from_u128(0xd535471b_85bf_3b4d_a542_93bea4f59d33),
            aurora_i18n::tr!("import-emote-cry"),
        ),
        (
            "express_disdain",
            Uuid::from_u128(0xd4416ff1_09d3_300f_4183_1b68a19b9fc1),
            aurora_i18n::tr!("import-emote-disdain"),
        ),
        (
            "express_embarrassed_emote",
            Uuid::from_u128(0x0b8c8211_d78c_33e8_fa28_c51a9594e424),
            aurora_i18n::tr!("import-emote-embarrassed"),
        ),
        (
            "express_frown",
            Uuid::from_u128(0xfee3df48_fa3d_1015_1e26_a205810e3001),
            aurora_i18n::tr!("import-emote-frown"),
        ),
        (
            "express_kiss",
            Uuid::from_u128(0x1e8d90cc_a84e_e135_884c_7c82c8b03a14),
            aurora_i18n::tr!("import-emote-kiss"),
        ),
        (
            "express_laugh_emote",
            Uuid::from_u128(0x62570842_0950_96f8_341c_809e65110823),
            aurora_i18n::tr!("import-emote-laugh"),
        ),
        (
            "express_tongue_out",
            Uuid::from_u128(0x835965c6_7f2f_bda2_5deb_2478737f91bf),
            aurora_i18n::tr!("import-emote-tongue"),
        ),
        (
            "express_repulsed_emote",
            Uuid::from_u128(0xf76cda94_41d4_a229_2872_e0296e58afe1),
            aurora_i18n::tr!("import-emote-repulsed"),
        ),
        (
            "express_sad_emote",
            Uuid::from_u128(0xeb6ebfb2_a4b3_a19c_d388_4dd5c03823f7),
            aurora_i18n::tr!("import-emote-sad"),
        ),
        (
            "express_shrug_emote",
            Uuid::from_u128(0xa351b1bc_cc94_aac2_7bea_a7e6ebad15ef),
            aurora_i18n::tr!("import-emote-shrug"),
        ),
        (
            "express_smile",
            Uuid::from_u128(0xb7c7c833_e3d3_c4e3_9fc0_131237446312),
            aurora_i18n::tr!("import-emote-smile"),
        ),
        (
            "express_surprise_emote",
            Uuid::from_u128(0x728646d9_cc79_08b2_32d6_937f0a835c24),
            aurora_i18n::tr!("import-emote-surprise"),
        ),
        (
            "express_wink_emote",
            Uuid::from_u128(0xda020525_4d94_59d6_23d7_81fdebf33148),
            aurora_i18n::tr!("import-emote-wink"),
        ),
        (
            "express_worry_emote",
            Uuid::from_u128(0x9c05e5c7_6f07_6ca4_ed5a_b230390c3950),
            aurora_i18n::tr!("import-emote-worry"),
        ),
    ]
}
pub fn hands() -> [&'static str; 14] {
    [
        aurora_i18n::tr!("import-hands-spread"),
        aurora_i18n::tr!("import-hands-relaxed"),
        aurora_i18n::tr!("import-hands-point"),
        aurora_i18n::tr!("import-hands-fist"),
        aurora_i18n::tr!("import-hands-relaxed-left"),
        aurora_i18n::tr!("import-hands-point-left"),
        aurora_i18n::tr!("import-hands-fist-left"),
        aurora_i18n::tr!("import-hands-relaxed-right"),
        aurora_i18n::tr!("import-hands-point-right"),
        aurora_i18n::tr!("import-hands-fist-right"),
        aurora_i18n::tr!("import-hands-salute"),
        aurora_i18n::tr!("import-hands-typing"),
        aurora_i18n::tr!("import-hands-peace"),
        aurora_i18n::tr!("import-hands-spread-right"),
    ]
}
