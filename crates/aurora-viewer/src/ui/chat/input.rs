//! Paragraph entry shared by conversations and nearby chat. Follow
//! FSFloaterIM::handleKeyHere and FSNearbyChatControl::handleKeyHere
//! (indra/newview): Ctrl+Shift+Enter inserts a line break, while the ordinary
//! send/shout/whisper keys retain their actions. LLChatEntry::expandText
//! (indra/llui/llchatentry.cpp) bounds expansion before scrolling.

use crate::ui::emoji::Emoji;
use egui::{Context, Id, TextEdit, Ui};

const MAX_ROWS: f32 = 4.0;

/// Before completion reads the keyboard: route an explicit paragraph break
/// through the editor's native paste handling (selection replacement/undo),
/// and consume its Enter so the send path cannot also handle it.
pub(super) fn prepare(ui: &Ui, field: Id, local: bool) {
    if !ui.memory(|m| m.has_focus(field)) {
        return;
    }
    ui.input_mut(|i| {
        i.events.retain_mut(|event| {
            match event {
                // Whisper belongs only to nearby chat. Consume this chord
                // in IMs/sessions before completion can accept it either.
                egui::Event::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    modifiers,
                    ..
                } if !local && modifiers.shift && !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd => return false,
                egui::Event::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.ctrl && modifiers.shift && !modifiers.alt && !modifiers.mac_cmd => {
                    *event = egui::Event::Paste("\n".into());
                }
                egui::Event::Paste(text) if text.contains('\r') => {
                    *text = text.replace("\r\n", "\n").replace('\r', "\n");
                }
                _ => {}
            }
            true
        });
    });
}

/// The previous layout's bounded editor height. Parents reserve this space
/// before laying out the transcript or the bottom bar, so typing cannot
/// grow the containing window or cover its neighbouring controls.
pub(crate) fn height(ctx: &Context, field: Id, minimum: f32) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(field.with("paragraph_height")))
        .unwrap_or(minimum)
        .max(minimum)
}

/// A wrapping editor with at most four visible rows. Its full galley feeds
/// selection/completion while its response and painting stay in the viewport.
/// Call `prepare` before the field's completion/history handlers.
pub(super) fn show(
    ui: &mut Ui,
    emoji: Option<&mut Emoji>,
    field: Id,
    text: &mut String,
    width: f32,
    minimum: f32,
    build: impl FnOnce(TextEdit<'_>) -> TextEdit<'_>,
) -> egui::text_edit::TextEditOutput {
    let previous = height(ui.ctx(), field, minimum);
    let scroll = ui
        .allocate_ui_with_layout(egui::vec2(width, previous), egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(field.with("paragraph_scroll"))
                .auto_shrink([false, true])
                .min_scrolled_height(minimum)
                .max_height(previous)
                .show(ui, |ui| {
                    let width = ui.available_width();
                    match emoji {
                        Some(emoji) => emoji.text_field(ui, text, |edit| editor(build(edit), field, width, minimum)),
                        None => editor(build(TextEdit::multiline(text)), field, width, minimum).show(ui),
                    }
                })
        })
        .inner;
    let mut output = scroll.inner;
    let content_height = output.response.rect.height();
    let row_height = output.galley.rows.first().map_or(minimum, |r| r.row.size.y);
    let padding = (content_height - output.galley.size().y).max(0.0);
    let maximum = (MAX_ROWS * row_height + padding).max(minimum);
    let next = content_height.clamp(minimum, maximum);
    if !ui.is_sizing_pass() && (next - previous).abs() > 0.5 {
        ui.ctx().data_mut(|d| d.insert_temp(field.with("paragraph_height"), next));
        // The bounded viewport never overflows this pass. Reserve its new
        // height on the next frame without replaying the edit's input events.
        ui.ctx().request_repaint();
    }
    output.response.response.rect = output.response.rect.intersect(scroll.inner_rect);
    output.text_clip_rect = output.text_clip_rect.intersect(scroll.inner_rect);
    output
}

fn editor(edit: TextEdit<'_>, field: Id, width: f32, minimum: f32) -> TextEdit<'_> {
    edit.id(field)
        .desired_width(width)
        .min_size(egui::vec2(0.0, minimum))
        .desired_rows(1)
        .return_key(None)
        .align(egui::Align2::LEFT_CENTER)
}
