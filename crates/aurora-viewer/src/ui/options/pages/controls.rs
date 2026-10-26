//! Preferences › Mouse and keyboard: the Legacy Viewer's "Move & View ›
//! Movement" (panel_preferences_move.xml) and what the keys move.

use super::super::Page;
use super::super::cx::{Cx, field};

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
        cx.link(
            tr!("ui-options-shortcuts"),
            tr!("ui-options-keys-and-mouse-buttons-for-each-action"),
            tr!("ui-options-edit-shortcuts"),
            Page::Shortcuts,
            0,
        );
    });
}
