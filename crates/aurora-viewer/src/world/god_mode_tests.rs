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
