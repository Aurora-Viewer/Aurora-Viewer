//! The keyboard of the modal dialogs: Enter clicks the dialog's main button,
//! and its first text field takes the focus, its text selected, when the
//! dialog opens.
//!
//! Follows `LLPanel::handleKeyHere` (indra/llui/llpanel.cpp): Return with no
//! modifier commits the default button, unless the focused widget keeps it
//! (a focused button clicks itself, a multi-line editor writes a new line);
//! and `LLToastAlertPanel` (indra/newview/lltoastalertpanel.cpp): the line
//! editor of an alert gets the focus with its text selected. The default
//! button of the Legacy Viewer's alerts is their affirmative answer
//! (`default="true"`, index 0 of `okcancelbuttons`), here the `Primary`
//! button, or the `Danger` one of a dialog with no `Primary`.
//!
//! The buttons are drawn before the dialog knows what Enter did (a
//! single-line field gives up its focus on Enter, a multi-line one keeps
//! it), so the click lands on the next pass. A dialog that handles Enter
//! itself consumes it (`widgets::enter_pressed`) and gets no second click.

use egui::{Context, Id, Key, LayerId};

/// What `Modal::show` keeps of a dialog between passes.
#[derive(Clone, Copy, Default)]
struct State {
    /// The pass the dialog was last shown on.
    last_pass: u64,
    /// The first text field still has to take the focus.
    focus_pending: bool,
    /// The button Enter clicks, and the pass it clicks it on.
    click: Option<(Id, u64)>,
}

/// The dialog being drawn, for the buttons inside it.
#[derive(Clone, Copy)]
struct Current {
    layer: LayerId,
    primary: Option<Id>,
    danger: Option<Id>,
    click: Option<Id>,
}

fn state_id(id: Id) -> Id {
    id.with("aurora_modal_keys")
}

fn current_id() -> Id {
    Id::unique("aurora_modal_keys_current")
}

/// Kept by `begin` for `end`.
pub(super) struct Scope {
    id: Id,
    layer: LayerId,
    state: State,
    /// Enter pressed while this dialog was the top one.
    enter: bool,
    /// The widget focused before the dialog's body.
    focused: Option<Id>,
    outer: Option<Current>,
}

/// A fresh press of Enter with no modifier: Enter held down since the
/// dialog opened (on the list that opened it) does not answer it.
fn plain_enter(ctx: &Context) -> bool {
    ctx.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, repeat: false, modifiers, .. } if modifiers.is_none()))
    })
}

fn layer_of(ctx: &Context, id: Id) -> Option<LayerId> {
    ctx.viewport(|v| {
        v.this_pass
            .widgets
            .get(id)
            .or_else(|| v.prev_pass.widgets.get(id))
            .map(|w| w.layer_id)
    })
}

fn is_text_field(ctx: &Context, id: Id) -> bool {
    egui::TextEdit::load_state(ctx, id).is_some()
}

/// Before the dialog's body.
pub(super) fn begin(ctx: &Context, layer: LayerId) -> Scope {
    let id = layer.id;
    let pass = ctx.cumulative_pass_nr();
    let mut state: State = ctx.data(|d| d.get_temp(state_id(id))).unwrap_or_default();
    if state.last_pass == 0 || state.last_pass + 1 < pass {
        // opening: not shown on the previous pass
        state = State {
            focus_pending: true,
            ..State::default()
        };
    }
    state.last_pass = pass;
    let click = state.click.take().filter(|&(_, at)| at == pass).map(|(button, _)| button);
    // the top modal of the previous pass: the Enter that opened the dialog
    // does not answer it
    let top = ctx.memory(|m| m.top_modal_layer()) == Some(layer);
    let current = Current {
        layer,
        primary: None,
        danger: None,
        click,
    };
    let outer = ctx.data_mut(|d| {
        let outer = d.get_temp(current_id());
        d.insert_temp(current_id(), current);
        outer
    });
    Scope {
        id,
        layer,
        state,
        enter: top && plain_enter(ctx),
        focused: ctx.memory(|m| m.focused()),
        outer,
    }
}

/// After the dialog's body.
pub(super) fn end(ctx: &Context, scope: Scope) {
    let Scope {
        id,
        layer,
        mut state,
        enter,
        focused,
        outer,
    } = scope;
    let current: Option<Current> = ctx.data_mut(|d| {
        let current = d.get_temp(current_id());
        match outer {
            Some(outer) => {
                d.insert_temp(current_id(), outer);
            }
            None => d.remove::<Current>(current_id()),
        }
        current
    });
    let focused_now = ctx.memory(|m| m.focused());
    let inside = |id: Id| layer_of(ctx, id) == Some(layer);
    if state.focus_pending {
        focus_first_field(ctx, layer, focused_now.filter(|&f| inside(f)), &mut state);
    }
    // still there: nothing in the dialog took it
    if enter && plain_enter(ctx) {
        let answers = match focused.filter(|&f| inside(f)) {
            None => true,
            // a single-line field gives up its focus on Enter
            Some(field) => is_text_field(ctx, field) && focused_now != Some(field),
        };
        if let Some(button) = current.and_then(|c| c.primary.or(c.danger)).filter(|_| answers) {
            let pass = ctx.cumulative_pass_nr();
            state.click = Some((button, pass + 1));
            ctx.request_repaint();
        }
    }
    ctx.data_mut(|d| d.insert_temp(state_id(id), state));
}

