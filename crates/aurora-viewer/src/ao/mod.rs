//! Viewer-side Animation Overrider. Behavior and inventory format follow
//! AOEngine / AOSet (Firestorm indra/newview/aoengine.cpp, aoset.cpp,
//! originally LGPL 2.1). Inventory links keep the original animations intact.

pub mod anchor;
mod anchor_blend;
pub mod bootstrap;
mod completion;
pub mod demo;
pub mod inventory;
pub mod migration;
pub mod runtime;
#[cfg(test)]
mod swimming_tests;
#[cfg(test)]
mod tests;
pub mod water_demo;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// UseAO is per-account and off initially; pause and stands are session-only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub accounts: HashMap<Uuid, Account>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub enabled: bool,
    pub selected: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum State {
    #[default]
    Standing,
    Walking,
    Running,
    Sitting,
    GroundSit,
    Crouching,
    CrouchWalk,
    Landing,
    SoftLanding,
    StandingUp,
    Falling,
    FlyingDown,
    FlyingUp,
    Flying,
    FlyingSlow,
    Hovering,
    Jumping,
    PreJumping,
    TurningRight,
    TurningLeft,
    Typing,
    Floating,
    SwimmingForward,
    UnderwaterSwimming,
    SwimmingUp,
    SwimmingDown,
    UnderwaterFloating,
    WaterFlightExit,
    WaterEntry,
    WaterShoreExit,
    WaterDiveJump,
    WaterDiveEntry,
    WaterCannonball,
    Always,
    LegacySwimmingCrawl,
    LegacySwimmingBackward,
}

impl State {
    pub const ALL: [Self; 34] = [
        Self::Standing,
        Self::Walking,
        Self::Running,
        Self::Sitting,
        Self::GroundSit,
        Self::Crouching,
        Self::CrouchWalk,
        Self::Landing,
        Self::SoftLanding,
        Self::StandingUp,
        Self::Falling,
        Self::FlyingDown,
        Self::FlyingUp,
        Self::Flying,
        Self::FlyingSlow,
        Self::Hovering,
        Self::Jumping,
        Self::PreJumping,
        Self::TurningRight,
        Self::TurningLeft,
        Self::Typing,
        Self::Floating,
        Self::SwimmingForward,
        Self::UnderwaterSwimming,
        Self::SwimmingUp,
        Self::SwimmingDown,
        Self::UnderwaterFloating,
        Self::WaterFlightExit,
        Self::WaterEntry,
        Self::WaterShoreExit,
        Self::WaterDiveJump,
        Self::WaterDiveEntry,
        Self::WaterCannonball,
        Self::Always,
    ];

