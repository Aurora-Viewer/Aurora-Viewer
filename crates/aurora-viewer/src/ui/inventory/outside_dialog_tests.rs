//! A confirmation asked from an Inventory window outside the viewer's
//! window is drawn over that window, in a pass of its own on its events
//! (`outside::modal_window`, `widgets::Modal::show`); one asked from the
//! viewer's own window stays there, and a window of its own (Properties)
//! opens in the viewer's window as before.

use super::*;
use crate::ui::widgets::Floater;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const WINDOW: &str = "test outside inventory dialogs";

fn window() -> egui::Id {
    egui::Id::unique(WINDOW)
}

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

type Drawn = Rc<RefCell<HashMap<egui::ViewportId, Vec<String>>>>;
type Events = Rc<RefCell<HashMap<egui::ViewportId, Vec<egui::Event>>>>;

/// The viewer's window and the passes outside it, run inside its own as
/// `outside::pass` runs them (no native window in a test): the
/// Inventory window's, and a dialog's over it.
struct Viewer {
    ctx: egui::Context,
    inv: Inventory,
    st: InventoryUi,
    actions: Vec<InvAction>,
    /// The texts each pass outside drew at the last frame.
    drawn: Drawn,
    /// The events of a pass outside at the next frame.
    events: Events,
    frames: u32,
}

impl Viewer {
    fn new(dialog: EditDialog, outside: bool) -> Self {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let (drawn, events): (Drawn, Events) = Default::default();
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
                events: events_for_pass.borrow_mut().remove(&ids.this).unwrap_or_default(),
                time: Some(ctx.input(|i| i.time)),
                ..Default::default()
            };
            let out = headless_output(ctx.run_ui(input, |ui| viewport_ui_cb(ui)));
            for_pass.borrow_mut().insert(ids.this, texts(&out.shapes));
        });
        crate::outside::want(&ctx, window(), outside);
        crate::outside::set_native(&ctx, window(), outside);
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
            drawn,
            events,
            frames: 0,
        }
    }

    /// A frame of the viewer, the Inventory window open, its dialogs after
    /// it as `inventory::show` draws them: what the viewer's window drew.
    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<String> {
        let p = crate::theme::Theme::default().palette();
        let (prefs, facts) = (InventoryPreferences::default(), Facts::default());
        self.frames += 1;
        self.drawn.borrow_mut().clear();
        let time = Some(f64::from(self.frames) / 60.0);
        let Self { ctx, inv, st, actions, .. } = self;
        let out = headless_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))),
                time,
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                let mut open = true;
                Floater::new(WINDOW, "Inventory", egui::pos2(20.0, 20.0), egui::vec2(400.0, 500.0))
                    .silent()
                    .show(&ctx, &p, &mut open, |ui| {
                        ui.label("the window's body");
                    });
                dialogs::show(&ctx, &p, &Icons::default(), inv, st, &prefs, &facts, &Default::default(), actions);
            },
        ));
        texts(&out.shapes)
    }

    /// What the window outside drew, and what the dialog over it drew.
    fn outside(&self) -> (Vec<String>, Vec<String>) {
        let drawn = self.drawn.borrow();
        let of = |id| drawn.get(&id).cloned().unwrap_or_default();
        (
            of(crate::outside::viewport(window())),
            of(crate::outside::overlay(&self.ctx, window())),
        )
    }

    fn press_over_window(&self, event: egui::Event) {
        let overlay = crate::outside::overlay(&self.ctx, window());
        self.events.borrow_mut().entry(overlay).or_default().push(event);
    }
}

fn delete() -> EditDialog {
    EditDialog::Delete(vec![Uuid::from_u128(8100)], false)
}

fn delete_title() -> String {
    aurora_i18n::tr!("ui-inventory-dialogs-delete-selection").to_string()
}

fn count(texts: &[String], text: &str) -> usize {
    texts.iter().filter(|t| *t == text).count()
}

