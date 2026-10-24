//! Searching a conversation: the filter of its toolbar (only the matching
//! lines stay) and the find field opened by Ctrl+F or the head's magnifier,
//! in the head's row (the lines stay, the matches are highlighted and
//! visited one after the other, like LLFloaterSearchReplace on a chat
//! history). Each conversation has its own.

use super::view::View;
use crate::theme::Palette;
use egui::{Color32, RichText};
use uuid::Uuid;

/// Height of the find bar.
pub const BAR_H: f32 = 26.0;

#[derive(Default)]
pub struct Search {
    /// Text of the toolbar's filter (None = closed).
    pub filter: Option<String>,
    pub find: Option<Find>,
}

impl Search {
    /// Open the find bar (keeping its text when it is already open) and put
    /// the keyboard in it.
    pub fn open_find(&mut self) {
        self.find.get_or_insert_with(Find::default).focus = true;
    }
}

#[derive(Default)]
pub struct Find {
    pub text: String,
    /// Which match is the current one (index among the view's hits).
    current: usize,
    /// The text changed: the current match becomes the most recent one.
    restart: bool,
    /// Put the keyboard in the field.
    focus: bool,
}

impl Find {
    /// Line of the current match.
    pub fn current_line(&self, view: &View) -> Option<usize> {
        let row = *view.hits().get(self.current)?;
        view.row(row).map(|r| r.line)
    }

    /// The matches changed (new text, new lines): keep the current one in
    /// range; a new text starts from the most recent match.
    pub fn hits_changed(&mut self, view: &mut View) {
        let n = view.hits().len();
        if self.restart {
            self.restart = false;
            self.current = n.saturating_sub(1);
            if let Some(&row) = view.hits().last() {
                view.scroll_to(row);
            }
        }
        self.current = self.current.min(n.saturating_sub(1));
    }

    /// Go to the next (older lines are before: "previous" goes up) match.
    fn step(&mut self, view: &mut View, forward: bool) {
        let n = view.hits().len();
        if n == 0 {
            return;
        }
        self.current = if forward {
            (self.current + 1) % n
        } else {
            (self.current + n - 1) % n
        };
        view.scroll_to(view.hits()[self.current]);
    }
}

/// The wash behind the matches: the palette's amber, translucent.
pub fn wash(p: &Palette) -> Color32 {
    p.amber.gamma_multiply(0.28)
}

/// Where the message of the row being drawn spans (top, and bottom when the
/// drawer knows it), below its day and "New" lines.
fn message_key() -> egui::Id {
    egui::Id::unique("conv_find_message_span")
}

/// A row's drawer says where its message starts, below the day and "New"
/// lines it drew first, and where it ends when it leaves a gap after it.
pub fn note_message(ui: &egui::Ui, top: f32, bottom: Option<f32>) {
    ui.ctx().data_mut(|d| d.insert_temp(message_key(), (top, bottom)));
}

/// Before a row is drawn: nothing noted yet.
pub fn forget_message(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<(f32, Option<f32>)>(message_key()));
}

/// Mark of the line of the current match: an amber bar along the left edge
/// of its message only (the human's report of 2026-10-16: it ran over the
/// day and "New" lines above the message and the gap under it).
pub fn current_marker(ui: &egui::Ui, p: &Palette) {
    let r = ui.min_rect();
    let (top, bottom) = ui
        .ctx()
        .data(|d| d.get_temp::<(f32, Option<f32>)>(message_key()))
        .unwrap_or((r.top(), None));
    let top = top.clamp(r.top(), r.bottom());
    let bottom = bottom.unwrap_or(r.bottom()).clamp(top, r.bottom());
    let bar = egui::Rect::from_min_max(egui::pos2(r.left(), top), egui::pos2(r.left() + 2.0, bottom));
    ui.painter().rect_filled(bar, 0.0, p.amber);
}

/// The find field of `conversation` (None: the nearby chat): each its
/// own, as a conversation in its own window (`popout.rs`) has its find
/// field beside the Conversations window's.
pub fn field_id(conversation: Option<Uuid>) -> egui::Id {
    egui::Id::unique(("conv_find_field", conversation))
}

/// The filter field of `conversation`, the same way.
pub fn filter_id(conversation: Option<Uuid>) -> egui::Id {
    egui::Id::unique(("conv_filter_field", conversation))
}

/// Ctrl+F was pressed for the window this `ui` is in: the keyboard is in
/// one of `own` fields, or it is nowhere while the pointer is over this
/// window. Another window with the keyboard (the script editor, a search
/// field) keeps its shortcut, and the viewer's own ("Search") runs
/// when the pointer is over the world or another window. The key is
/// consumed when taken. The rule itself, shared with the app, is
/// `find_shortcut::owner`.
pub fn wants_shortcut(ui: &egui::Ui, own: &[egui::Id]) -> bool {
    super::super::find_shortcut::take(ui, own)
}

/// Side of the small buttons at the field's end.
const BUTTON: f32 = 20.0;

