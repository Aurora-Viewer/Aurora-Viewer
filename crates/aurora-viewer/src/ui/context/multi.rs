//! The menu of several residents chosen in a list (Ctrl / Shift): Firestorm's
//! multiple-selection menus of People › Nearby and of the radar
//! (menu_people_nearby_multiselect.xml, menu_fs_radar_multiselect.xml,
//! PeopleContextMenu in llpanelpeoplemenus.cpp, originally LGPL 2.1). Only
//! what those menus allow for several people; nothing here depends on how
//! the list draws its rows.

use super::{CtxAction, request};
use crate::theme::Palette;
use crate::ui::menu;
use crate::world::World;
use uuid::Uuid;

/// The entries of the menu and whether each is enabled
/// (PeopleContextMenu::enableContextMenuItem), apart from drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiFacts {
    /// "can_add": never for more than one (EXT-7389).
    pub add_friends: bool,
    /// "can_delete": everyone chosen is a friend.
    pub remove_friends: bool,
    /// "can_call": voice is on (LLAvatarActions::canCall).
    pub call: bool,
    /// "can_offer_teleport": every one of them can be offered one.
    pub offer_teleport: bool,
}

pub fn facts(world: &World, ids: &[Uuid]) -> MultiFacts {
    let me = world.agent_id;
    let friend = |id: &Uuid| world.social.is_friend(id);
    // LLAvatarActions::canOfferTeleport: not ourselves, and a friend who
    // is offline cannot receive one
    let reachable = |id: &Uuid| *id != me && world.social.friends.iter().find(|f| f.id == *id).is_none_or(|f| f.online);
    MultiFacts {
        add_friends: ids.len() == 1 && !ids.iter().any(friend),
        remove_friends: !ids.is_empty() && ids.iter().all(friend),
        call: world.calls.voice_enabled && !ids.is_empty(),
        offer_teleport: !ids.is_empty() && ids.iter().all(reachable),
    }
}

/// Right-click menu of several residents of a list.
pub fn avatars_menu(ui: &mut egui::Ui, p: &Palette, world: &World, ids: &[Uuid]) {
    let ctx = ui.ctx().clone();
    let act = |a: CtxAction| request(&ctx, a);
    let f = facts(world, ids);
    // (menu_fs_radar_multiselect.xml, in its order)
    if menu::item_if(ui, p, "user-plus", aurora_i18n::tr!("ui-context-multi-add-friends"), f.add_friends)
        && let Some(id) = ids.first()
    {
        act(CtxAction::AddFriend(*id));
    }
    super::contact_set_submenu(ui, p, world, ids, aurora_i18n::tr!("ui-context-add-to-contact-set"));
    if menu::item_if(
        ui,
        p,
        "user-minus",
        aurora_i18n::tr!("ui-context-multi-remove-friends"),
        f.remove_friends,
    ) {
        act(CtxAction::RemoveFriends(ids.to_vec()));
    }
    menu::separator(ui, p);
    // PeopleContextMenu::startConference, LLAvatarActions::startAdhocCall
    if menu::item(ui, p, "message-circle-filled", aurora_i18n::tr!("ui-context-multi-im")) {
        act(CtxAction::Conference(ids.to_vec()));
    }
    if menu::item_if(ui, p, "phone-filled", aurora_i18n::tr!("common-voice-call"), f.call) {
        act(CtxAction::ConferenceCall(ids.to_vec()));
    }
    // "*TODO: unimplemented" for several in Firestorm too
    menu::todo(ui, p, "share", aurora_i18n::tr!("common-share"));
    menu::todo(ui, p, "coin-filled", aurora_i18n::tr!("common-pay"));
    if menu::item_if(ui, p, "send-filled", aurora_i18n::tr!("common-offer-teleport"), f.offer_teleport) {
        act(CtxAction::OfferTeleports(ids.to_vec()));
    }
    menu::separator(ui, p);
    // Render Settings (Nearby.People.SetRenderMode) for each
    menu::submenu(
        ui,
        p,
        "adjustments-filled",
        aurora_i18n::tr!("ui-context-multi-render-settings"),
        true,
        |ui| {
            for (mode, icon, label) in [
                (
                    0u8,
                    "adjustments-filled",
                    aurora_i18n::tr!("ui-context-display-according-to-your-preferences"),
                ),
                (1, "ghost-filled", aurora_i18n::tr!("ui-context-never-fully-render")),
                (2, "user-scan", aurora_i18n::tr!("ui-context-always-show-full-detail")),
            ] {
                if menu::item(ui, p, icon, label) {
                    for id in ids {
                        act(CtxAction::SetRender(*id, mode));
                    }
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_people_get_firestorms_rules() {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.agent_id = Uuid::from_u128(1);
        w.calls.voice_enabled = true;
        let (a, b) = (Uuid::from_u128(2), Uuid::from_u128(3));
        let f = facts(&w, &[a, b]);
        // never "Add friends" for more than one, "Remove" only for friends
        assert!(!f.add_friends && !f.remove_friends);
        assert!(f.call && f.offer_teleport);
        assert!(facts(&w, &[a]).add_friends);
        w.calls.voice_enabled = false;
        assert!(!facts(&w, &[a, b]).call);
        // ourselves cannot be offered a teleport
        assert!(!facts(&w, &[a, w.agent_id]).offer_teleport);
    }
}
