//! LLLineEditor prevalidation (indra/llui/lllineeditor.cpp): reject the
//! attempted insertion, including an entire paste, without deleting its
//! selection. The same buffer handles committed IME and accessibility edits.
use crate::text_validation::{Policy, reason};
use egui::{
    TextBuffer,
    text::{CCursor, CCursorRange, CharIndex},
};
use std::ops::Range;

pub fn optional(
    ui: &mut egui::Ui,
    editable: bool,
    text: &mut String,
    policy: Option<Policy>,
    make: impl for<'a> FnOnce(&'a mut dyn TextBuffer, &egui::Ui) -> egui::TextEdit<'a>,
) -> egui::Response {
    match policy {
        Some(policy) => field(ui, editable, text, policy, make),
        None => super::text_field(ui, editable, text, make),
    }
}

struct Buffer<'a> {
    text: &'a mut String,
    policy: Policy,
    before: Option<String>,
    deleted: Option<(String, Range<CharIndex>)>,
    rejected: Option<Range<CharIndex>>,
}

impl Buffer<'_> {
    fn remember(&mut self) {
        if self.before.is_none() {
            self.before = Some(self.text.clone());
        }
    }
}

impl TextBuffer for Buffer<'_> {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Buffer<'static>>()
    }
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        self.text
    }
    fn insert_text(&mut self, text: &str, at: CharIndex) -> usize {
        self.remember();
        if !self.policy.accepts(text) {
            let range = if let Some((before, range)) = self.deleted.take() {
                *self.text = before;
                range
            } else {
                at..at
            };
            self.rejected = Some(range);
            return 0;
        }
        self.deleted = None;
        self.text.insert_text(text, at)
    }
    fn delete_char_range(&mut self, range: Range<CharIndex>) {
        self.remember();
        self.deleted = Some((self.text.clone(), range.clone()));
        self.text.delete_char_range(range);
    }
    // Undo/redo restores a previously accepted value. Existing grid text
    // remains readable and undoable even when it predates the restriction.
    fn replace_with(&mut self, text: &str) {
        self.remember();
        self.text.replace_with(text);
    }
    fn insert_text_at(&mut self, cursor: &mut CCursor, text: &str, limit: usize) {
        // Validate the whole operation before egui truncates it at char_limit:
        // a forbidden character beyond that limit must not turn a paste into
        // a silently shortened, apparently successful name.
        if !self.policy.accepts(text) {
            self.insert_text(text, cursor.index);
            return;
        }
        let cutoff = limit.saturating_sub(self.text.chars().count());
        let text = text.char_indices().nth(cutoff).map_or(text, |(i, _)| &text[..i]);
        cursor.index += self.insert_text(text, cursor.index);
    }
}

#[derive(Clone)]
pub(super) struct Feedback {
    until: f64,
    policy: Policy,
}

pub fn field(
    ui: &mut egui::Ui,
    editable: bool,
    text: &mut String,
    policy: Policy,
    make: impl for<'a> FnOnce(&'a mut dyn TextBuffer, &egui::Ui) -> egui::TextEdit<'a>,
) -> egui::Response {
    show(ui, editable, text, policy, make).response.response
}

pub fn show(
    ui: &mut egui::Ui,
    editable: bool,
    text: &mut String,
    policy: Policy,
    make: impl for<'a> FnOnce(&'a mut dyn TextBuffer, &egui::Ui) -> egui::TextEdit<'a>,
) -> egui::text_edit::TextEditOutput {
    if !editable {
        let mut shown: &str = text;
        let mut output = make(&mut shown, ui).text_color(ui.visuals().weak_text_color()).show(ui);
        output.response.flags.remove(egui::response::Flags::CHANGED);
        return output;
    }
    let id_key = ui.next_auto_id().with("restricted_text_id");
    let previous_id = ui.ctx().data(|d| d.get_temp::<egui::Id>(id_key));
    let (preedit, invalid_commit, invalid_insertion) = ui.input(|i| {
        let commit = i.events.iter().any(|e| matches!(e, egui::Event::Ime(egui::ImeEvent::Commit(_))));
        (
            // Wayland may send an empty preedit alongside ordinary typing
            // when its IME cursor area changes. Like TextEdit's empty-event
            // guard, do not roll back that typing as provisional composition.
            !commit
                && i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) if !text.is_empty())),
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Ime(egui::ImeEvent::Commit(text)) if !policy.accepts(text))),
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Text(text) | egui::Event::Paste(text) if !policy.accepts(text))),
        )
    });
    // An idle field needs no rollback copy. In particular, do not copy the
    // chat editor's undo history once for every visible restricted field.
    let state_before = if preedit || invalid_commit || invalid_insertion {
        previous_id
            .filter(|id| ui.memory(|m| m.has_focus(*id)))
            .and_then(|id| egui::TextEdit::load_state(ui.ctx(), id).map(|state| (id, state.clone(), state.undoer())))
    } else {
        None
    };
    let mut buffer = Buffer {
        text,
        policy,
        before: None,
        deleted: None,
        rejected: None,
    };
    let mut output = make(&mut buffer, ui).show(ui);
    ui.ctx().data_mut(|d| d.insert_temp(id_key, output.response.id));
    // The OS may compose ASCII romaji before offering a Japanese candidate.
    // Provisional text must never become the model's fallback on rejection.
    // Retain the original selection until an allowed composition commits.
    if (preedit || invalid_commit)
        && let Some((id, state, undoer)) = state_before.as_ref()
        && *id == output.response.id
    {
        if let Some(before) = buffer.before.take() {
            *buffer.text = before;
        }
        output.state = state.clone();
        output.state.set_undoer(undoer.clone());
        output.cursor_range = output.state.cursor.char_range();
        output.state.clone().store(ui.ctx(), *id);
        output.response.flags.remove(egui::response::Flags::CHANGED);
        if invalid_commit {
            reject(&output.response, policy);
        }
        feedback(&output.response);
        ui.ctx().request_repaint();
        return output;
    }
    if let Some(range) = buffer.rejected {
        output
            .state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(range.start.0), CCursor::new(range.end.0))));
        output.state.clone().store(ui.ctx(), output.response.id);
        if buffer.before.as_ref().is_some_and(|before| buffer.text.as_str() == before.as_str()) {
            output.response.flags.remove(egui::response::Flags::CHANGED);
            // egui records cursor as well as text in its undo checkpoints.
            // Restoring text alone leaves a spurious undo step for the
            // temporarily deleted selection of a rejected paste.
            if let Some((id, _, undoer)) = state_before
                && id == output.response.id
            {
                output.state.set_undoer(undoer);
                output.state.clone().store(ui.ctx(), id);
            }
        }
        // Composition is provisional. Reject its characters without a stream
        // of warnings; the committed input produces the actionable feedback.
        if !preedit {
            reject(&output.response, policy);
        }
        ui.ctx().request_repaint();
    }
    feedback(&output.response);
    output
}