/// The bug of 2026-11-26 (#4998): Delete asked from an Inventory window
/// outside showed its confirmation over the 3D view of the viewer's
/// window.
#[test]
fn a_delete_confirmation_asked_outside_shows_over_that_window() {
    let title = delete_title();
    let mut viewer = Viewer::new(delete(), true);
    // the click on Delete went to the window outside
    crate::outside::focus_for_test(&viewer.ctx, Some(window()));
    for _ in 0..30 {
        let inside = viewer.frame(Vec::new());
        assert_eq!(count(&inside, &title), 0, "never in the viewer's window: {inside:?}");
    }
    let (body, over) = viewer.outside();
    assert!(body.iter().any(|t| t == "the window's body"), "the window is outside: {body:?}");
    assert_eq!(count(&body, &title), 0, "{body:?}");
    assert_eq!(count(&over, &title), 1, "over the window that asked: {over:?}");
    // the focus going elsewhere leaves it where it opened
    crate::outside::focus_for_test(&viewer.ctx, None);
    viewer.frame(Vec::new());
    assert_eq!(count(&viewer.outside().1, &title), 1);
    assert!(matches!(viewer.st.dialog, Some(EditDialog::Delete(..))), "it waits for an answer");
    // the keys of the viewer's window do not answer it
    viewer.frame(vec![escape()]);
    assert!(viewer.st.dialog.is_some(), "Escape in the viewer's window leaves it");
    // Escape over the window cancels it, as inside
    viewer.press_over_window(escape());
    viewer.frame(Vec::new());
    assert!(viewer.st.dialog.is_none(), "Escape in its window cancels it");
    assert!(viewer.actions.is_empty(), "cancelling deletes nothing");
}

/// A give's confirmation is a modal too: it follows the window outside.
#[test]
fn a_give_confirmation_asked_outside_shows_over_that_window() {
    let title = aurora_i18n::tr!("ui-inventory-dialogs-give-to").to_string();
    let share = EditDialog::Share(vec![Uuid::from_u128(8100)], vec![Uuid::from_u128(0x5e)]);
    let mut viewer = Viewer::new(share, true);
    crate::outside::focus_for_test(&viewer.ctx, Some(window()));
    let mut inside = Vec::new();
    for _ in 0..30 {
        inside = viewer.frame(Vec::new());
    }
    let (_, over) = viewer.outside();
    assert!(!inside.iter().any(|t| t.contains(&title)), "not in the viewer's window: {inside:?}");
    assert!(over.iter().any(|t| t.contains(&title)), "over the window that asked: {over:?}");
}

/// Asked while the viewer's own window has the focus (its menu, a key
/// there), a confirmation stays in the viewer's window, the window
/// outside or not.
#[test]
fn a_confirmation_asked_from_the_viewer_s_window_stays_there() {
    let title = delete_title();
    for outside in [true, false] {
        let mut viewer = Viewer::new(delete(), outside);
        let mut inside = Vec::new();
        for _ in 0..30 {
            inside = viewer.frame(Vec::new());
            assert!(count(&inside, &title) <= 1, "never twice: {inside:?}");
        }
        assert_eq!(count(&inside, &title), 1, "{inside:?}");
        assert_eq!(count(&viewer.outside().1, &title), 0);
        viewer.frame(vec![escape()]);
        assert!(viewer.st.dialog.is_none(), "Escape cancels it");
    }
}

/// The window outside closed or brought back inside with its confirmation
/// waiting: the confirmation is not lost, it goes to the viewer's window.
#[test]
fn a_confirmation_whose_window_leaves_goes_to_the_viewer_s_window() {
    let title = delete_title();
    let mut viewer = Viewer::new(delete(), true);
    crate::outside::focus_for_test(&viewer.ctx, Some(window()));
    for _ in 0..30 {
        viewer.frame(Vec::new());
    }
    assert_eq!(count(&viewer.outside().1, &title), 1);
    crate::outside::want(&viewer.ctx, window(), false);
    crate::outside::set_native(&viewer.ctx, window(), false);
    let mut inside = Vec::new();
    for _ in 0..30 {
        inside = viewer.frame(Vec::new());
    }
    assert_eq!(count(&inside, &title), 1, "{inside:?}");
    // and stays there, the window outside again
    crate::outside::want(&viewer.ctx, window(), true);
    crate::outside::set_native(&viewer.ctx, window(), true);
    viewer.frame(Vec::new());
    assert_eq!(count(&viewer.outside().1, &title), 0);
}

/// Properties is a window of its own, not a modal: it opens in the
/// viewer's window, as any window does, and can go outside itself.
#[test]
fn properties_asked_from_a_window_outside_open_in_the_viewer_s_window() {
    let title = aurora_i18n::tr!("ui-inventory-dialogs-object-properties").to_string();
    let mut viewer = Viewer::new(EditDialog::Properties(Uuid::from_u128(8100)), true);
    crate::outside::focus_for_test(&viewer.ctx, Some(window()));
    let mut inside = Vec::new();
    for _ in 0..30 {
        inside = viewer.frame(Vec::new());
    }
    let (body, over) = viewer.outside();
    assert!(!body.iter().chain(&over).any(|t| t.contains(&title)), "{body:?} {over:?}");
    assert!(inside.iter().any(|t| t.contains(&title)), "in the viewer's window: {inside:?}");
}
