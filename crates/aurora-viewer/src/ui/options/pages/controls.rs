//! Preferences › Mouse and keyboard: the Legacy Viewer's "Move & View ›
//! Movement" (panel_preferences_move.xml) and what the keys move.

use super::super::Page;
use super::super::cx::{Cx, Nothing, field};
use super::super::toggle;

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-mouse"), |cx| {
        cx.slider(tr!("ui-options-sensitivity"), "", &field!(mouse_sensitivity), 0.1..=5.0, "×");
        cx.toggle(tr!("ui-options-invert-vertical-axis"), "", &field!(invert_mouse));
        cx.toggle(
            tr!("ui-options-world-mouse-walk"),
            tr!("ui-options-world-mouse-walk-help"),
            &field!(world_mouse_walk),
        );
        // "Double-click on land" (double_click_action_combo)
        let choices: Vec<(crate::autopilot::DoubleClick, &str)> =
            crate::autopilot::DoubleClick::ALL.iter().map(|a| (*a, a.label())).collect();
        cx.combo(
            tr!("ui-options-double-click-ground"),
            tr!("ui-options-double-click-ground-help"),
            &field!(double_click),
            &choices,
        );
        cx.toggle(
            tr!("ui-options-double-click-scripted"),
            tr!("ui-options-double-click-scripted-help"),
            &field!(double_click_scripted),
        );
        // Legacy Viewer › Protection (FSBlockClickSit)
        cx.toggle(
            tr!("ui-options-block-click-sit"),
            tr!("ui-options-block-click-sit-help"),
            &field!(block_click_sit),
        );
    });
    cx.group(tr!("ui-options-keyboard"), |cx| {
        cx.toggle(
            tr!("ui-options-arrows-strafe"),
            tr!("ui-options-left-right-strafe-instead-of-turning-shift-reverses-this"),
            &field!(arrows_strafe),
        );
        cx.toggle(
            tr!("ui-options-arrow-keys-always-move"),
            tr!("ui-options-arrow-keys-always-move-help"),
            &field!(arrow_keys_always_move),
        );
        cx.toggle(
            tr!("ui-options-letter-keys-move"),
            tr!("ui-options-letter-keys-move-help"),
            &field!(letter_keys_move),
        );
        // FSDisableTurningAroundWhenWalkingBackwards
        cx.toggle(
            tr!("ui-options-walk-backwards-without-turning"),
            tr!("ui-options-walk-backwards-without-turning-help"),
            &field!(walk_backwards_without_turning),
        );
        // AutomaticFly (agent_jump / LLAgent::propagate), split into two
        // independent choices at the human's request.
        cx.toggle(
            tr!("ui-options-automatic-fly"),
            tr!("ui-options-automatic-fly-help"),
            &field!(automatic_fly),
        );
        cx.toggle(
            tr!("ui-options-automatic-land"),
            tr!("ui-options-automatic-land-help"),
            &field!(automatic_land),
        );
        // FSCrouchToggle, FSIgnoreFinishAnimation, FSFlyAfterTeleport
        cx.toggle(
            tr!("ui-options-crouch-toggle"),
            tr!("ui-options-crouch-toggle-help"),
            &field!(crouch_toggle),
        );
        cx.toggle(tr!("ui-options-quickjump"), tr!("ui-options-quickjump-help"), &field!(quickjump));
        cx.toggle(
            tr!("ui-options-fly-after-teleport"),
            tr!("ui-options-fly-after-teleport-help"),
            &field!(fly_after_teleport),
        );
        friends_only_row(cx);
        friends_only_keep_row(cx);
        cx.link(
            tr!("ui-options-shortcuts"),
            tr!("ui-options-keys-and-mouse-buttons-for-each-action"),
            tr!("ui-options-edit-shortcuts"),
            Page::Shortcuts,
            0,
        );
    });
}

/// "Show Friends Only" itself (FSRenderFriendsOnly), the switch of the
/// toolbar button and the World menu: an Aurora addition, the Legacy Viewer
/// has only the next row in its Preferences. Kept per account, so it waits
/// for a login.
fn friends_only_row(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    let state = cx.st.friends_only;
    let mut on = state.unwrap_or(false);
    let hint = if state.is_some() {
        tr!("ui-options-friends-only-show-help")
    } else {
        tr!("ui-options-friends-only-keep-login")
    };
    cx.row_with(tr!("toolbar-command-friends-only"), hint, &Nothing, |ui, _, _, r| {
        if ui.add_enabled_ui(state.is_some(), |ui| toggle(ui, &p, &mut on)).inner {
            r.friends_only = Some(on);
        }
        false
    });
}

/// "Keep 'Show Friends Only' enabled after teleporting" (FSRenderFriendsOnlyPersistsTP of
/// panel_preferences_move.xml): kept per account, so it waits for a login,
/// as in the Legacy Viewer.
fn friends_only_keep_row(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    let state = cx.st.friends_only_keep;
    let mut kept = state.unwrap_or(false);
    let hint = if state.is_some() {
        tr!("ui-options-friends-only-keep-help")
    } else {
        tr!("ui-options-friends-only-keep-login")
    };
    cx.row_with(tr!("ui-options-friends-only-keep"), hint, &Nothing, |ui, _, _, r| {
        if ui.add_enabled_ui(state.is_some(), |ui| toggle(ui, &p, &mut kept)).inner {
            r.friends_only_keep = Some(kept);
        }
        false
    });
}