pub fn reject(response: &egui::Response, policy: Policy) {
    // A paste/IME commit and Enter can arrive in one input batch. Do not let
    // the modal submit the old value after rejecting its replacement.
    let enter = response.ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    if enter {
        response.request_focus();
    }
    let until = response.ctx.input(|i| i.time) + 4.0;
    response
        .ctx
        .data_mut(|d| d.insert_temp(response.id.with("invalid_input"), Feedback { until, policy }));
}

fn feedback(response: &egui::Response) {
    let id = response.id.with("invalid_input");
    let now = response.ctx.input(|i| i.time);
    let info = response.ctx.data(|d| d.get_temp::<Feedback>(id));
    if let Some(info) = info {
        if now < info.until && response.has_focus() {
            egui::Tooltip::for_widget(response).show(|ui| {
                ui.strong(aurora_i18n::tr!("input-restricted-title"));
                ui.label(reason(info.policy));
            });
            response.ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else {
            response.ctx.data_mut(|d| d.remove::<Feedback>(id));
        }
    }
}

#[derive(Clone, Copy)]
struct NumericBounds {
    rect: egui::Rect,
    layer: egui::LayerId,
}

/// A numeric editor owns its internal text buffer. Filter rejected input
/// events before it sees them, retaining the native selection and undo state.
/// Only opt in controls whose reference has a character validator; expression
/// editors (LLSpinCtrl / LLCalc) keep their broader input language.
pub fn number(widget: impl egui::Widget, policy: Policy) -> impl egui::Widget {
    struct Number<W> {
        widget: W,
        policy: Policy,
    }
    impl<W: egui::Widget> egui::Widget for Number<W> {
        fn ui(self, ui: &mut egui::Ui) -> egui::Response {
            let key = ui.next_auto_id().with("restricted_number_bounds");
            let previous = ui.ctx().data(|d| d.get_temp::<NumericBounds>(key));
            let focused = ui.memory(|m| m.focused()).and_then(|id| ui.ctx().read_response(id));
            let owns_focus = previous
                .zip(focused.as_ref())
                .is_some_and(|(bounds, focus)| bounds.layer == focus.layer_id && bounds.rect.contains(focus.rect.center()));
            let mut rejected = false;
            if owns_focus {
                ui.input_mut(|i| {
                    // A click/Tab may transfer focus to an unrestricted field
                    // in this same input batch. Let its own editor handle it.
                    let moving_focus = i.pointer.any_pressed() || i.key_pressed(egui::Key::Tab);
                    if !moving_focus {
                        i.events.retain(|event| {
                            let (text, preedit) = match event {
                                egui::Event::Text(text) | egui::Event::Paste(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                                    (text, false)
                                }
                                egui::Event::Ime(egui::ImeEvent::Preedit { .. }) => return false,
                                _ => return true,
                            };
                            let accepted = self.policy.accepts(text)
                                && (self.policy != Policy::Decimal
                                    || !text.contains(',')
                                    || aurora_i18n::text("number-decimal-separator") == ",");
                            rejected |= !accepted && !preedit;
                            accepted
                        });
                    }
                });
            }
            let response = self.widget.ui(ui);
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    key,
                    NumericBounds {
                        rect: response.rect,
                        layer: response.layer_id,
                    },
                )
            });
            if owns_focus && let Some(focused) = focused {
                if rejected {
                    reject(&focused, self.policy);
                }
                feedback(&focused);
            }
            response
        }
    }
    Number { widget, policy }
}
