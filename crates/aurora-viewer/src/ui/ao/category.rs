//! Presentation groups for the existing AO states; animation ownership is unchanged.
use crate::ao::State;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Category {
    General,
    Ground,
    Jumping,
    Flight,
    Water,
}

impl Category {
    pub const ALL: [Self; 5] = [Self::General, Self::Ground, Self::Jumping, Self::Flight, Self::Water];

    pub fn of(state: State) -> Self {
        match state {
            State::Standing | State::Sitting | State::GroundSit | State::StandingUp | State::Typing | State::Always => Self::General,
            State::Landing | State::SoftLanding | State::Falling | State::Jumping | State::PreJumping => Self::Jumping,
            State::FlyingDown | State::FlyingUp | State::Flying | State::FlyingSlow | State::Hovering => Self::Flight,
            State::Floating
            | State::SwimmingForward
            | State::UnderwaterSwimming
            | State::SwimmingUp
            | State::SwimmingDown
            | State::UnderwaterFloating
            | State::WaterFlightExit
            | State::WaterEntry
            | State::WaterShoreExit
            | State::WaterDiveJump
            | State::WaterDiveEntry
            | State::WaterCannonball
            | State::LegacySwimmingCrawl
            | State::LegacySwimmingBackward => Self::Water,
            State::Walking | State::Running | State::Crouching | State::CrouchWalk | State::TurningRight | State::TurningLeft => {
                Self::Ground
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::General => aurora_i18n::tr!("ao-category-general"),
            Self::Ground => aurora_i18n::tr!("ao-category-ground"),
            Self::Jumping => aurora_i18n::tr!("ao-category-jumping"),
            Self::Flight => aurora_i18n::tr!("ao-category-flight"),
            Self::Water => aurora_i18n::tr!("ao-category-water"),
        }
    }

    pub fn states(self) -> impl Iterator<Item = State> + Clone {
        const GROUND_STATES: &[State] = &[
            State::Walking,
            State::TurningRight,
            State::TurningLeft,
            State::Running,
            State::Crouching,
            State::CrouchWalk,
        ];
        const JUMP_STATES: &[State] = &[
            State::PreJumping,
            State::Jumping,
            State::Falling,
            State::SoftLanding,
            State::Landing,
        ];
        const FLIGHT_STATES: &[State] = &[
            State::Hovering,
            State::FlyingSlow,
            State::Flying,
            State::FlyingDown,
            State::FlyingUp,
        ];
        const WATER_STATES: &[State] = &[
            State::Floating,
            State::SwimmingForward,
            State::UnderwaterFloating,
            State::UnderwaterSwimming,
            State::SwimmingUp,
            State::SwimmingDown,
            State::WaterFlightExit,
            State::WaterShoreExit,
            State::WaterDiveJump,
            State::WaterDiveEntry,
            State::WaterCannonball,
            State::WaterEntry,
        ];
        let states = match self {
            Self::Ground => GROUND_STATES,
            Self::Jumping => JUMP_STATES,
            Self::Flight => FLIGHT_STATES,
            Self::Water => WATER_STATES,
            _ => &State::ALL,
        };
        states.iter().copied().filter(move |state| Self::of(*state) == self)
    }

    pub fn initial_state(self) -> State {
        if self == Self::Jumping {
            State::Jumping
        } else {
            self.states().next().unwrap_or(State::Standing)
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}
