//! Searching a conversation: the filter of its toolbar (only the matching
//! lines stay) and the find bar opened by Ctrl+F (the lines stay, the
//! matches are highlighted and visited one after the other, like
//! Firestorm's LLFloaterSearchReplace on a chat history). Each conversation
//! has its own.

use super::view::View;
use crate::theme::Palette;
use egui::{Color32, RichText};

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

/// Mark of the line of the current match: an amber bar along its left edge.
pub fn current_marker(ui: &egui::Ui, p: &Palette) {
    let r = ui.min_rect();
    let bar = egui::Rect::from_min_max(r.left_top(), egui::pos2(r.left() + 2.0, r.bottom()));
    ui.painter().rect_filled(bar, 0.0, p.amber);
}

pub fn field_id() -> egui::Id {
    egui::Id::new("conv_find_field")
}

pub fn filter_id() -> egui::Id {
    egui::Id::new("conv_filter_field")
}

/// Ctrl+F was pressed for the window this `ui` is in: the keyboard is in
/// one of `own` fields, or it is nowhere while the pointer is over this
/// window. Another window with the keyboard (the script editor, a search
/// field) keeps its shortcut, and so does whatever Ctrl+F does when the
/// pointer is over the world or another window. The key is consumed when
/// taken.
pub fn wants_shortcut(ui: &egui::Ui, own: &[egui::Id]) -> bool {
    let ctx = ui.ctx();
    let ours = match ctx.memory(|m| m.focused()) {
        Some(id) => own.contains(&id),
        None => ctx.pointer_latest_pos().and_then(|pos| ctx.layer_id_at(pos)) == Some(ui.layer_id()),
    };
    ours && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
}

/// The find bar: field, previous / next, "3 / 17", close. Enter goes to the
/// next match (Shift+Enter to the previous), Escape closes. True when it
/// was closed.
pub fn bar(ui: &mut egui::Ui, p: &Palette, f: &mut Find, view: &mut View) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.set_height(BAR_H);
        ui.spacing_mut().item_spacing.x = 4.0;
        let field = ui.add(
            egui::TextEdit::singleline(&mut f.text)
                .id(field_id())
                .hint_text(aurora_i18n::tr!("ui-chat-find"))
                .desired_width((ui.available_width() - 190.0).clamp(80.0, 260.0)),
        );
        if std::mem::take(&mut f.focus) {
            field.request_focus();
        }
        if field.changed() {
            f.restart = true;
        }
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        let n = view.hits().len();
        let prev = super::super::widgets::icon_button(ui, p, "caret-up-filled", aurora_i18n::tr!("ui-chat-find-previous"), n > 0);
        let next = super::super::widgets::icon_button(ui, p, "caret-down-filled", aurora_i18n::tr!("ui-chat-find-next"), n > 0);
        if enter {
            // Enter walks up the conversation, from the latest match
            f.step(view, ui.input(|i| i.modifiers.shift));
            field.request_focus();
        } else if prev {
            f.step(view, false);
        } else if next {
            f.step(view, true);
        }
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
        let color = if n == 0 && !f.text.is_empty() { p.warn } else { p.muted };
        ui.label(RichText::new(count).size(12.0).color(color));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if super::super::widgets::icon_button(ui, p, "x", aurora_i18n::tr!("ui-chat-find-close"), true) {
                close = true;
            }
        });
    });
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
                ctx.memory_mut(|m| m.request_focus(egui::Id::new(field)));
            }
            let window = |name: &str, x: f32| egui::Area::new(egui::Id::new(name)).fixed_pos(egui::pos2(x, 0.0));
            window("ours", 0.0).show(&ctx, |ui| {
                ui.set_min_size(egui::vec2(300.0, 300.0));
                ui.add(egui::TextEdit::singleline(&mut ours).id(egui::Id::new(OWN)));
                taken = wants_shortcut(ui, &[egui::Id::new(OWN)]);
            });
            window("other", 600.0).show(&ctx, |ui| {
                ui.set_min_size(egui::vec2(300.0, 300.0));
                ui.add(egui::TextEdit::singleline(&mut theirs).id(egui::Id::new(OTHER)));
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
