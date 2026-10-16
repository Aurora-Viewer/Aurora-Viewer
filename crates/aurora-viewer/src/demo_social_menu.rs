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
    /// "Give my calling card": offered, then accepted by the demo server.
    Card,
    /// The wolf offers us his calling card.
    CardOffer,
}

pub fn step() -> Option<Step> {
    Some(match std::env::var("AURORA_DEMO_SOCIAL_MENU").ok()?.trim() {
        "1" | "menu" => Step::Menu,
        "picker" => Step::Picker,
        "invite" => Step::Invite,
        "new-set" => Step::NewSet,
        "new-set-done" => Step::NewSetDone,
        "particles" => Step::Particles,
        "card" => Step::Card,
        "card-offer" => Step::CardOffer,
        _ => return None,
    })
}

/// The resident of the scenario.
pub const RESIDENT: Uuid = super::DEMO_LOUP;

/// The demo server's answers: every calling card we offer is accepted
/// (AcceptCallingCard).
pub fn reply(cmd: &aurora_net::NetCommand) -> Option<Vec<aurora_net::NetEvent>> {
    match cmd {
        aurora_net::NetCommand::OfferCallingCard { .. } => Some(vec![aurora_net::NetEvent::CallingCardAnswered { accepted: true }]),
        aurora_net::NetCommand::AnswerCallingCard { folder, .. } => {
            log::info!(
                "demo social menu: calling card {}",
                if folder.is_some() { "accepted" } else { "declined" }
            );
            Some(Vec::new())
        }
        _ => None,
    }
}

/// The wolf's offer of his calling card (`card-offer`), from the demo region.
pub fn card_offer() -> aurora_net::NetEvent {
    aurora_net::NetEvent::CallingCardOffered {
        from: RESIDENT,
        transaction: Uuid::from_u128(0xCA4D),
        handle: super::HANDLE,
    }
}