/// The first enabled text field of the dialog takes the focus, its text
/// selected; a dialog with no field takes it away from the widgets behind.
/// The first pass of an area only measures it, its widgets disabled: wait
/// for one that can be used.
fn focus_first_field(ctx: &Context, layer: LayerId, focused: Option<Id>, state: &mut State) {
    if focused.is_some() {
        // the dialog chose its own
        state.focus_pending = false;
        return;
    }
    let usable: Vec<(Id, bool)> = ctx.viewport(|v| {
        v.this_pass
            .widgets
            .get_layer(layer)
            .filter(|w| w.enabled && w.sense.is_focusable())
            .map(|w| (w.id, w.sense.senses_drag()))
            .collect()
    });
    if usable.is_empty() {
        return;
    }
    // in the order they were laid out
    let field = usable
        .into_iter()
        .find(|&(id, drag)| drag && is_text_field(ctx, id))
        .map(|(id, _)| id);
    state.focus_pending = false;
    match field {
        Some(field) => {
            ctx.memory_mut(|m| m.request_focus(field));
            if let Some(mut text) = egui::TextEdit::load_state(ctx, field) {
                // clamped to the text by the field
                text.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(usize::MAX),
                )));
                text.store(ctx, field);
            }
        }
        None => {
            if ctx.memory(|m| m.focused()).is_some() {
                ctx.memory_mut(|m| m.stop_text_input());
            }
        }
    }
}

/// Which answer of the dialog a button gives.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    Primary,
    Danger,
}

/// An `answer` button of a dialog: the first enabled one of its kind is the
/// one Enter clicks.
pub(crate) fn offer(response: &mut egui::Response, answer: Answer) {
    // a greyed `action_button_if` stays in an enabled Ui, sensing hover only
    if !response.enabled() || !response.sense.senses_click() {
        return;
    }
    let (id, layer) = (response.id, response.layer_id);
    let clicked = response.ctx.data_mut(|d| {
        let Some(mut current) = d.get_temp::<Current>(current_id()).filter(|c| c.layer == layer) else {
            return false;
        };
        let slot = match answer {
            Answer::Primary => &mut current.primary,
            Answer::Danger => &mut current.danger,
        };
        slot.get_or_insert(id);
        d.insert_temp(current_id(), current);
        current.click == Some(id)
    });
    if clicked {
        response.flags.insert(egui::response::Flags::FAKE_PRIMARY_CLICKED);
    }
}

/// The main answer of a dialog drawn with a plain button: Enter clicks it,
/// as it clicks a `Primary` one.
pub(crate) fn default_button(mut response: egui::Response) -> egui::Response {
    offer(&mut response, Answer::Primary);
    response
}

/// Enter with no modifier, consumed: for a dialog that does something of its
/// own with it (search, a field's commit), so Enter does not click its main
/// button too.
pub(crate) fn enter_pressed(ui: &egui::Ui) -> bool {
    ui.input_mut(|i| i.modifiers.is_none() && i.consume_key(egui::Modifiers::NONE, Key::Enter))
}

#[cfg(test)]
mod tests {
    use crate::ui::widgets::{self, ActionKind, Modal};
    use egui::{Event, Id, Key, Modifiers};

    #[derive(Clone, Copy, PartialEq)]
    enum Field {
        None,
        Single,
        Multi,
    }

    struct Dialog {
        field: Field,
        /// A `Primary` answer, else a `Danger` one.
        primary: bool,
        /// The dialog commits its field on Enter itself.
        own_enter: bool,
        /// The main button can be clicked.
        main_enabled: bool,
        text: String,
    }

    #[derive(Default)]
    struct Pass {
        main: bool,
        cancel: Option<egui::Response>,
        field: Option<Id>,
        own: bool,
    }

