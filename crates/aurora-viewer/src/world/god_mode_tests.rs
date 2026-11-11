//! The god level the region grants (GrantGodlikePowers) and its notices.

use super::*;

fn texts(w: &World) -> Vec<String> {
    w.chat.iter().map(|l| l.text.clone()).collect()
}

#[test]
fn a_granted_god_level_is_kept_and_said_once_then_given_back() {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.apply(NetEvent::GodLevel(200));
    assert_eq!(w.god_level, 200);
    let entering = aurora_i18n::tr!("god-mode-entering", level = 200);
    assert_eq!(texts(&w).iter().filter(|t| **t == entering).count(), 1);
    // the region granting the same level again says nothing more
    w.apply(NetEvent::GodLevel(200));
    assert_eq!(texts(&w).iter().filter(|t| **t == entering).count(), 1);
    // leaving names the level given back
    w.apply(NetEvent::GodLevel(0));
    assert_eq!(w.god_level, 0);
    assert!(texts(&w).contains(&aurora_i18n::tr!("god-mode-leaving", level = 200)));
}

#[test]
fn a_god_manages_the_estate_of_a_region_it_does_not_own() {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    for event in crate::demo::events() {
        w.apply(event);
    }
    let handle = w.main_region.expect("demo region");
    let info = w.regions.get_mut(&handle).and_then(|r| r.info.as_mut()).expect("region info");
    let info = Arc::make_mut(info);
    info.owner = Uuid::from_u128(0xE57A7E);
    info.is_estate_manager = false;
    let manages = |w: &World| {
        w.moderation_place(handle, Vec3::new(128.0, 128.0, 20.0), None)
            .expect("place")
            .estate_manager
    };
    assert!(!manages(&w));
    // LLViewerRegion::canManageEstate: isGodlike() first
    w.apply(NetEvent::GodLevel(225));
    assert!(manages(&w));
}

#[test]
fn a_god_gets_the_estate_entries_outside_its_estate() {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    for event in crate::demo::events() {
        w.apply(event);
    }
    let handle = w.main_region.expect("demo region");
    let info = w.regions.get_mut(&handle).and_then(|r| r.info.as_mut()).expect("region info");
    let info = Arc::make_mut(info);
    info.owner = Uuid::from_u128(0xE57A7E);
    info.is_estate_manager = false;
    let estate = |w: &World| {
        let place = w.moderation_place(handle, Vec3::new(128.0, 128.0, 20.0), None).expect("place");
        land_moderation::rights(&place, &w.moderation_agent()).estate
    };
    assert!(!estate(&w));
    // LLAvatarActions::canEstateKickOrTeleportHomeMultiple: gods can always kick
    w.apply(NetEvent::GodLevel(200));
    assert!(estate(&w));
}
