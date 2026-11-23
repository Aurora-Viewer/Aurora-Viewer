//! Read-only notice text: the transcript's selection and its rich links,
//! scoped to one document. LLTextEditor::handleRightMouseDown keeps the
//! selected text and offers Copy (indra/llui/lltexteditor.cpp, LGPL 2.1).

use super::{Selection, hit, row, take_drawn, word_at};
use crate::theme::Palette;
use crate::ui::menu;
use egui::{Key, Modifiers, PointerButton, Sense};

pub(crate) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    salt: impl std::hash::Hash + std::fmt::Debug,
    text: &str,
    mut draw: impl FnMut(&mut egui::Ui, &str),
) -> egui::Response {
    // A changed notice starts a new selection, independent of other cards
    // and of a conversation that is being laid out in this same pass.
    let id = ui.make_persistent_id((salt, text));
    let mut selection = ui.data_mut(|d| d.get_temp::<Selection>(id)).unwrap_or_default();
    let previous = take_drawn();
    let response = ui
        .scope_builder(
            egui::UiBuilder::new()
                .scope_id(id)
                .sense(Sense::click_and_drag())
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                for (seq, line) in text.split('\n').enumerate() {
                    ui.scope(|ui| {
                        row(ui, seq as u64, selection.of_row(seq as u64), true, |ui| {
                            if line.is_empty() {
                                ui.allocate_space(egui::vec2(1.0, 6.0));
                            } else {
                                draw(ui, line);
                            }
                        });
                    });
                }
            },
        )
        .response;
    let drawn = take_drawn();
    super::DRAWN.with_borrow_mut(|d| *d = previous);
    let area = response.rect.intersect(ui.clip_rect());
    let origin = ui.input(|i| i.pointer.press_origin()).filter(|pos| area.contains(*pos));
    // Linked text owns a child click response. Take the document's focus
    // after that click too, so Ctrl+A/C applies to its whole body.
    if ui.input(|i| i.pointer.primary_pressed() || i.pointer.primary_clicked())
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|pos| area.contains(pos) && ui.ctx().layer_id_at(pos) == Some(response.layer_id))
    {
        response.request_focus();
    }
    if response.drag_started_by(PointerButton::Primary)
        && let Some((point, text)) = origin.and_then(|pos| hit(&drawn, pos))
    {
        selection.start_at(point, text);
    }
    if response.dragged_by(PointerButton::Primary)
        && origin.is_some()
        && let Some((point, text)) = response.interact_pointer_pos().and_then(|pos| hit(&drawn, pos))
    {
        selection.extend_to(point, text);
    }
    if response.clicked() {
        selection.clear();
    }
    if response.double_clicked()
        && let Some((seq, range, text)) = response.interact_pointer_pos().and_then(|pos| word_at(&drawn, pos))
    {
        selection.select_in_row(seq, range, text);
    }
    let select_all = |selection: &mut Selection| {
        if let (Some(first), Some(last)) = (drawn.first(), drawn.last()) {
            selection.select_all(first.seq, last.seq);
        }
    };
    if response.has_focus() {
        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) {
            select_all(&mut selection);
        }
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            selection.clear();
        }
    }
    let copy =
        |selection: &Selection| selection.copy_text(drawn.iter().map(|r| r.seq), |seq| (String::new(), drawn[seq as usize].text.clone()));
    if response.has_focus() && ui.input(|i| i.events.contains(&egui::Event::Copy)) && !selection.is_empty() {
        ui.ctx().copy_text(copy(&selection));
    }
    // A selected link gets the text menu, as in LLTextEditor. A link with
    // no selection retains its own menu and activation.
    let selected_click = ui
        .input(|i| i.pointer.secondary_clicked().then(|| i.pointer.interact_pos()).flatten())
        .filter(|pos| area.contains(*pos) && ui.ctx().layer_id_at(*pos) == Some(response.layer_id))
        .and_then(|pos| hit(&drawn, pos))
        .zip(selection.bounds())
        .is_some_and(|((point, _), (start, end))| start <= point && point <= end);
    menu::context_menu_also(&response, p, selected_click, |ui| {
        if menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("common-copy-2"), !selection.is_empty()) {
            ui.ctx().copy_text(copy(&selection));
            response.request_focus();
        }
        if menu::item(ui, p, "select-all", aurora_i18n::tr!("common-select-all")) {
            select_all(&mut selection);
            response.request_focus();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, selection));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, Rect};

    const TEXT: &str = "An announcement with émoji\n\nA second line to copy.";

    struct Notice {
        ctx: egui::Context,
        p: Palette,
        rect: Rect,
        selection: Selection,
        time: f64,
    }

    impl Notice {
        fn new() -> Self {
            let theme = crate::theme::Theme::default();
            let ctx = egui::Context::default();
            theme.apply(&ctx, 1.0);
            let mut test = Self {
                ctx,
                p: theme.palette(),
                rect: Rect::NOTHING,
                selection: Selection::default(),
                time: 0.0,
            };
            for _ in 0..3 {
                test.frame(Vec::new());
            }
            test
        }

        fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
            self.time += 0.1;
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let id = ui.make_persistent_id(("notice", TEXT));
                    self.rect = show(ui, &self.p, "notice", TEXT, |ui, line| {
                        // The cards' sender link and the plain text both
                        // participate in the same character selection.
                        crate::ui::notifications::text::show(
                            ui,
                            &self.p,
                            line,
                            &[crate::world::notifications::NameLink {
                                name: "announcement".into(),
                                target: crate::world::notifications::NameTarget::Avatar(uuid::Uuid::from_u128(3811)),
                            }],
                            14.0,
                            self.p.ink,
                            false,
                            true,
                            crate::world::names::NameScope::Roster,
                        );
                    })
                    .rect;
                    self.selection = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
                },
            );
            output.textures_delta.clear();
            output
        }

        fn pointer(&mut self, pos: Pos2, button: PointerButton, pressed: bool) -> egui::FullOutput {
            self.frame(vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ])
        }
    }

    fn text_rect(output: &egui::FullOutput, label: &str) -> Option<Rect> {
        fn find(shape: &egui::Shape, label: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(t.galley.rect.translate(t.pos.to_vec2())),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
                _ => None,
            }
        }
        output.shapes.iter().find_map(|s| find(&s.shape, label))
    }

    #[test]
    fn notice_drag_selection_survives_right_press_and_copy_menu() {
        let mut notice = Notice::new();
        let start = notice.rect.left_top() + egui::vec2(2.0, 5.0);
        let end = notice.rect.left_top() + egui::vec2(130.0, 5.0);
        notice.pointer(start, PointerButton::Primary, true);
        notice.frame(vec![Event::PointerMoved(end)]);
        notice.pointer(end, PointerButton::Primary, false);
        let selected = notice.selection.bounds();
        assert!(selected.is_some(), "drag selects text");
        let before = notice.frame(vec![Event::Copy]).platform_output.commands;
        let copied = before
            .iter()
            .find_map(|c| match c {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .expect("keyboard copy");
        let at = start + egui::vec2(35.0, 0.0);
        notice.pointer(at, PointerButton::Secondary, true);
        assert_eq!(notice.selection.bounds(), selected, "right press keeps selection");
        notice.pointer(at, PointerButton::Secondary, false);
        notice.frame(Vec::new());
        let output = notice.frame(Vec::new());
        let copy = text_rect(&output, aurora_i18n::tr!("common-copy-2")).expect("Copy menu").center();
        notice.pointer(copy, PointerButton::Primary, true);
        let output = notice.pointer(copy, PointerButton::Primary, false);
        assert_eq!(notice.selection.bounds(), selected, "menu click keeps selection");
        assert!(
            output
                .platform_output
                .commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(text) if *text == copied))
        );
    }

    #[test]
    fn notice_select_all_copies_paragraphs_and_unicode() {
        let mut notice = Notice::new();
        let at = notice.rect.left_top() + egui::vec2(3.0, 5.0);
        notice.pointer(at, PointerButton::Primary, true);
        notice.pointer(at, PointerButton::Primary, false);
        notice.frame(vec![Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }]);
        let output = notice.frame(vec![Event::Copy]);
        assert!(
            output
                .platform_output
                .commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(text) if text == TEXT)),
            "selection {:?}, focus {:?}, output {:?}",
            notice.selection.bounds(),
            notice.ctx.memory(|m| m.focused()),
            output.platform_output.commands
        );
    }
}
