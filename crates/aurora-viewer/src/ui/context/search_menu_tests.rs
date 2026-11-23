//! The menu of a resident found by Search › People
//! (menu_fs_avatar_search.xml), drawn in real egui frames.

use super::*;
use crate::ui::widgets::elide_tests::{Frames, Text, painted};
use aurora_net::social::GroupMembership;

const NOVA: Uuid = Uuid::from_u128(0x7);

fn world() -> World {
    let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.agent_id = Uuid::from_u128(1);
    w.calls.voice_enabled = true;
    w
}

fn texts(w: &World, p: &Palette) -> (Frames, Vec<Text>) {
    let mut f = Frames::new();
    f.run(Vec::new(), |ui| avatar_roster_menu(ui, p, w, NOVA, AvatarList::Search));
    let texts = f.run(Vec::new(), |ui| avatar_roster_menu(ui, p, w, NOVA, AvatarList::Search));
    (f, texts)
}

/// Clicks `label` in the menu: what it asked.
fn click(w: &World, p: &Palette, label: &str) -> Vec<CtxAction> {
    let (mut f, texts) = texts(w, p);
    let at = painted(&texts, label).rect.center();
    let _ = take_requests(&f.ctx);
    for pressed in [true, false] {
        let events = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        f.run(events, |ui| avatar_roster_menu(ui, p, w, NOVA, AvatarList::Search));
    }
    take_requests(&f.ctx)
}

#[test]
fn the_search_menu_has_the_legacy_entries_in_their_order() {
    let p = crate::theme::Theme::default().palette();
    let w = world();
    let (_, texts) = texts(&w, &p);
    let row = |label: &str| painted(&texts, label).rect.center().y;
    let order = [
        aurora_i18n::text("common-view-profile"),
        aurora_i18n::text("common-become-friends"),
        aurora_i18n::text("ui-context-add-to-contact-set"),
        aurora_i18n::text("ui-context-send-im"),
        aurora_i18n::text("common-voice-call"),
        aurora_i18n::text("common-share"),
        aurora_i18n::text("common-pay"),
        aurora_i18n::text("common-offer-teleport"),
        aurora_i18n::text("common-request-teleport"),
        aurora_i18n::text("ui-context-invite-to-group"),
        aurora_i18n::text("common-ignore"),
    ];
    let ys: Vec<f32> = order.iter().map(|l| row(l)).collect();
    assert!(ys.windows(2).all(|pair| pair[0] < pair[1]), "{order:?} at {ys:?}");
    // Aurora's own additions stay
    for extra in ["common-zoom-2", "common-teleport-to", "ui-context-report-abuse", "common-copy-name"] {
        painted(&texts, aurora_i18n::text(extra));
    }
}

#[test]
fn the_search_menu_entries_ask_for_their_action() {
    let p = crate::theme::Theme::default().palette();
    let w = world();
    let asked = click(&w, &p, aurora_i18n::text("common-pay"));
    assert!(matches!(&asked[..], [CtxAction::PayResident(i)] if *i == NOVA), "{asked:?}");
    let asked = click(&w, &p, aurora_i18n::text("common-voice-call"));
    assert!(matches!(&asked[..], [CtxAction::Call(i)] if *i == NOVA), "{asked:?}");
    let asked = click(&w, &p, aurora_i18n::text("ui-context-invite-to-group"));
    assert!(matches!(&asked[..], [CtxAction::InviteToGroup(i)] if *i == NOVA), "{asked:?}");
}

/// "can_call": no call while voice is off.
#[test]
fn the_search_menus_call_is_greyed_without_voice() {
    let p = crate::theme::Theme::default().palette();
    let mut w = world();
    w.calls.voice_enabled = false;
    assert!(click(&w, &p, aurora_i18n::text("common-voice-call")).is_empty());
}

/// "group_ban_separator" / BanFromAllGroups: only with the ban power in one
/// of our groups at least (canBanFromAnyGroup).
#[test]
fn ban_from_all_groups_shows_only_with_a_ban_power() {
    let p = crate::theme::Theme::default().palette();
    let mut w = world();
    let ban_all = aurora_i18n::text("ui-chat-ban-from-all-groups");
    assert!(!texts(&w, &p).1.iter().any(|t| t.source == ban_all));
    w.groups.groups.push(GroupMembership {
        id: Uuid::from_u128(0x60),
        name: "Builders".into(),
        insignia: Uuid::nil(),
        powers: aurora_net::groups::powers::GROUP_BAN_ACCESS,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
    let asked = click(&w, &p, ban_all);
    assert!(
        matches!(&asked[..], [CtxAction::BanFromAllGroups { agent, ban: true }] if *agent == NOVA),
        "{asked:?}"
    );
}
