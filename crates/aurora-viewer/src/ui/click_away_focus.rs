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
//!
//! A window of the interface outside the viewer's (outside.rs) has a pass
//! of its own, inside the viewer's, and egui runs the output hook once that
//! pass is over: the focus `Memory` gives then is the viewer's own
//! window's. What such a window surrenders is taken when its next pass
//! begins, before any of its widgets runs. And egui forgets the focus
//! requests of the viewer's own pass when that window's pass begins inside
//! it: they are read just before (`focus_asked`), so that a focus given
//! back to a field of the viewer's window is kept with a window outside
//! drawn later in the frame.

use egui::output::IMEOutput;
use egui::{Context, FullOutput, IMEPurpose, Id, RawInput, Rect, SurrenderFocusOn, Ui, ViewportId};

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
    /// The window whose pass decided `surrender`.
    window: ViewportId,
    /// What the windows outside the viewer's surrender, each at its next
    /// pass.
    later: Vec<(ViewportId, Id)>,
    /// Something asked for a focus in the viewer's own pass before the pass
    /// of a window outside began in it.
    asked_before: bool,
}

/// Whether something asked for a focus since the pass in progress began.
/// egui keeps it in a flag of its `Memory` that it only shows in the output
/// of a pass that ends, and forgets when any pass begins: its `Debug` is
/// the one way to read it in between. A test fails if egui renames it.
fn focus_asked(ctx: &Context) -> bool {
    ctx.memory(|m| format!("{m:?}")).contains("requested_interrupt_ime: true")
}

