//! RLVa `@showloc` and the SLURLs, the teleport history and the windows
//! that name the agent's place, as the Legacy Viewer hides them
//! (LLSLURL::getSLURLString, llslurl.cpp; LLTeleportHistory, llteleporthistory.cpp).

use super::*;
use aurora_net::{NetCommand, NetEvent};

/// The demo world, arrived, RLVa on and `@showloc` set from the console.
pub(crate) fn showloc_world() -> World {
    let mut w = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        w.apply(event);
    }
    w.rlva_configure(true);
    w.rlva_console_input("@showloc=n");
    assert!(w.rlva.has("showloc"));
    w
}

/// LLAgentUI::buildSLURL then getSLURLString: the place the agent is in is
/// "(Hidden region)" with no position under `@showloc`; a region the agent
/// is not connected to keeps its address, and so does the restriction's end.
#[test]
fn showloc_hides_the_connected_region_of_the_agents_slurl() {
    let mut w = showloc_world();
    let hidden = format!("http://maps.secondlife.com/secondlife/{}", aurora_i18n::tr!("rlva-hidden-region"));
    assert_eq!(w.back_slurl().as_deref(), Some(hidden.as_str()));
    let here = w.region_name();
    assert_eq!(w.rlva_slurl_on(None, &here, Vec3::new(1.0, 2.0, 3.0)), hidden);
    assert_eq!(
        w.rlva_slurl_on(None, "Far Away", Vec3::new(1.0, 2.0, 3.0)),
        "http://maps.secondlife.com/secondlife/Far%20Away/1/2/3"
    );
    let (label, regions) = w.rlva_slurl_hidden_regions().expect("@showloc");
    assert_eq!(label, aurora_i18n::tr!("rlva-hidden-region"));
    assert!(regions.contains(&here));
    w.rlva_console_input("@showloc=y");
    assert!(w.rlva_slurl_hidden_regions().is_none());
    assert!(w.back_slurl().is_some_and(|s| s.contains(&crate::slurl::encode(&here))));
}

/// LLTeleportHistory::updateCurrentLocation (llteleporthistory.cpp): a
/// place reached under `@showloc` is kept as "(Hidden parcel)", leading
/// nowhere.
#[test]
fn showloc_keeps_an_arrival_as_the_hidden_parcel() {
    let mut w = showloc_world();
    let handle = w.main_region.expect("arrived");
    w.begin_teleport(false);
    w.apply(NetEvent::TeleportStarted { flags: 0 });
    w.apply(NetEvent::TeleportFinished { handle, flying: false });
    w.apply(NetEvent::AgentMovementComplete {
        handle,
        position: Vec3::new(60.0, 70.0, 25.0),
        look_at: Vec3::X,
    });
    let session = w.tp_history.session();
    let current = &session.items[session.current];
    assert_eq!(current.region, aurora_i18n::tr!("rlva-hidden-parcel"));
    assert!(current.is_hidden());
}

/// LLPanelProfileClassifieds::updateButtons greys New under `@showloc`, and
/// LLPanelProfileClassified::onSetLocationClick (llpanelprofileclassifieds.cpp)
/// has no RLVa test: an existing classified moved to where the agent is
/// is published, a new one is not.
#[test]
fn showloc_publishes_an_existing_classified_moved_here() {
    use aurora_net::profile::ClassifiedInfo;
    let mut w = showloc_world();
    let mut classified = ClassifiedInfo {
        id: Uuid::from_u128(0x0C1A),
        creator: w.agent_id,
        pos_global: glam::DVec3::new(1.0, 2.0, 3.0),
        ..Default::default()
    };
    let update = |c: &ClassifiedInfo| NetCommand::ClassifiedUpdate(Box::new(c.clone()));
    assert!(!w.rlva_allows_command(&update(&classified), Uuid::nil()), "New is greyed");
    w.profiles.classifieds.insert(classified.id, classified.clone());
    classified.pos_global = w.agent_global().expect("arrived");
    assert!(w.rlva_allows_command(&update(&classified), Uuid::nil()), "Set location");
}

/// handle_lure (llviewermessage.cpp): the invitation's region is "(Hidden)"
/// under `@showloc`, the region's name without it.
#[test]
fn showloc_names_the_lure_region_hidden() {
    let mut w = showloc_world();
    assert_eq!(w.rlva_lure_region(), aurora_i18n::tr!("rlva-hidden-generic"));
    w.rlva_console_input("@showloc=y");
    assert_eq!(w.rlva_lure_region(), w.region_name());
}