    pub fn name(self) -> &'static str {
        [
            "Standing",
            "Walking",
            "Running",
            "Sitting",
            "Sitting On Ground",
            "Crouching",
            "Crouch Walking",
            "Landing",
            "Soft Landing",
            "Standing Up",
            "Falling",
            "Flying Down",
            "Flying Up",
            "Flying",
            "Flying Slow",
            "Hovering",
            "Jumping",
            "Pre Jumping",
            "Turning Right",
            "Turning Left",
            "Typing",
            "Floating",
            "Swimming Forward",
            "Underwater Swimming",
            "Swimming Up",
            "Swimming Down",
            "Underwater Floating",
            "Water Flight Exit",
            "Water Entry",
            "Water Shore Exit",
            "Water Dive Jump",
            "Water Dive Entry",
            "Water Cannonball",
            "Always",
            "Swimming Crawl",
            "Swimming Backward",
        ][self as usize]
    }

    pub fn label(self) -> &'static str {
        aurora_i18n::text(
            [
                "ao-standing",
                "ao-walking",
                "ao-running",
                "ao-sitting",
                "ao-ground-sit",
                "ao-crouching",
                "ao-crouch-walk",
                "ao-landing",
                "ao-soft-landing",
                "ao-standing-up",
                "ao-falling",
                "ao-flying-down",
                "ao-flying-up",
                "ao-flying",
                "ao-flying-slow",
                "ao-hovering",
                "ao-jumping",
                "ao-pre-jumping",
                "ao-turning-right",
                "ao-turning-left",
                "ao-typing",
                "ao-floating",
                "ao-swimming-forward",
                "ao-underwater-swimming",
                "ao-swimming-up",
                "ao-swimming-down",
                "ao-underwater-floating",
                "ao-water-flight-exit",
                "ao-water-entry",
                "ao-water-shore-exit",
                "ao-water-dive-jump",
                "ao-water-dive-entry",
                "ao-water-cannonball",
                "ao-always",
                "ao-swimming-crawl",
                "ao-swimming-backward",
            ][self as usize],
        )
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain(Self::LEGACY_SWIMMING)
            .find(|s| s.name() == name)
            .or(match name {
                "Stand.1" | "Stand.2" | "Stand.3" => Some(Self::Standing),
                "Walk.N" => Some(Self::Walking),
                "Sit.N" => Some(Self::Sitting),
                "Sit.G" => Some(Self::GroundSit),
                "Crouch" => Some(Self::Crouching),
                "Walk.C" => Some(Self::CrouchWalk),
                "Land.N" => Some(Self::Landing),
                "Stand.U" => Some(Self::StandingUp),
                "Hover.D" => Some(Self::FlyingDown),
                "Hover.U" => Some(Self::FlyingUp),
                "Fly.N" => Some(Self::Flying),
                "Hover.N" => Some(Self::Hovering),
                "Jump.N" => Some(Self::Jumping),
                "Jump.P" => Some(Self::PreJumping),
                "Turn.R" => Some(Self::TurningRight),
                "Turn.L" => Some(Self::TurningLeft),
                "Swim.H" => Some(Self::Floating),
                "Swim.N" => Some(Self::SwimmingForward),
                "Swim.U" => Some(Self::SwimmingUp),
                "Swim.D" => Some(Self::SwimmingDown),
                _ => None,
            })
    }

    pub fn motion(self) -> Uuid {
        Uuid::from_u128(match self {
            Self::Standing => 0x2408fe9e_df1d_1d7d_f4ff_1384fa7b350f,
            Self::Walking => 0x6ed24bd8_91aa_4b12_ccc7_c97c857ab4e0,
            Self::Running => 0x05ddbff8_aaa9_92a1_2b74_8fe77a29b445,
            Self::Sitting => 0x1a5fe8ac_a804_8a5d_7cbd_56bd83184568,
            Self::GroundSit => 0x1a2bd58e_87ff_0df8_0b4c_53e047b0bb6e,
            Self::Crouching => 0x201f3fdf_cb1f_dbec_201f_7333e328ae7c,
            Self::CrouchWalk => 0x47f5f6fb_22e5_ae44_f871_73aaaf4a6022,
            Self::Landing => 0x7a17b059_12b2_41b1_570a_186368b6aa6f,
            Self::SoftLanding => 0xf4f00d6e_b9fe_9292_f4cb_0ae06ea58d57,
            Self::StandingUp => 0x3da1d753_028a_5446_24f3_9c9b856d9422,
            Self::Falling | Self::WaterEntry | Self::WaterDiveEntry => 0x666307d9_a860_572d_6fd4_c3ab8865c094,
            Self::FlyingDown | Self::SwimmingDown => 0x20f063ea_8306_2562_0b07_5c853b37b31e,
            Self::FlyingUp | Self::SwimmingUp => 0x62c5de58_cb33_5743_3d07_9e4cd4352864,
            Self::Flying | Self::SwimmingForward | Self::UnderwaterSwimming | Self::LegacySwimmingCrawl | Self::LegacySwimmingBackward => {
                0xaec4610c_757f_bc4e_c092_c6e9caf18daf
            }
            Self::FlyingSlow => 0x2b5a38b2_5e00_3a97_a495_4c826bc443e6,
            Self::Hovering | Self::Floating | Self::UnderwaterFloating => 0x4ae8016b_31b9_03bb_c401_b1ea941db41d,
            Self::Jumping | Self::WaterFlightExit | Self::WaterShoreExit | Self::WaterDiveJump | Self::WaterCannonball => {
                0x2305bd75_1ca9_b03b_1faa_b176b8a8c49e
            }
            Self::PreJumping => 0x7a4e87fe_de39_6fcb_6223_024b00893244,
            Self::TurningRight => 0x2d6daa51_3192_6794_8e2e_a15f8338ec30,
            Self::TurningLeft => 0x56e0ba0d_4a9f_7f27_6117_32f2ebbf6135,
            Self::Typing => 0xc541c47f_e0c0_058b_ad1a_d6ae3a4584d9,
            Self::Always => 0,
        })
    }

    pub fn from_motion(id: Uuid) -> Option<Self> {
        if id == SIT_FEMALE {
            return Some(Self::Sitting);
        }
        if id == Uuid::from_u128(0x1c7600d6_661f_b87b_efe2_d7421eb93c86) {
            return Some(Self::GroundSit);
        }
        if STAND_VARIANTS.contains(&id) {
            return Some(Self::Standing);
        }
        Self::ALL[..21].iter().copied().find(|s| s.motion() == id)
    }

    pub fn transient(self) -> bool {
        matches!(
            self,
            Self::GroundSit | Self::Landing | Self::SoftLanding | Self::StandingUp | Self::PreJumping
        )
    }

    pub const CORE_SWIMMING: [Self; 8] = [
        Self::Floating,
        Self::SwimmingForward,
        Self::SwimmingDown,
        Self::SwimmingUp,
        Self::UnderwaterFloating,
        Self::WaterFlightExit,
        Self::WaterEntry,
        Self::WaterShoreExit,
    ];

    /// AOSet::stateNames / AOEngine::mapSwimming (indra/newview/aoset.cpp,
    /// aoengine.cpp): these are the Legacy Viewer's four inventory swim slots.
    pub const LEGACY_VIEWER_SWIMMING: [Self; 4] = [Self::Floating, Self::SwimmingForward, Self::SwimmingUp, Self::SwimmingDown];

    /// Extra clips are optional for existing eight-slot sets.
    pub const SWIMMING: [Self; 12] = [
        Self::Floating,
        Self::SwimmingForward,
        Self::UnderwaterSwimming,
        Self::UnderwaterFloating,
        Self::SwimmingDown,
        Self::SwimmingUp,
        Self::WaterEntry,
        Self::WaterFlightExit,
        Self::WaterShoreExit,
        Self::WaterDiveJump,
        Self::WaterDiveEntry,
        Self::WaterCannonball,
    ];

    pub const LEGACY_SWIMMING: [Self; 2] = [Self::LegacySwimmingCrawl, Self::LegacySwimmingBackward];

    pub fn swimming(self) -> bool {
        Self::SWIMMING.contains(&self) || Self::LEGACY_SWIMMING.contains(&self)
    }
}