impl egui::Plugin for ClickAwayFocus {
    fn debug_name(&self) -> &'static str {
        "click_away_focus"
    }

    fn input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        if input.viewport_id == ViewportId::ROOT {
            self.asked_before = false;
            return;
        }
        // the pass of a window outside is about to begin, in the viewer's
        // own: only a click that may take a focus there needs the answer
        let clicked = ctx.input(|i| match self.on {
            SurrenderFocusOn::Presses => i.pointer.any_pressed(),
            SurrenderFocusOn::Clicks => i.pointer.any_click(),
            SurrenderFocusOn::Never => false,
        });
        if clicked && !self.asked_before && ctx.memory(|m| m.focused()).is_some() {
            self.asked_before = focus_asked(ctx);
        }
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
        let window = ui.ctx().viewport_id();
        self.later.retain(|(of, id)| {
            if *of == window {
                ui.ctx().memory_mut(|m| m.surrender_focus(*id));
            }
            *of != window
        });
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
        self.window = ctx.viewport_id();
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
            .is_some_and(|ime| ime.should_interrupt_composition)
            || (self.window == ViewportId::ROOT && self.asked_before);
        if std::mem::take(&mut self.probe) {
            output.platform_output.ime = None;
        }
        if requested {
            return;
        }
        if self.window == ctx.viewport_id() {
            ctx.memory_mut(|m| m.surrender_focus(id));
        } else {
            // (a window closed before its next pass leaves an entry behind:
            // one per window at most, its field gone with it)
            self.later.retain(|(of, _)| *of != self.window);
            self.later.push((self.window, id));
            ctx.request_repaint_of(self.window);
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

    /// A field and a button in a window outside the viewer's, whose pass
    /// runs inside the viewer's as `outside::pass` runs it; the button
    /// gives the focus back to the field when `back`.
    struct Outside {
        ctx: Context,
        window: egui::ViewportId,
        field: Id,
        events: std::rc::Rc<std::cell::RefCell<Vec<Event>>>,
        back: bool,
    }

    impl Outside {
        fn new(back: bool) -> Self {
            let ctx = Context::default();
            super::install(&ctx);
            ctx.set_embed_viewports(false);
            let events: std::rc::Rc<std::cell::RefCell<Vec<Event>>> = Default::default();
            let for_pass = events.clone();
            Context::set_immediate_viewport_renderer(move |ctx, viewport| {
                let egui::ImmediateViewport {
                    ids, mut viewport_ui_cb, ..
                } = viewport;
                let info = egui::ViewportInfo {
                    parent: Some(ids.parent),
                    ..Default::default()
                };
                let input = RawInput {
                    viewport_id: ids.this,
                    viewports: std::iter::once((ids.this, info)).collect(),
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
                    events: std::mem::take(&mut *for_pass.borrow_mut()),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| viewport_ui_cb(ui));
                out.textures_delta.clear();
            });
            let mut outside = Outside {
                ctx,
                window: egui::ViewportId::from_hash_of("click_away_outside"),
                field: Id::unique("click_away_outside_field"),
                events,
                back,
            };
            outside.frame(Vec::new());
            let field = outside.field;
            outside.in_window(move |ui| ui.ctx().memory_mut(|m| m.request_focus(field)));
            assert!(outside.focused());
            outside
        }

        /// A frame of the viewer: a field of its own, then the window
        /// outside, `extra` run in it after its widgets.
        fn frame_with(&mut self, events: Vec<Event>, extra: impl Fn(&mut egui::Ui)) {
            *self.events.borrow_mut() = events;
            let (window, field, back) = (self.window, self.field, self.back);
            let mut out = self.ctx.run_ui(RawInput::default(), |ui| {
                ui.label("inside");
                ui.ctx().show_viewport_immediate(window, egui::ViewportBuilder::default(), |ui, _| {
                    let mut text = String::from("hello");
                    ui.add(TextEdit::singleline(&mut text).id(field));
                    ui.add_space(100.0);
                    if ui.button("Bold").clicked() && back {
                        ui.ctx().memory_mut(|m| m.request_focus(field));
                    }
                    extra(ui);
                });
            });
            out.textures_delta.clear();
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.frame_with(events, |_| {});
        }

        fn in_window(&mut self, extra: impl Fn(&mut egui::Ui)) {
            self.frame_with(Vec::new(), extra);
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

        /// The field has the focus of its window, read in that window.
        fn focused(&mut self) -> bool {
            let seen = std::cell::Cell::new(false);
            let field = self.field;
            self.in_window(|ui| seen.set(ui.ctx().memory(|m| m.has_focus(field))));
            seen.get()
        }
    }

    /// The field of a window outside the viewer's loses the focus to a
    /// click elsewhere in that window (the web browser's address kept the
    /// keys typed after its page was clicked), and keeps it when the widget
    /// clicked gives it back.
    #[test]
    fn a_click_elsewhere_takes_the_focus_in_a_window_outside_too() {
        let mut o = Outside::new(false);
        o.click(BUTTON);
        assert!(!o.focused(), "the click away took the focus");
        // the viewer's own window has no focus to lose, and none appeared
        assert_eq!(o.ctx.memory(|m| m.focused()), None);
        let mut o = Outside::new(true);
        o.click(BUTTON);
        assert!(o.focused(), "the button gave the focus back");
        let mut o = Outside::new(false);
        let field = pos2(40.0, 15.0);
        o.click(field);
        assert!(o.focused(), "a click on the field keeps it");
    }

    /// A field of the viewer's own window keeps the focus a widget gives
    /// back to it, a window outside drawn after that widget in the frame.
    #[test]
    fn the_focus_given_back_inside_is_kept_with_a_window_outside_drawn_after() {
        let ctx = Context::default();
        super::install(&ctx);
        ctx.set_embed_viewports(false);
        Context::set_immediate_viewport_renderer(move |ctx, viewport| {
            let egui::ImmediateViewport {
                ids, mut viewport_ui_cb, ..
            } = viewport;
            let info = egui::ViewportInfo {
                parent: Some(ids.parent),
                ..Default::default()
            };
            let input = RawInput {
                viewport_id: ids.this,
                viewports: std::iter::once((ids.this, info)).collect(),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| viewport_ui_cb(ui));
            out.textures_delta.clear();
        });
        let field = Id::unique("click_away_inside_field");
        let window = egui::ViewportId::from_hash_of("click_away_other");
        let back = std::cell::Cell::new(true);
        let frame = |events: Vec<Event>| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let mut text = String::from("hello");
                ui.add(TextEdit::singleline(&mut text).id(field));
                ui.add_space(100.0);
                if ui.button("Cut").clicked() && back.get() {
                    ui.ctx().memory_mut(|m| m.request_focus(field));
                }
                ui.ctx().show_viewport_immediate(window, egui::ViewportBuilder::default(), |ui, _| {
                    ui.label("outside");
                });
            });
            out.textures_delta.clear();
        };
        frame(Vec::new());
        ctx.memory_mut(|m| m.request_focus(field));
        // (what `focus_asked` reads, under the name egui gives it)
        assert!(super::focus_asked(&ctx));
        frame(Vec::new());
        assert!(!super::focus_asked(&ctx));
        let press = |pressed| Event::PointerButton {
            pos: BUTTON,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![Event::PointerMoved(BUTTON), press(true)]);
        frame(vec![press(false)]);
        assert!(ctx.memory(|m| m.has_focus(field)), "the button gave the focus back");
        // and a button that gives nothing back still takes it
        back.set(false);
        frame(vec![press(true)]);
        frame(vec![press(false)]);
        assert!(!ctx.memory(|m| m.has_focus(field)), "the click away took the focus");
    }
}
