//! A text field that cannot be typed in without the right to, but whose text
//! still selects and copies. egui's disabled fields (`add_enabled(false, …)`)
//! take no pointer at all: the name of a no-modify object, a parcel's media
//! address or a gesture's trigger could not even be copied to be typed
//! elsewhere. The Legacy Viewer's read-only line editors keep their text
//! selectable (LLLineEditor::setReadOnly).

/// `make` builds the field around the buffer it is given, the same way in
/// both cases; it reads the `ui` it is given for widths. With `editable`, the
/// field edits `text`; without, it shows `text` read-only in the theme's weak
/// text colour: the pointer selects it and Ctrl+C copies it, nothing typed
/// reaches it, and the response never reports a change.
pub fn text_field(
    ui: &mut egui::Ui,
    editable: bool,
    text: &mut String,
    make: impl for<'a> FnOnce(&'a mut dyn egui::TextBuffer, &egui::Ui) -> egui::TextEdit<'a>,
) -> egui::Response {
    if editable {
        let edit = make(text, ui);
        return ui.add(edit);
    }
    let mut shown: &str = text.as_str();
    let edit = make(&mut shown, ui).text_color(ui.visuals().weak_text_color());
    let mut response = ui.add(edit);
    // egui still reports a change when keys are pressed in a focused
    // read-only field; nothing changed, and callers save on a change
    response.flags.remove(egui::response::Flags::CHANGED);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, text: &mut String, editable: bool) -> egui::Response {
        let mut out = None;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            out = Some(text_field(ui, editable, text, |b, _| {
                egui::TextEdit::singleline(b).id_salt("field").desired_width(300.0)
            }));
        });
        output.textures_delta.clear();
        out.expect("drawn")
    }

    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        vec![egui::Event::PointerMoved(at), press(true), press(false)]
    }

    #[test]
    fn a_read_only_field_takes_the_pointer_but_never_the_keyboard() {
        let ctx = egui::Context::default();
        let mut text = String::from("No modify object");
        let r = frame(&ctx, vec![], &mut text, false);
        // a disabled field would sense nothing: this one can be selected
        assert!(r.sense.senses_click() || r.sense.senses_drag(), "the text can be selected");
        let at = r.rect.center();
        // focused (clicked into), it still takes no typing
        frame(&ctx, click(at), &mut text, false);
        ctx.memory_mut(|m| m.request_focus(r.id));
        frame(&ctx, vec![], &mut text, false);
        let r = frame(&ctx, vec![egui::Event::Text("typed".into())], &mut text, false);
        assert!(!r.changed());
        assert_eq!(text, "No modify object");
        // editable, the same field takes what is typed
        let r = frame(&ctx, vec![], &mut text, true);
        ctx.memory_mut(|m| m.request_focus(r.id));
        frame(&ctx, vec![], &mut text, true);
        frame(&ctx, vec![egui::Event::Text("!".into())], &mut text, true);
        assert!(text.contains('!'), "{text}");
    }
}