pub const STAND_VARIANTS: [Uuid; 4] = [
    Uuid::from_u128(0x15468e00_3400_bb66_cecc_646d7c14458e),
    Uuid::from_u128(0x370f3a20_6ca6_9971_848c_9a01bc42ae3c),
    Uuid::from_u128(0x42b46214_4b44_79ae_deb8_0df61424ff4b),
    Uuid::from_u128(0xf22fed8b_a5ed_2c93_64d5_bdd8b93c889f),
];
pub const SIT_GENERIC: Uuid = Uuid::from_u128(0x245f3c54_f1c0_bf2e_811f_46d8eeb386e7);
pub const SIT_FEMALE: Uuid = Uuid::from_u128(0xb1709c8d_ecd3_54a1_4f28_d55ac0840782);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Animation {
    pub link: Uuid,
    pub original: Uuid,
    pub asset: Uuid,
    pub name: String,
    pub order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub id: Uuid,
    pub name: String,
    pub order: i32,
    pub group: bool,
    pub members: Vec<Animation>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cycle {
    pub cycle: bool,
    pub random: bool,
    pub seconds: f32,
}

impl Cycle {
    pub fn parse(params: &[&str]) -> Self {
        let seconds = params
            .iter()
            .find_map(|p| p.strip_prefix("CT").and_then(|v| v.parse::<f32>().ok()))
            .unwrap_or(0.0);
        Self {
            cycle: params.contains(&"CY"),
            random: params.contains(&"RN"),
            seconds: if seconds.is_finite() { seconds.clamp(0.0, 86400.0) } else { 0.0 },
        }
    }
    pub fn suffix(&self) -> String {
        let mut s = String::new();
        if self.seconds.is_finite() && self.seconds > 0.0 {
            s.push_str(&format!(":CT{:.2}", self.seconds.clamp(0.0, 86400.0)));
        }
        if self.cycle {
            s.push_str(":CY");
        }
        if self.random {
            s.push_str(":RN");
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: Uuid,
    pub name: String,
    pub cycle: Cycle,
    pub members: Vec<Animation>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StateConfig {
    pub folder: Uuid,
    pub cycle: Cycle,
    pub steps: Vec<Step>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Set {
    pub id: Uuid,
    pub name: String,
    pub sits: bool,
    pub smart: bool,
    pub mouselook: bool,
    pub default: bool,
    pub states: HashMap<State, StateConfig>,
}

impl Set {
    /// A representative explicit clip for movement timing/readiness. Water
    /// states use the same lists, groups and parallel tracks as ordinary AO.
    pub fn swimming_animation(&self, state: State) -> Option<&Animation> {
        let config = self.states.get(&state).filter(|_| state.swimming())?;
        config
            .steps
            .iter()
            .flat_map(|s| &s.members)
            .chain(config.tracks.iter().flat_map(|t| &t.members))
            .find(|a| !a.asset.is_nil())
    }

    /// Old sets keep their existing surface clip until underwater swimming is
    /// assigned; never manufacture an uploaded asset or a bundled fallback.
    pub fn animation_config(&self, state: State) -> Option<&StateConfig> {
        let state = if state == State::UnderwaterSwimming && self.swimming_animation(state).is_none() {
            State::SwimmingForward
        } else {
            state
        };
        self.states.get(&state)
    }

    #[cfg(test)]
    pub fn swimming_slots(&self) -> usize {
        State::SWIMMING
            .into_iter()
            .filter(|s| self.swimming_animation(*s).is_some())
            .count()
    }

    pub fn core_swimming_slots(&self) -> usize {
        State::CORE_SWIMMING
            .into_iter()
            .filter(|s| self.swimming_animation(*s).is_some())
            .count()
    }

    pub fn swimming_ready(&self) -> bool {
        self.core_swimming_slots() == State::CORE_SWIMMING.len()
    }

    pub fn legacy_swimming_pending(&self) -> bool {
        State::LEGACY_SWIMMING.iter().any(|state| self.swimming_animation(*state).is_some())
    }

    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            sits: false,
            smart: false,
            mouselook: false,
            default: false,
            states: HashMap::new(),
        }
    }
    pub fn folder_name(&self) -> String {
        format!(
            "{}{}{}{}{}",
            self.name,
            if self.sits { ":SO" } else { "" },
            if self.smart { ":SM" } else { "" },
            if self.mouselook { ":DM" } else { "" },
            if self.default { ":**" } else { "" }
        )
    }
}
