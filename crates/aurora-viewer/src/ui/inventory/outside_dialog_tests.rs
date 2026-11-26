//! A confirmation asked from an Inventory window outside the viewer's
//! window is drawn in that window's own pass, over the window that asked
//! (`widgets::dialog_here`); a window of its own (Properties) still opens
//! in the viewer's window, and a window inside keeps its dialogs there.

use super::*;
use crate::ui::widgets::{DialogCall, Floater};
use std::cell::RefCell;
use std::rc::Rc;

const WINDOW: &str = "test outside inventory dialogs";

fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes.iter().for_each(|shape| collect(&shape.shape, &mut out));
    out
}

fn escape() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}

/// The viewer's window and, when the Inventory window is outside, its
/// window there, whose pass runs inside the viewer's as `outside::pass`
/// runs it (no native window in a test).
struct Viewer {
    ctx: egui::Context,
    inv: Inventory,
    st: InventoryUi,
    actions: Vec<InvAction>,
    /// The texts the window outside drew at the last pass.
    outside: Rc<RefCell<Option<Vec<String>>>>,
    /// The events of the window outside at its next pass.
    outside_events: Rc<RefCell<Vec<egui::Event>>>,
    /// The viewer's frames, for its clock (a window fades in).
    frames: u32,
}

impl Viewer {
    fn new(dialog: EditDialog, outside: bool) -> Self {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let drawn: Rc<RefCell<Option<Vec<String>>>> = Rc::default();
        let events: Rc<RefCell<Vec<egui::Event>>> = Rc::default();
        let (for_pass, events_for_pass) = (drawn.clone(), events.clone());
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| {
            let egui::ImmediateViewport {
                ids, mut viewport_ui_cb, ..
            } = viewport;
            let info = egui::ViewportInfo {
                parent: Some(ids.parent),
                ..Default::default()
            };
            let input = egui::RawInput {
                viewport_id: ids.this,
                viewports: std::iter::once((ids.this, info)).collect(),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 600.0))),
                events: std::mem::take(&mut *events_for_pass.borrow_mut()),
                // (the viewer's clock, as `outside::pass_input` gives it)
                time: Some(ctx.input(|i| i.time)),
                ..Default::default()
            };
            let out = headless_output(ctx.run_ui(input, |ui| viewport_ui_cb(ui)));
            *for_pass.borrow_mut() = Some(texts(&out.shapes));
        });
        let window = egui::Id::unique(WINDOW);
        crate::outside::want(&ctx, window, outside);
        crate::outside::set_native(&ctx, window, outside);
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        Self {
            ctx,
            inv,
            st: InventoryUi {
                dialog: Some(dialog),
                ..Default::default()
            },
            actions: Vec::new(),
            outside: drawn,
            outside_events: events,
            frames: 0,
        }
    }

    /// A pass of the viewer's window, with the Inventory window `open` or
    /// closed: what the viewer's window drew.
    fn frame(&mut self, open: bool, events: Vec<egui::Event>) -> Vec<String> {
        let p = crate::theme::Theme::default().palette();
        let (prefs, facts) = (InventoryPreferences::default(), Facts::default());
        let window = egui::Id::unique(WINDOW);
        self.frames += 1;
        let time = Some(f64::from(self.frames) / 60.0);
        let Self { ctx, inv, st, actions, .. } = self;
        let mut open = open;
        *self.outside.borrow_mut() = None;
        let out = headless_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))),
                time,
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                let icons = Icons::default();
                let images = Default::default();
                Floater::new(WINDOW, "Inventory", egui::pos2(20.0, 20.0), egui::vec2(400.0, 500.0))
                    .silent()
                    .show(&ctx, &p, &mut open, |ui| {
                        ui.label("the window's body");
                        let here = Some((window, DialogCall::InWindow));
                        dialogs::show(ui.ctx(), &p, &icons, inv, st, &prefs, &facts, &images, actions, here);
                    });
                let after = Some((window, DialogCall::After));
                dialogs::show(&ctx, &p, &icons, inv, st, &prefs, &facts, &images, actions, after);
            },
        ));
        texts(&out.shapes)
    }

    fn drawn_outside(&self) -> Vec<String> {
        self.outside.borrow().clone().unwrap_or_default()
    }
}

fn delete() -> EditDialog {
    EditDialog::Delete(vec![Uuid::from_u128(8100)], false)
}

fn count(texts: &[String], text: &str) -> usize {
    texts.iter().filter(|t| *t == text).count()
}

