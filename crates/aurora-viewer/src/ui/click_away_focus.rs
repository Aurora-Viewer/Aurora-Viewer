//! A click elsewhere takes the focus from a widget after the pass, as egui
//! 0.36 did, not before the widget has run.
//!
//! egui 0.37's `TextEdit` reads its own response for its style
//! (`Ui::widget_style`) before it handles its events, and that read applies
//! "clicked elsewhere: surrender the focus". A field then loses the focus,
//! and with it its selection, before the click that took it has run: the
//! field menu's Cut and Copy, or the notecard toolbar's Bold, find no
//! selection any more. With egui 0.36 the field surrendered when it
//! allocated its rectangle, after its events, so the widget clicked still
//! saw the selection, and a focus requested back for the field (the menu
//! entries, the toolbar) kept it.
//!
//! This plugin turns egui's surrender off during the pass and applies it at
//! its end: the widget focused when the click came loses the focus, unless
//! it is under the pointer or something asked for a focus during the pass
//! (`Memory::request_focus`, whose only trace outside egui is the IME
//! interrupt it requests). To be removed once egui is fixed.

use egui::output::IMEOutput;
use egui::{Context, FullOutput, IMEPurpose, Id, Rect, SurrenderFocusOn, Ui};

/// Installs the plugin on `ctx` (once: egui keeps one plugin per type).
pub fn install(ctx: &Context) {
    ctx.add_plugin(ClickAwayFocus::default());
}

#[derive(Default)]
struct ClickAwayFocus {
    /// What surrenders the focus, as the options asked before the plugin
    /// turned it off.
    on: SurrenderFocusOn,
    /// The focused widget a click elsewhere takes the focus from, decided at
    /// the end of the pass.
    surrender: Option<Id>,
    /// The IME output put in so that egui reports a focus request of the
    /// pass, to take out again.
    probe: bool,
}

impl egui::Plugin for ClickAwayFocus {
    fn debug_name(&self) -> &'static str {
        "click_away_focus"
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        ui.ctx().options_mut(|o| {
            let on = &mut o.input_options.surrender_focus_on;
            if *on != SurrenderFocusOn::Never {
                self.on = *on;
                *on = SurrenderFocusOn::Never;
            }
        });
        self.surrender = None;
        self.probe = false;
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx();
        let clicked = ctx.input(|i| match self.on {
            SurrenderFocusOn::Presses => i.pointer.any_pressed(),
            SurrenderFocusOn::Clicks => i.pointer.any_click(),
            SurrenderFocusOn::Never => false,
        });
        let Some(focused) = ctx.memory(|m| m.focused()).filter(|_| clicked) else {
            return;
        };
        // (the surrender being off, this read takes nothing)
        if ctx.read_response(focused).is_some_and(|r| r.hovered()) {
            return;
        }
        self.surrender = Some(focused);
        // egui only reports the focus requests of the pass in an IME output
        ctx.output_mut(|o| {
            if o.ime.is_none() {
                o.ime = Some(IMEOutput {
                    purpose: IMEPurpose::Normal,
                    rect: Rect::NOTHING,
                    cursor_rect: Rect::NOTHING,
                    should_interrupt_composition: false,
                });
                self.probe = true;
            }
        });
    }

    fn output_hook(&mut self, ctx: &Context, output: &mut FullOutput) {
        let Some(id) = self.surrender.take() else {
            return;
        };
        let requested = output
            .platform_output
            .ime
            .as_ref()
            .is_some_and(|ime| ime.should_interrupt_composition);
        if std::mem::take(&mut self.probe) {
            output.platform_output.ime = None;
        }
        if !requested {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::text::{CCursor, CCursorRange};
    use egui::{Context, Event, Id, PointerButton, Pos2, RawInput, Rect, TextEdit, pos2, vec2};

    const BUTTON: Pos2 = pos2(30.0, 130.0);

    /// A field over a button; the button cuts the field's selection when
    /// `cut`, as the field menu and the notecard toolbar do.
    struct Bench {
        ctx: Context,
        field: Id,
        text: String,
        cut: bool,
        selection_at_click: Option<(usize, usize)>,
    }

    impl Bench {
        fn new(cut: bool) -> Self {
            let ctx = Context::default();
            super::install(&ctx);
            let mut bench = Bench {
                ctx,
                field: Id::unique("click_away_field"),
                text: "hello world".into(),
                cut,
                selection_at_click: None,
            };
            bench.frame(Vec::new());
            bench.ctx.memory_mut(|m| m.request_focus(bench.field));
            bench.frame(Vec::new());
            let mut state = TextEdit::load_state(&bench.ctx, bench.field).expect("field");
            state
                .cursor
                .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(11))));
            state.store(&bench.ctx, bench.field);
            bench.frame(Vec::new());
            bench
        }

        fn frame(&mut self, events: Vec<Event>) {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
                events,
                ..Default::default()
            };
            let (field, cut) = (self.field, self.cut);
            let (text, selection_at_click) = (&mut self.text, &mut self.selection_at_click);
            let mut out = self.ctx.run_ui(input, |ui| {
                ui.add(TextEdit::singleline(text).id(field));
                ui.add_space(100.0);
                if ui.button("Cut").clicked() {
                    let range = TextEdit::load_state(ui.ctx(), field).and_then(|s| s.cursor.char_range());
                    *selection_at_click = range.map(|r| (r.secondary.index.0, r.primary.index.0));
                    if cut {
                        ui.ctx().memory_mut(|m| m.request_focus(field));
                    }
                }
            });
            out.textures_delta.clear();
        }

        fn click(&mut self, at: Pos2) {
            let press = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![Event::PointerMoved(at), press(true)]);
            self.frame(vec![press(false)]);
        }

        fn focused(&self) -> bool {
            self.ctx.memory(|m| m.has_focus(self.field))
        }

        fn selection(&self) -> Option<(usize, usize)> {
            let range = TextEdit::load_state(&self.ctx, self.field).and_then(|s| s.cursor.char_range());
            range.map(|r| (r.secondary.index.0, r.primary.index.0))
        }
    }

    /// The button clicked away from the field still sees its selection, and
    /// the field keeps its focus and selection when the button asks for it.
    #[test]
    fn a_button_clicked_away_sees_the_selection_and_can_give_the_focus_back() {
        let mut b = Bench::new(true);
        b.click(BUTTON);
        assert_eq!(b.selection_at_click, Some((0, 11)), "the selection when the button ran");
        assert!(b.focused(), "the button gave the focus back");
        b.frame(Vec::new());
        assert!(b.focused());
        assert_eq!(b.selection(), Some((0, 11)), "the selection kept");
    }

    /// Otherwise a click elsewhere still takes the focus, after the pass,
    /// and a click on the field keeps it.
    #[test]
    fn a_click_elsewhere_still_takes_the_focus() {
        let mut b = Bench::new(false);
        b.click(BUTTON);
        assert_eq!(b.selection_at_click, Some((0, 11)));
        assert!(!b.focused(), "the click away took the focus");
        b.frame(Vec::new());
        assert!(!b.focused());
        let mut b = Bench::new(false);
        let field = b.ctx.read_response(b.field).expect("field").rect.center();
        b.click(field);
        assert!(b.focused(), "a click on the field keeps it");
        // egui's own surrender stays off
        assert_eq!(b.ctx.options(|o| o.input_options.surrender_focus_on), egui::SurrenderFocusOn::Never);
    }
}
