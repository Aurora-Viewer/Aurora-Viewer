//! An empty click gives the build window back its last text field.
//!
//! Legacy Viewer: `LLFloater::handleMouseDown` / `handleRightMouseDown`
//! (`indra/llui/llfloater.cpp`) call `bringToFront`, and
//! `LLFloaterView::bringToFront` calls `setFocus` only when
//! `!gFocusMgr.childHasKeyboardFocus`. `LLFloater::setFocus` restores
//! `getLastFocusForGroup` when that control is enabled and visible, after
//! `LLPanel::setFocus` has tried `focusFirstItem` (`indra/llui/llpanel.cpp`).
//! Object Name and Object Description set `select_on_focus="true"`
//! (`indra/newview/skins/default/xui/en/floater_tools.xml`), so
//! `LLLineEditor::setFocus` (`indra/llui/lllineeditor.cpp`) selects all.
//!
//! A press on a control, or on the field itself, stays with that widget.
//! A direct click still places the caret: selecting all on that click is
//! the field's own `select_on_focus`, and it is not done here. Another tab
//! does not build these rows, so the click does not open General to focus
//! a field that is not shown. The Legacy Viewer's `focusFirstItem` would
//! then focus whatever control is first on that tab; these two fields are
//! the ones with `select_on_focus`.

use egui::text::CCursorRange;
use egui::text_edit::TextEditState;

use super::INSPECTOR_ID;

const NAME: &str = "name";
const DESC: &str = "desc";

pub(super) fn field_id(which: &str) -> egui::Id {
    egui::Id::unique((INSPECTOR_ID, "select-on-focus", which))
}

fn last_key() -> egui::Id {
    egui::Id::unique((INSPECTOR_ID, "select-on-focus", "last"))
}

fn value_key(which: &str) -> egui::Id {
    field_id(which).with("value")
}

fn keep_key() -> egui::Id {
    egui::Id::unique((INSPECTOR_ID, "select-on-focus", "keep"))
}

/// Name or Description, when one of them holds the keyboard.
pub(super) fn note(ctx: &egui::Context) {
    let Some(focused) = ctx.memory(|m| m.focused()) else {
        return;
    };
    let which = if focused == field_id(NAME) {
        NAME
    } else if focused == field_id(DESC) {
        DESC
    } else {
        return;
    };
    ctx.data_mut(|d| d.insert_temp(last_key(), which.to_string()));
}

/// The text that field is showing, so a later empty click can select it.
pub(super) fn remember_value(ctx: &egui::Context, which: &str, value: &str) {
    ctx.data_mut(|d| d.insert_temp(value_key(which), value.to_string()));
}

/// The release of the empty press is one click. Click-away focus would take
/// the keyboard back on that release, because the pointer is not on the
/// field the press just focused (`ui/click_away_focus.rs` surrenders on
/// click unless this pass asked for a focus). Ask again so the release
/// keeps it. Legacy Viewer focuses on mouse-down
/// (`LLFloater::handleMouseDown`) and the mouse-up does not undo that.
pub(super) fn keep_through_release(ctx: &egui::Context) {
    let armed = ctx.data(|d| d.get_temp::<bool>(keep_key())).unwrap_or(false);
    if !armed {
        return;
    }
    let (released, down) = ctx.input(|i| (i.pointer.any_released(), i.pointer.any_down()));
    if !released && down {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(keep_key(), false));
    if !released {
        return;
    }
    let which = ctx
        .data(|d| d.get_temp::<String>(last_key()))
        .filter(|which| which == NAME || which == DESC)
        .unwrap_or_else(|| NAME.to_string());
    let id = field_id(&which);
    if ctx.memory(|m| m.has_focus(id)) {
        ctx.memory_mut(|m| m.request_focus(id));
    }
}

/// Primary or secondary press on this window's empty space, while the
/// window does not already hold the keyboard.
pub(super) fn pending(ctx: &egui::Context) -> bool {
    if crate::ui::widgets::floater_holds_keyboard(ctx, INSPECTOR_ID) || crate::ui::widgets::floater_minimized(ctx, INSPECTOR_ID) {
        return false;
    }
    let Some(pos) = ctx.input(|i| {
        let pointer = &i.pointer;
        let pressed = pointer.button_pressed(egui::PointerButton::Primary) || pointer.button_pressed(egui::PointerButton::Secondary);
        pressed.then(|| pointer.press_origin()).flatten()
    }) else {
        return false;
    };
    let window = egui::Id::unique(INSPECTOR_ID);
    if ctx.layer_id_at(pos).map(|layer| layer.id) != Some(window) || crate::ui::widgets::pointer_on_floater_edge(ctx, pos) {
        return false;
    }
    // Drag-only widgets (a scroll background) do not take the press. A
    // clickable one does, disabled included: its stored sense still clicks,
    // which is what egui's hit test strips for the frame. The move plane
    // covers the whole floater (`window_click_tolerance` adds a click to
    // Area's drag id, `ui/widgets.rs`). It is the window background, not a
    // control. A button, a tab or a field above it still senses the click.
    // The title bar's double-click target is one of those controls.
    let background = window.with("move");
    let under = ctx.interaction_snapshot(|snap| snap.contains_pointer.iter().copied().collect::<Vec<_>>());
    let taken = under.into_iter().any(|id| {
        id != background
            && ctx
                .read_response(id)
                .is_some_and(|response| response.layer_id.id == window && response.sense.senses_click())
    });
    !taken
}

/// Focus the last of Name and Description (Name when neither has been focused
/// in this window) and select its whole text. No-op when that row is not
/// on screen this pass.
pub(super) fn apply(ctx: &egui::Context) {
    let which = ctx
        .data(|d| d.get_temp::<String>(last_key()))
        .filter(|which| which == NAME || which == DESC)
        .unwrap_or_else(|| NAME.to_string());
    let id = field_id(&which);
    let shown = ctx.viewport(|vp| vp.this_pass.widgets.contains(id));
    if !shown {
        return;
    }
    let text = ctx.data(|d| d.get_temp::<String>(value_key(&which))).unwrap_or_default();
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::select_all_str(&text)));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
    ctx.data_mut(|d| d.insert_temp(keep_key(), true));
}