/// The bug of 2026-11-26: Delete asked from an Inventory window outside
/// showed its confirmation over the 3D view of the viewer's window.
#[test]
fn a_delete_confirmation_asked_outside_shows_in_that_window() {
    let title = aurora_i18n::tr!("ui-inventory-dialogs-delete-selection").to_string();
    let mut viewer = Viewer::new(delete(), true);
    for frame in 0..30 {
        let inside = viewer.frame(true, Vec::new());
        assert_eq!(count(&inside, &title), 0, "never in the viewer's window: {inside:?}");
        if frame < 29 {
            continue;
        }
        // (once it has faded in)
        let outside = viewer.drawn_outside();
        assert!(
            outside.iter().any(|t| t == "the window's body"),
            "the window is outside: {outside:?}"
        );
        assert_eq!(count(&outside, &title), 1, "once, over the window that asked: {outside:?}");
    }
    assert!(matches!(viewer.st.dialog, Some(EditDialog::Delete(..))), "it waits for an answer");
    // the keys of the viewer's window do not answer it: it is the other
    // window's
    viewer.frame(true, vec![escape()]);
    assert!(viewer.st.dialog.is_some(), "Escape in the viewer's window leaves it");
    // Escape in its window cancels it, as inside
    viewer.outside_events.borrow_mut().push(escape());
    viewer.frame(true, Vec::new());
    assert!(viewer.st.dialog.is_none(), "Escape in its window cancels it");
    assert!(viewer.actions.is_empty(), "cancelling deletes nothing");
}

/// A give's confirmation is a modal too: it follows the window outside.
#[test]
fn a_give_confirmation_asked_outside_shows_in_that_window() {
    let title = aurora_i18n::tr!("ui-inventory-dialogs-give-to").to_string();
    let share = EditDialog::Share(vec![Uuid::from_u128(8100)], vec![Uuid::from_u128(0x5e)]);
    let mut viewer = Viewer::new(share, true);
    let mut inside = Vec::new();
    for _ in 0..30 {
        inside = viewer.frame(true, Vec::new());
    }
    let outside = viewer.drawn_outside();
    assert!(!inside.iter().any(|t| t.contains(&title)), "not in the viewer's window: {inside:?}");
    assert!(
        outside.iter().any(|t| t.contains(&title)),
        "over the window that asked: {outside:?}"
    );
    assert!(viewer.st.dialog.is_some());
}

/// A window inside keeps its confirmation in the viewer's window, drawn
/// once (the call in its body draws nothing there).
#[test]
fn a_delete_confirmation_asked_inside_shows_once_in_the_viewer_s_window() {
    let title = aurora_i18n::tr!("ui-inventory-dialogs-delete-selection").to_string();
    let mut viewer = Viewer::new(delete(), false);
    let mut last = Vec::new();
    for _ in 0..30 {
        let inside = viewer.frame(true, Vec::new());
        assert!(count(&inside, &title) <= 1, "never twice: {inside:?}");
        last = inside;
        assert!(viewer.outside.borrow().is_none(), "no window outside");
    }
    assert_eq!(count(&last, &title), 1, "{last:?}");
    viewer.frame(true, vec![escape()]);
    assert!(viewer.st.dialog.is_none(), "Escape cancels it");
}

/// The window outside closed with its confirmation waiting: nothing draws
/// there, and the confirmation is not lost, it shows in the viewer's
/// window (as it does for a window inside that closed).
#[test]
fn a_confirmation_of_a_closed_window_outside_shows_in_the_viewer_s_window() {
    let title = aurora_i18n::tr!("ui-inventory-dialogs-delete-selection").to_string();
    let mut viewer = Viewer::new(delete(), true);
    let mut inside = Vec::new();
    for _ in 0..30 {
        inside = viewer.frame(false, Vec::new());
    }
    assert!(viewer.outside.borrow().is_none(), "the closed window is drawn nowhere");
    assert_eq!(count(&inside, &title), 1, "{inside:?}");
}

/// Properties is a window of its own, not a modal: it opens in the
/// viewer's window, as any window does, and can go outside itself.
#[test]
fn properties_asked_from_a_window_outside_open_in_the_viewer_s_window() {
    let title = aurora_i18n::tr!("ui-inventory-dialogs-object-properties").to_string();
    let mut viewer = Viewer::new(EditDialog::Properties(Uuid::from_u128(8100)), true);
    let mut inside = Vec::new();
    for _ in 0..30 {
        inside = viewer.frame(true, Vec::new());
    }
    let outside = viewer.drawn_outside();
    assert!(!outside.iter().any(|t| t.contains(&title)), "{outside:?}");
    assert!(matches!(viewer.st.dialog, Some(EditDialog::Properties(_))), "it stays open");
    assert!(inside.iter().any(|t| t.contains(&title)), "in the viewer's window: {inside:?}");
}
