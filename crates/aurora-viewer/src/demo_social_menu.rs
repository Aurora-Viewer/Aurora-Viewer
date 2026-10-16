//! AURORA_DEMO_SOCIAL_MENU: a resident's "Invite to group", "Add to
//! contact set" and "Block particles" (app/social_menu_demo.rs runs the
//! steps). The violet wolf stands in for the resident: not blocked in this
//! scenario, owner of the plaza's fountain, and our abilities in "Aurora
//! Builders" include inviting (an officer's).
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The wolf's world menu, from its name plate.
    Menu,
    /// "Invite to group": the group picker.
    Picker,
    /// The picker answered: the invitation into "Aurora Builders".
    Invite,
    /// "New contact set…": its name asked.
    NewSet,
    /// The name given: People › Contact sets on the new set.
    NewSetDone,
    /// "Block particles": the fountain's particles go, then the menu shows
    /// the entry checked.
    Particles,
}

pub fn step() -> Option<Step> {
    Some(match std::env::var("AURORA_DEMO_SOCIAL_MENU").ok()?.trim() {
        "1" | "menu" => Step::Menu,
        "picker" => Step::Picker,
        "invite" => Step::Invite,
        "new-set" => Step::NewSet,
        "new-set-done" => Step::NewSetDone,
        "particles" => Step::Particles,
        _ => return None,
    })
}

/// The resident of the scenario.
pub const RESIDENT: Uuid = super::DEMO_LOUP;
