//! The right-click menu of a conversation in the list (the workspace's
//! column and its rail, the tabs of the Bubbles and Lines styles), after
//! the Legacy Viewer's conversation items (menu_conversation.xml,
//! LLConversationItemSession::buildContextMenu, LLFloaterIMContainer::
//! doToSelectedConversation / enableContextMenuItem): an IM gets Close and
//! its call, then the resident's own menu (the one of their name in a
//! conversation, so both stay the same) and the rows the conversation item
//! adds to it; a group Close, its call, the transcript and the group's
//! actions (and its snooze, the chiclet menu's "Snooze Session"); a
//! conference Close, its call and the transcript; the nearby chat its
//! transcript. Each one first "Detach" into its own window, or "Attach to
//! Conversations" when it has one (`popout.rs`).

use super::sidebar::{self, Kind};
use super::{ChatUi, ConvAction, close_tip};
use crate::theme::Palette;
use crate::ui::context::{self, AvatarList, CtxAction};
use crate::ui::menu;
use crate::world::World;
use uuid::Uuid;

/// The menu of the entry `resp` of a conversation (None: the nearby chat).
pub(super) fn attach(resp: &egui::Response, p: &Palette, world: &World, actions: &mut Vec<ConvAction>, conversation: Option<Uuid>) {
    menu::context_menu(resp, p, |ui| items(ui, p, world, actions, conversation));
}

/// The rows of the menu.
pub(super) fn items(ui: &mut egui::Ui, p: &Palette, world: &World, actions: &mut Vec<ConvAction>, conversation: Option<Uuid>) {
    let ctx = ui.ctx().clone();
    let act = |a: CtxAction| context::request(&ctx, a);
    // "Chat transcript…" (enabled while the account keeps transcripts, the
    // window says when there is none, as the residents' menus)
    let history = |ui: &mut egui::Ui| {
        if menu::item_if(
            ui,
            p,
            "history",
            aurora_i18n::tr!("common-view-conversation-history"),
            world.chat_log.dir().is_some(),
        ) {
            act(CtxAction::ShowHistory(conversation));
        }
    };
    // in its own window, or back into Conversations (`popout.rs`)
    super::popout::menu_row(ui, p, actions, conversation);
    // CONV_SESSION_NEARBY: its transcript only
    let Some(id) = conversation else {
        history(ui);
        return;
    };
    let kind = sidebar::kind(world, &id);
    if menu::item(ui, p, "x", close_tip(world, &id)) {
        actions.push(ConvAction::Close(id));
    }
    // "Snooze Session" of a group's chiclet menu (menu_fs_imchiclet_group.xml)
    if super::can_snooze(world, &id) && menu::item(ui, p, super::SNOOZE_ICON, aurora_i18n::tr!("ui-chat-snooze")) {
        actions.push(ConvAction::Snooze(id));
    }
    voice(ui, p, world, actions, id);
    match kind {
        Kind::Private => {
            menu::separator(ui, p);
            context::avatar_list_menu(ui, p, world, id, AvatarList::Name);
            // the conversation item's rows the name's menu does not have
            // (buildParticipantMenuOptions)
            menu::separator(ui, p);
            menu::todo(ui, p, "users-plus", aurora_i18n::tr!("ui-context-invite-to-group"));
            if menu::item_if(ui, p, "map", aurora_i18n::tr!("common-show-on-map-2"), world.can_locate_avatar(id)) {
                act(CtxAction::ShowOnMap(id));
            }
            menu::todo(ui, p, "share", aurora_i18n::tr!("common-share"));
            if menu::item(ui, p, "coin-filled", aurora_i18n::tr!("common-pay")) {
                act(CtxAction::PayResident(id));
            }
        }
        Kind::Group => {
            history(ui);
            menu::separator(ui, p);
            if menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("people-group-profile")) {
                crate::ui::group_profile::request_open(ui.ctx(), id);
            }
            // can_activate_group: not the active one already; leaving and
            // the chat's block need the membership
            let member = world.groups.is_member(&id);
            if menu::item_if(
                ui,
                p,
                "circle-check-filled",
                aurora_i18n::tr!("ui-chat-menu-activate-group"),
                member && world.groups.active != id,
            ) {
                act(CtxAction::ActivateGroup(id));
            }
            if menu::item_if(
                ui,
                p,
                "message-off",
                aurora_i18n::tr!("common-stop-receiving-this-group-s-chat"),
                member,
            ) {
                actions.push(ConvAction::BlockGroupChat(id));
            }
            if menu::item_if(ui, p, "logout", aurora_i18n::tr!("ui-contacts-leave-group"), member) {
                act(CtxAction::LeaveGroup(id));
            }
        }
        Kind::Conference => history(ui),
    }
}

/// The conversation's call: hang up while it is the call in progress
/// ("Disconnect from voice"), else call (greyed with voice off, as
/// LLAvatarActions::canCall).
fn voice(ui: &mut egui::Ui, p: &Palette, world: &World, actions: &mut Vec<ConvAction>, id: Uuid) {
    if world.call_of(&id).is_some() {
        if menu::item(ui, p, "phone-off", aurora_i18n::tr!("ui-call-hang-up")) {
            actions.push(ConvAction::HangUp);
        }
    } else if menu::item_if(
        ui,
        p,
        "phone-filled",
        aurora_i18n::tr!("common-voice-call"),
        world.calls.voice_enabled,
    ) {
        actions.push(ConvAction::Call(id));
    }
}

/// `AURORA_DEMO_CONV_MENU=local|im|group|conference`: the entry whose menu
/// the demo opens (the first conversation of that kind), for a capture.
pub(super) fn demo_mark(ctx: &egui::Context, world: &World, st: &ChatUi, conversation: Option<Uuid>, rect: egui::Rect) {
    let Some(want) = st.demo_menu.as_deref() else {
        return;
    };
    let kind = match conversation {
        None => "local",
        Some(id) => match sidebar::kind(world, &id) {
            Kind::Private => "im",
            Kind::Group => "group",
            Kind::Conference => "conference",
        },
    };
    let id = demo_id();
    if kind == want && ctx.data(|d| d.get_temp::<egui::Rect>(id)).is_none() {
        ctx.data_mut(|d| d.insert_temp(id, rect));
    }
}

fn demo_id() -> egui::Id {
    egui::Id::unique("conv_menu_demo")
}

/// The right click of `AURORA_DEMO_CONV_MENU` on its entry, at frames 700
/// to 702 (capture at 780 with `AURORA_FPS_LIMIT=60`).
pub(super) fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    if !(700..=702).contains(&frame) {
        return Vec::new();
    }
    let Some(r) = ctx.data(|d| d.get_temp::<egui::Rect>(demo_id())) else {
        return Vec::new();
    };
    // on the name, clear of the close button and the count
    let pos = egui::pos2(r.left() + r.width() * 0.4, r.center().y);
    match frame {
        700 => vec![egui::Event::PointerMoved(pos)],
        _ => vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: frame == 701,
            modifiers: Default::default(),
        }],
    }
}
