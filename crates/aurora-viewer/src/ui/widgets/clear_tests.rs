//! The clear cross of the search and filter fields (`clear_button`).

use crate::theme::{Palette, Theme};

struct Harness {
    ctx: egui::Context,
    p: Palette,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        Theme::default().apply(&ctx, 1.0);
        Self {
            ctx,
            p: Theme::default().palette(),
        }
    }

    fn frame(&self, events: Vec<egui::Event>, mut add: impl FnMut(&mut egui::Ui, &Palette)) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 300.0))),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| add(ui, &self.p));
        output.textures_delta.clear();
    }
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

fn tab() -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    }]
}

/// Where the cross of a field sits.
fn cross(field: egui::Rect) -> egui::Pos2 {
    egui::pos2(field.right() - 10.0, field.center().y)
}

fn field_id() -> egui::Id {
    egui::Id::unique("clear-test-field")
}

fn identified(ui: &mut egui::Ui, p: &Palette, text: &mut String) -> egui::Response {
    super::search_box_with(ui, p, text, "", 200.0, |edit| edit.id(field_id()))
}

#[test]
fn the_cross_empties_the_field_reports_a_change_and_keeps_the_keyboard() {
    let h = Harness::new();
    let mut text = "kink".to_owned();
    let mut rect = egui::Rect::NOTHING;
    let mut changed = false;
    for events in [Vec::new(), Vec::new()] {
        h.frame(events, |ui, p| {
            rect = identified(ui, p, &mut text).rect;
        });
    }
    h.frame(click(cross(rect)), |ui, p| {
        changed |= identified(ui, p, &mut text).changed();
    });
    h.frame(Vec::new(), |ui, p| {
        identified(ui, p, &mut text);
    });
    assert_eq!(text, "");
    assert!(changed, "the callers filter on `changed`");
    assert!(h.ctx.memory(|m| m.has_focus(field_id())), "typing can go on");
}

#[test]
fn nothing_to_clear_and_nothing_to_click_on_an_empty_or_disabled_field() {
    let h = Harness::new();
    let cross_id = field_id().with("clear");
    let mut text = String::new();
    h.frame(Vec::new(), |ui, p| {
        identified(ui, p, &mut text);
    });
    assert!(h.ctx.read_response(cross_id).is_none(), "an empty field has no cross");
    let mut text = "kink".to_owned();
    let mut rect = egui::Rect::NOTHING;
    for events in [Vec::new(), Vec::new()] {
        h.frame(events, |ui, p| {
            ui.add_enabled_ui(false, |ui| {
                rect = identified(ui, p, &mut text).rect;
            });
        });
    }
    assert!(h.ctx.read_response(cross_id).is_none(), "a disabled field has no cross");
    h.frame(click(cross(rect)), |ui, p| {
        ui.add_enabled_ui(false, |ui| {
            identified(ui, p, &mut text);
        });
    });
    assert_eq!(text, "kink");
}

#[test]
fn tab_goes_to_the_next_field_past_the_cross() {
    let h = Harness::new();
    let mut text = "kink".to_owned();
    let mut next = String::new();
    let next_id = egui::Id::unique("clear-test-next");
    let mut draw = |events: Vec<egui::Event>| {
        h.frame(events, |ui, p| {
            let field = identified(ui, p, &mut text);
            ui.add(egui::TextEdit::singleline(&mut next).id(next_id));
            if ui.ctx().cumulative_frame_nr() == 0 {
                field.request_focus();
            }
        });
    };
    draw(Vec::new());
    draw(Vec::new());
    draw(tab());
    draw(Vec::new());
    assert!(h.ctx.memory(|m| m.has_focus(next_id)));
}

/// The face and the text stay where they are whether the cross shows or not.
#[test]
fn the_first_letter_moves_nothing() {
    let h = Harness::new();
    let measure = |text: &str| {
        let mut text = text.to_owned();
        let mut rect = egui::Rect::NOTHING;
        for _ in 0..3 {
            h.frame(Vec::new(), |ui, p| {
                rect = identified(ui, p, &mut text).rect;
            });
        }
        rect
    };
    assert_eq!(measure(""), measure("kink"));
}