    fn enter() -> Event {
        Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn pass(ctx: &egui::Context, d: &mut Dialog, events: Vec<Event>) -> Pass {
        let p = crate::theme::Theme::default().palette();
        let mut out = Pass::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let _ = crate::ui::login::test_output(ctx.run_ui(input, |ui| {
            Modal::new(Id::unique("modal_keys_test")).show(ui.ctx(), |ui| {
                ui.label("Question");
                out.field = match d.field {
                    Field::None => None,
                    Field::Single => Some(ui.add(egui::TextEdit::singleline(&mut d.text)).id),
                    Field::Multi => Some(ui.add(egui::TextEdit::multiline(&mut d.text)).id),
                };
                if d.own_enter {
                    out.own = widgets::enter_pressed(ui);
                }
                ui.horizontal(|ui| {
                    let size = egui::vec2(0.0, 26.0);
                    out.cancel = Some(widgets::action_button(ui, &p, "x", "Cancel", ActionKind::Flat, size));
                    let kind = if d.primary { ActionKind::Primary } else { ActionKind::Danger };
                    out.main = widgets::action_button_if(ui, &p, "check", "OK", kind, size, d.main_enabled).clicked();
                });
            });
        }));
        out
    }

    fn opened(field: Field) -> (egui::Context, Dialog) {
        let ctx = egui::Context::default();
        let mut d = Dialog {
            field,
            primary: true,
            own_enter: false,
            main_enabled: true,
            text: "Outfit".into(),
        };
        for _ in 0..3 {
            assert!(!pass(&ctx, &mut d, vec![]).main);
        }
        (ctx, d)
    }

    #[test]
    fn the_first_field_takes_the_focus_with_its_text_selected() {
        let (ctx, mut d) = opened(Field::Single);
        let field = pass(&ctx, &mut d, vec![]).field.expect("field");
        assert_eq!(ctx.memory(|m| m.focused()), Some(field));
        let range = egui::TextEdit::load_state(&ctx, field)
            .and_then(|s| s.cursor.char_range())
            .expect("selection");
        let ends = [range.primary.index, range.secondary.index].map(|i| i.0);
        assert_eq!((ends[0].min(ends[1]), ends[0].max(ends[1])), (0, d.text.chars().count()));
    }

    #[test]
    fn enter_in_a_single_line_field_clicks_the_main_button_once() {
        let (ctx, mut d) = opened(Field::Single);
        assert!(!pass(&ctx, &mut d, vec![enter()]).main);
        assert!(pass(&ctx, &mut d, vec![]).main, "clicked on the next pass");
        assert!(!pass(&ctx, &mut d, vec![]).main, "only once");
        assert_eq!(d.text, "Outfit");
    }

    #[test]
    fn the_enter_that_opens_a_dialog_does_not_answer_it() {
        let ctx = egui::Context::default();
        let mut d = Dialog {
            field: Field::None,
            primary: true,
            own_enter: false,
            main_enabled: true,
            text: String::new(),
        };
        assert!(!pass(&ctx, &mut d, vec![enter()]).main);
        for _ in 0..3 {
            assert!(!pass(&ctx, &mut d, vec![]).main);
        }
    }

    #[test]
    fn a_greyed_main_button_is_not_clicked() {
        let (ctx, mut d) = opened(Field::Single);
        d.main_enabled = false;
        pass(&ctx, &mut d, vec![]);
        pass(&ctx, &mut d, vec![enter()]);
        assert!(!pass(&ctx, &mut d, vec![]).main);
    }

    #[test]
    fn an_enter_held_since_the_dialog_opened_does_not_answer_it() {
        let ctx = egui::Context::default();
        let mut d = Dialog {
            field: Field::None,
            primary: true,
            own_enter: false,
            main_enabled: true,
            text: String::new(),
        };
        // the press that opened it, then the key repeats (egui marks a press
        // of a key still down as a repeat)
        assert!(!pass(&ctx, &mut d, vec![enter()]).main);
        for _ in 0..4 {
            assert!(!pass(&ctx, &mut d, vec![enter()]).main);
        }
    }

    #[test]
    fn a_multi_line_field_keeps_enter_for_a_new_line() {
        let (ctx, mut d) = opened(Field::Multi);
        pass(&ctx, &mut d, vec![enter()]);
        for _ in 0..3 {
            assert!(!pass(&ctx, &mut d, vec![]).main);
        }
        assert!(d.text.contains('\n'));
    }

    #[test]
    fn without_a_primary_button_enter_clicks_the_danger_one() {
        let ctx = egui::Context::default();
        let mut d = Dialog {
            field: Field::None,
            primary: false,
            own_enter: false,
            main_enabled: true,
            text: String::new(),
        };
        for _ in 0..3 {
            pass(&ctx, &mut d, vec![]);
        }
        assert!(!pass(&ctx, &mut d, vec![enter()]).main);
        assert!(pass(&ctx, &mut d, vec![]).main);
    }

    #[test]
    fn a_focused_button_answers_enter_alone() {
        let (ctx, mut d) = opened(Field::None);
        let cancel = pass(&ctx, &mut d, vec![]).cancel.expect("cancel").id;
        ctx.memory_mut(|m| m.request_focus(cancel));
        pass(&ctx, &mut d, vec![]);
        let answer = pass(&ctx, &mut d, vec![enter()]);
        assert!(answer.cancel.expect("cancel").clicked(), "egui clicks the focused button");
        assert!(!answer.main);
        assert!(!pass(&ctx, &mut d, vec![]).main);
    }

    #[test]
    fn a_dialog_that_takes_enter_itself_gets_no_second_click() {
        let (ctx, mut d) = opened(Field::Single);
        d.own_enter = true;
        assert!(pass(&ctx, &mut d, vec![enter()]).own);
        for _ in 0..3 {
            assert!(!pass(&ctx, &mut d, vec![]).main);
        }
    }
}