/// The find field, laid over `rect` in the head's row in place of the
/// conversation's picture and name (the human's request of 2026-10-16: no
/// strip pushing the messages): the face of the other windows' search
/// fields (`widgets::field_frame`, here on `face`), a magnifier, the text,
/// then inside it the match count and previous / next. Enter goes to the
/// previous (older) match, Shift+Enter to the next one; Escape closes, as
/// the cross at its end when `cross` (a head without the magnifier's
/// button). True when it was closed. `conversation`: whose it is.
#[allow(clippy::too_many_arguments)]
pub fn field(
    ui: &mut egui::Ui,
    p: &Palette,
    rect: egui::Rect,
    face: Color32,
    f: &mut Find,
    view: &mut View,
    cross: bool,
    conversation: Option<Uuid>,
) -> bool {
    use super::super::widgets;
    let mut close = false;
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
    let n = view.hits().len();
    let count = if f.text.is_empty() {
        String::new()
    } else if n == 0 {
        aurora_i18n::tr!("ui-chat-find-none").to_owned()
    } else {
        aurora_i18n::tr!(
            "ui-chat-find-count",
            current = (f.current.min(n - 1) + 1).to_string(),
            total = n.to_string()
        )
    };
    let count_font = egui::FontId::proportional(12.0);
    let count_w = ui
        .painter()
        .layout_no_wrap(count.clone(), count_font.clone(), Color32::PLACEHOLDER)
        .size()
        .x;
    let gap = 4.0;
    let trailing = count_w + gap + 2.0 * BUTTON + gap + if cross { BUTTON + gap } else { 0.0 };
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(9.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui = &mut child;
    ui.spacing_mut().item_spacing.x = gap;
    widgets::glyph(ui, "search-filled", p.muted, 13.0);
    let text_w = (ui.available_width() - trailing).max(40.0);
    let edit = ui.add(
        egui::TextEdit::singleline(&mut f.text)
            .id(field_id(conversation))
            .frame(egui::Frame::NONE.inner_margin(crate::ui::widgets::field_text_margin(
                ui.ctx(),
                &egui::TextStyle::Body.resolve(ui.style()),
            )))
            .hint_text(aurora_i18n::tr!("ui-chat-find"))
            .desired_width(text_w),
    );
    if std::mem::take(&mut f.focus) {
        edit.request_focus();
    }
    if edit.changed() {
        f.restart = true;
    }
    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    let color = if n == 0 && !f.text.is_empty() { p.warn } else { p.muted };
    ui.label(RichText::new(count).font(count_font).color(color));
    let prev = widgets::icon_button(ui, p, "caret-up-filled", aurora_i18n::tr!("ui-chat-find-previous"), n > 0);
    let next = widgets::icon_button(ui, p, "caret-down-filled", aurora_i18n::tr!("ui-chat-find-next"), n > 0);
    if cross && widgets::icon_button(ui, p, "x", aurora_i18n::tr!("ui-chat-find-close"), true) {
        close = true;
    }
    if enter {
        // Enter walks up the conversation, from the latest match
        f.step(view, ui.input(|i| i.modifiers.shift));
        edit.request_focus();
    } else if prev {
        f.step(view, false);
    } else if next {
        f.step(view, true);
    }
    close
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::Frames;

    const OWN: &str = "own_field";
    const OTHER: &str = "other_field";
    /// Over the window that searches, over the other window, over the world.
    const ON_OURS: egui::Pos2 = egui::pos2(150.0, 150.0);
    const ON_OTHER: egui::Pos2 = egui::pos2(750.0, 150.0);
    const ON_WORLD: egui::Pos2 = egui::pos2(450.0, 700.0);

    /// One frame of two windows, each with a text field: ours asks for the
    /// shortcut, the other one (a script editor, a search window) looks at
    /// what is left of it. Returns (ours took it, the other still has it).
    fn frame(frames: &mut Frames, events: Vec<egui::Event>, keyboard: Option<&str>) -> (bool, bool) {
        let (mut taken, mut left) = (false, false);
        let (mut ours, mut theirs) = (String::new(), String::new());
        frames.run(events, |ui| {
            let ctx = ui.ctx().clone();
            if let Some(field) = keyboard {
                ctx.memory_mut(|m| m.request_focus(egui::Id::unique(field)));
            }
            let window = |name: &str, x: f32| egui::Area::new(egui::Id::unique(name)).fixed_pos(egui::pos2(x, 0.0));
            window("ours", 0.0).show(&ctx, |ui| {
                ui.set_min_size(egui::vec2(300.0, 300.0));
                ui.add(egui::TextEdit::singleline(&mut ours).id(egui::Id::unique(OWN)));
                taken = wants_shortcut(ui, &[egui::Id::unique(OWN)]);
            });
            window("other", 600.0).show(&ctx, |ui| {
                ui.set_min_size(egui::vec2(300.0, 300.0));
                ui.add(egui::TextEdit::singleline(&mut theirs).id(egui::Id::unique(OTHER)));
                left = ui.input(|i| i.key_pressed(egui::Key::F));
            });
        });
        (taken, left)
    }

    /// Ctrl+F pressed with the pointer on `pointer` and the keyboard in
    /// `keyboard`, once the windows are laid out.
    fn ctrl_f(pointer: egui::Pos2, keyboard: Option<&str>) -> (bool, bool) {
        let mut frames = Frames::new();
        frame(&mut frames, vec![egui::Event::PointerMoved(pointer)], keyboard);
        frame(&mut frames, Vec::new(), keyboard);
        let key = egui::Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        frame(&mut frames, vec![key], keyboard)
    }

    #[test]
    fn ctrl_f_is_taken_for_this_window_only() {
        // the keyboard nowhere: the window under the pointer
        assert_eq!(ctrl_f(ON_OURS, None), (true, false));
        assert_eq!(ctrl_f(ON_OTHER, None), (false, true), "pointer on another window");
        assert_eq!(ctrl_f(ON_WORLD, None), (false, true), "pointer on the world");
        // the keyboard in a field: the window of that field, wherever the pointer is
        assert_eq!(ctrl_f(ON_OTHER, Some(OWN)), (true, false));
        assert_eq!(ctrl_f(ON_OURS, Some(OTHER)), (false, true), "keyboard in another window");
    }
}
