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
//! `LLPanelPermissions::refresh` (`indra/newview/llpanelpermissions.cpp`)
//! disables both without the modify right, and a disabled control is not
//! given the focus: a read-only row is left alone here too.
//!
//! A press on a control, or on the field itself, stays with that widget.
//! A direct click still places the caret: selecting all on that click is
//! the field's own `select_on_focus`, and it is not done here. Another tab
//! does not build these rows, so the click does not open General to focus
//! a field that is not shown, and a row scrolled out of view is not focused
//! either: typing would replace a name the user cannot see. The Legacy
//! Viewer's `focusFirstItem` would then focus whatever control is first on
//! that tab; these two fields are the ones with `select_on_focus`.
//!
//! Deliberate difference: only Name and Description are remembered, where
//! `LLFloater::setFocus` restores whichever control had the focus last.
//!
//! General is also drawn by Object > Profile, a window of its own: the field
//! ids are scoped to the window that draws them, and only the build window's
//! fields are remembered and focused.

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;

use super::INSPECTOR_ID;

/// The two rows with `select_on_focus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Which {
    #[default]
    Name,
    Desc,
}

impl Which {
    const ALL: [Self; 2] = [Self::Name, Self::Desc];

    fn salt(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Desc => "desc",
        }
    }
}

/// What the build window drew of a row in its latest pass.
#[derive(Clone, Copy, Default)]
struct Shown {
    pass: u64,
    chars: usize,
    editable: bool,
    /// Some of the field is inside its clip rect (not scrolled out).
    visible: bool,
}

fn window() -> egui::Id {
    egui::Id::unique(INSPECTOR_ID)
}

/// The row's id in the window (layer) `layer`.
pub(super) fn field_id_on(layer: egui::Id, which: Which) -> egui::Id {
    layer.with(("select-on-focus", which.salt()))
}

/// The row's id in the window that draws it: Object > Profile's General gets
/// its own, so the two fields never share focus, keys or commits.
pub(super) fn field_id(ui: &egui::Ui, which: Which) -> egui::Id {
    field_id_on(ui.layer_id().id, which)
}

/// The build window's row.
pub(super) fn inspector_field(which: Which) -> egui::Id {
    field_id_on(window(), which)
}

fn last_key() -> egui::Id {
    window().with("select-on-focus-last")
}

fn shown_key(which: Which) -> egui::Id {
    window().with(("select-on-focus-shown", which.salt()))
}

fn keep_key() -> egui::Id {
    window().with("select-on-focus-keep")
}

fn last(ctx: &egui::Context) -> Which {
    ctx.data(|d| d.get_temp::<Which>(last_key())).unwrap_or_default()
}

/// Name or Description of the build window, when one of them holds the
/// keyboard. Another window's rows have other ids and are not remembered.
pub(super) fn note(ctx: &egui::Context) {
    let Some(focused) = ctx.memory(|m| m.focused()) else {
        return;
    };
    let Some(which) = Which::ALL.into_iter().find(|&which| focused == inspector_field(which)) else {
        return;
    };
    if last(ctx) != which {
        ctx.data_mut(|d| d.insert_temp(last_key(), which));
    }
}

/// The row as the build window drew it this pass, so a later empty click can
/// select its whole text. Ignored for another window's General.
pub(super) fn shown(ui: &egui::Ui, which: Which, response: &egui::Response, editable: bool, value: &str) {
    if response.layer_id.id != window() {
        return;
    }
    let now = Shown {
        pass: ui.ctx().cumulative_pass_nr(),
        chars: value.chars().count(),
        editable,
        visible: response.interact_rect.is_positive(),
    };
    // In place: no allocation once the entry exists.
    ui.ctx().data_mut(|d| *d.get_temp_mut_or_default::<Shown>(shown_key(which)) = now);
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
    let id = inspector_field(last(ctx));
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
    let window = window();
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
/// in this window) and select its whole text. No-op when the build window did
/// not draw that row this pass, drew it scrolled out of view, or drew it read
/// only (`LLPanelPermissions::refresh` disables it, `setFocus` skips it).
pub(super) fn apply(ctx: &egui::Context) {
    let which = last(ctx);
    let Some(shown) = ctx.data(|d| d.get_temp::<Shown>(shown_key(which))) else {
        return;
    };
    if shown.pass != ctx.cumulative_pass_nr() || !shown.visible || !shown.editable {
        return;
    }
    let id = inspector_field(which);
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::default(), CCursor::new(shown.chars))));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
    ctx.data_mut(|d| d.insert_temp(keep_key(), true));
}