/// The box takes the height of its line in a column, as in a row, and stays
/// left to right in a row laid out from the right.
#[test]
fn the_box_is_one_line_high_in_any_layout() {
    let h = Harness::new();
    let mut text = "kink".to_owned();
    let (mut column, mut row, mut right) = (egui::Rect::NOTHING, egui::Rect::NOTHING, egui::Rect::NOTHING);
    let mut rtl_field = egui::Rect::NOTHING;
    for _ in 0..3 {
        h.frame(Vec::new(), |ui, p| {
            let before = ui.cursor().top();
            super::search_box(ui, p, &mut text, "", 200.0);
            column = egui::Rect::from_min_max(egui::pos2(0.0, before), egui::pos2(1.0, ui.cursor().top()));
            row = ui.horizontal(|ui| super::search_box(ui, p, &mut text, "", 200.0).rect).inner;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                rtl_field = super::search_box(ui, p, &mut text, "", 200.0).rect;
                right = ui.max_rect();
            });
        });
    }
    assert!(column.height() < 60.0, "a column's whole height is not taken: {}", column.height());
    assert_eq!(rtl_field.height(), row.height());
    assert!(right.width() > 0.0);
}

/// In a toolbar's row (the inventory's) the box sits level with the button
/// after it, right under the line above.
#[test]
fn a_sized_box_stays_level_with_the_buttons_of_its_row() {
    let h = Harness::new();
    let mut text = String::new();
    let (mut above, mut face, mut next) = (egui::Rect::NOTHING, egui::Rect::NOTHING, egui::Rect::NOTHING);
    for _ in 0..3 {
        h.frame(Vec::new(), |ui, p| {
            above = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover()).0;
            ui.horizontal(|ui| {
                face = ui
                    .scope(|ui| super::search_face_sized(ui, p, "search-filled", 200.0, 30.0, |ui| ui.text_edit_singleline(&mut text)))
                    .response
                    .rect;
                next = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover()).0;
            });
        });
    }
    assert!((face.height() - 30.0).abs() < 1.0, "as high as the row: {face:?}");
    assert!((face.center().y - next.center().y).abs() < 1.0, "level: {face:?} {next:?}");
    assert!(face.top() - above.bottom() < 8.0, "right under the line above: {above:?} {face:?}");
}

/// A row taller than the box does not push the text down its face: the
/// row's centring stops at the face.
#[test]
fn a_taller_row_does_not_push_the_text_down_the_face() {
    let h = Harness::new();
    let mut text = "kink".to_owned();
    let (mut face, mut field) = (egui::Rect::NOTHING, egui::Rect::NOTHING);
    for _ in 0..3 {
        h.frame(Vec::new(), |ui, p| {
            let row = ui.horizontal(|ui| {
                let shown = ui
                    .scope(|ui| {
                        ui.spacing_mut().interact_size.y = 30.0;
                        super::search_box(ui, p, &mut text, "", 200.0)
                    })
                    .inner
                    .rect;
                ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
                shown
            });
            face = row.response.rect;
            field = row.inner;
        });
    }
    assert!(
        field.top() - face.top() < 6.0,
        "at the top of the row, not pushed down: {face:?} {field:?}"
    );
}

/// A window's taller rows (the inventory's tree) leave the box its height.
#[test]
fn the_box_keeps_its_height_under_taller_rows() {
    let h = Harness::new();
    let mut text = "kink".to_owned();
    let (mut usual, mut tall) = (egui::Rect::NOTHING, egui::Rect::NOTHING);
    for _ in 0..3 {
        h.frame(Vec::new(), |ui, p| {
            usual = ui.scope(|ui| super::search_box(ui, p, &mut text, "", 200.0)).response.rect;
            tall = ui
                .scope(|ui| {
                    ui.spacing_mut().interact_size.y = 40.0;
                    super::search_box(ui, p, &mut text, "", 200.0)
                })
                .response
                .rect;
        });
    }
    assert_eq!(usual.height(), tall.height());
}

#[test]
fn every_search_widget_has_the_cross() {
    let h = Harness::new();
    let mut text = "kink".to_owned();
    let mut ids = Vec::new();
    h.frame(Vec::new(), |ui, p| {
        ids.push(super::search_box(ui, p, &mut text, "", 200.0).id);
        ids.push(super::filter_field(ui, p, &mut text, egui::Id::unique("filter"), 200.0).id);
    });
    for id in ids {
        assert!(h.ctx.read_response(id.with("clear")).is_some());
    }
}
