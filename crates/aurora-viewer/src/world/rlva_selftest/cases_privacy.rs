//! Phase 1 cases of the names and location privacy parity (`@shownames`,
//! `@showloc`, `@showminimap`, `@showworldmap`, `@shownearby`,
//! `@shownametags`, `@showhovertext*`): rules set through the controller,
//! then what the viewer shows or lets out asked of the world, nothing
//! restricted sent.
use super::{Case, Step};

mod location;
mod map;
mod names;

/// A phase 1 case.
pub(super) fn case(id: &'static str, title: &'static str, steps: Vec<Step>) -> Case {
    Case {
        id,
        title,
        phase: 1,
        gate: false,
        known: None,
        steps,
    }
}

pub(super) fn phase1() -> Vec<Case> {
    let mut cases = names::cases();
    cases.extend(location::cases());
    cases.extend(map::cases());
    cases
}
