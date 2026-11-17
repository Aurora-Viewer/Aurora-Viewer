use super::*;
use crate::ui::widgets::elide_tests::Frames;
use egui::{Pos2, Sense};

#[test]
fn a_carried_row_takes_the_place_under_it_and_the_others_make_way() {
    // four rows, 40 apart
    assert_eq!(target_slot(0.0, 40.0, 4), 0);
    assert_eq!(target_slot(19.0, 40.0, 4), 0, "under half a row: stays");
    assert_eq!(target_slot(21.0, 40.0, 4), 1);
    assert_eq!(target_slot(-90.0, 40.0, 4), 0, "above the list: first");
    assert_eq!(target_slot(800.0, 40.0, 4), 3, "below it: last");
    assert_eq!(target_slot(10.0, 40.0, 0), 0);
    // A (0) carried to 2: B and C step up, D stays
    assert_eq!([1, 2, 3].map(|i| shifted_slot(i, 0, 2)), [0, 1, 3]);
    // D (3) carried to 1: B and C step down, A stays
    assert_eq!([0, 1, 2].map(|i| shifted_slot(i, 3, 1)), [0, 2, 3]);
}

#[test]
fn a_drop_from_elsewhere_goes_before_or_after_the_row_under_the_pointer() {
    // rows at 100, 140, 180
    assert_eq!(insert_slot(105.0, 100.0, 40.0, 3), 0, "upper half of the first");
    assert_eq!(insert_slot(125.0, 100.0, 40.0, 3), 1, "lower half of the first");
    assert_eq!(insert_slot(150.0, 100.0, 40.0, 3), 1);
    assert_eq!(insert_slot(50.0, 100.0, 40.0, 3), 0, "above: first");
    assert_eq!(insert_slot(900.0, 100.0, 40.0, 3), 3, "below: last");
}

const ROW: f32 = 40.0;

/// A list of `keys` drawn with the widget: the order is the caller's, the
/// tops drawn and the moves are recorded.
struct List {
    frames: Frames,
    keys: Vec<&'static str>,
    tops: Vec<f32>,
    allocated: Vec<f32>,
    lifted: Option<usize>,
    started: Option<usize>,
    moved: Option<(usize, usize)>,
}

impl List {
    fn new(keys: &[&'static str]) -> Self {
        Self {
            frames: Frames::new(),
            keys: keys.to_vec(),
            tops: Vec::new(),
            allocated: Vec::new(),
            lifted: None,
            started: None,
            moved: None,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>, enabled: bool, gap: Option<usize>) {
        let keys = self.keys.clone();
        let mut out = None;
        self.frames.run(events, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let rows: Vec<egui::Response> = keys
                .iter()
                .map(|_| ui.allocate_exact_size(Vec2::new(200.0, ROW), Sense::click_and_drag()).1)
                .collect();
            let layout = update(ui, egui::Id::unique("reorder-test"), &keys, &rows, enabled, gap);
            out = Some((layout, rows.iter().map(|r| r.rect.top()).collect::<Vec<_>>()));
        });
        let (layout, allocated) = out.expect("the frame ran");
        self.tops = layout.tops;
        self.lifted = layout.lifted;
        self.allocated = allocated;
        if layout.started.is_some() {
            self.started = layout.started;
        }
        if layout.moved.is_some() {
            self.moved = layout.moved;
        }
    }

    fn top(&self, key: &str) -> f32 {
        let i = self.keys.iter().position(|k| *k == key).expect("a key of the list");
        self.tops[i]
    }

    /// Press on `key`'s row and carry it down by `by`, a little at a time.
    fn carry(&mut self, key: &str, by: f32) -> Pos2 {
        self.frame(Vec::new(), true, None);
        let i = self.keys.iter().position(|k| *k == key).expect("a key of the list");
        let start = Pos2::new(60.0, self.allocated[i] + 20.0);
        self.frame(vec![egui::Event::PointerMoved(start), button(start, true)], true, None);
        let mut at = start;
        let steps = (by.abs() / 8.0).ceil() as usize;
        for _ in 0..steps {
            at.y += by.signum() * 8.0;
            self.frame(vec![egui::Event::PointerMoved(at)], true, None);
        }
        at
    }
}

fn button(at: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// A drag down the real list, frame by frame: the card follows the
/// pointer, and from the frame it is released to the frame the new order
/// shows it does not jump (it waits where it was let go, then glides).
#[test]
fn a_dropped_row_stays_where_it_was_let_go_until_it_glides_into_place() {
    let mut list = List::new(&["A", "B", "C", "D"]);
    let at = list.carry("A", 96.0);
    assert_eq!(list.started, Some(0));
    assert_eq!(list.lifted, Some(0));
    let card = list.top("A");
    assert!(
        (card - (at.y - 20.0)).abs() < 1.0,
        "the card is under the pointer: {card} vs {}",
        at.y - 20.0
    );
    // B and C made way, D stayed
    for _ in 0..30 {
        list.frame(vec![egui::Event::PointerMoved(at)], true, None);
    }
    assert!((list.top("B") - list.allocated[0]).abs() < 0.5, "B stepped up");
    assert!((list.top("C") - list.allocated[1]).abs() < 0.5, "C stepped up");
    assert!((list.top("D") - list.allocated[3]).abs() < 0.5, "D stayed");
    // let go: the move is asked, the card stays put until it is applied
    list.frame(vec![button(at, false)], true, None);
    assert_eq!(list.moved, Some((0, 2)));
    assert!((list.top("A") - card).abs() < 1.0, "jumped on release");
    list.frame(Vec::new(), true, None);
    assert!((list.top("A") - card).abs() < 1.0, "jumped before the new order");
    list.keys = vec!["B", "C", "A", "D"];
    list.frame(Vec::new(), true, None);
    // the first step of a glide from where it was let go, not a jump
    let (now, slot) = (list.top("A"), list.allocated[2]);
    assert!(
        now <= card && now > slot && card - now < (card - slot) / 2.0,
        "jumped when the order changed: {now}, let go at {card}, place {slot}"
    );
    assert!((list.top("B") - list.allocated[0]).abs() < 1.0, "B did not move again");
    for _ in 0..60 {
        list.frame(Vec::new(), true, None);
    }
    assert!((list.top("A") - list.allocated[2]).abs() < 0.5, "settled in its place");
    assert_eq!(list.lifted, None, "nothing lifted once it has landed");
}

#[test]
fn a_row_let_go_where_it_was_asks_for_nothing() {
    let mut list = List::new(&["A", "B", "C"]);
    let at = list.carry("B", 8.0);
    list.frame(vec![button(at, false)], true, None);
    assert_eq!(list.moved, None);
    for _ in 0..60 {
        list.frame(Vec::new(), true, None);
    }
    assert_eq!(list.lifted, None);
    assert!((list.top("B") - list.allocated[1]).abs() < 0.5);
}

#[test]
fn a_disabled_list_picks_nothing_up() {
    let mut list = List::new(&["A", "B", "C"]);
    list.frame(Vec::new(), false, None);
    let start = Pos2::new(60.0, list.allocated[0] + 20.0);
    list.frame(vec![egui::Event::PointerMoved(start), button(start, true)], false, None);
    let mut at = start;
    for _ in 0..12 {
        at.y += 8.0;
        list.frame(vec![egui::Event::PointerMoved(at)], false, None);
    }
    list.frame(vec![button(at, false)], false, None);
    assert_eq!((list.started, list.lifted, list.moved), (None, None, None));
}

#[test]
fn a_gap_opens_before_a_row_for_a_drop_and_closes_after() {
    let mut list = List::new(&["A", "B", "C"]);
    list.frame(Vec::new(), true, None);
    for _ in 0..60 {
        list.frame(Vec::new(), true, Some(1));
    }
    assert!((list.top("A") - list.allocated[0]).abs() < 0.5, "A stays");
    assert!((list.top("B") - (list.allocated[1] + ROW)).abs() < 0.5, "B makes room");
    assert!((list.top("C") - (list.allocated[2] + ROW)).abs() < 0.5, "C makes room");
    // the row dropped arrives in the gap: nothing else moves
    list.keys = vec!["A", "N", "B", "C"];
    list.frame(Vec::new(), true, None);
    assert!((list.top("N") - list.allocated[1]).abs() < 0.5, "the new row in the gap");
    assert!((list.top("B") - list.allocated[2]).abs() < 0.5, "B stayed where it was drawn");
}

/// A list no longer drawn while a row is carried (its window closed with a
/// key, another folder shown) forgets the carry: drawn again with the
/// button up, it does not take that for a drop and moves nothing.
#[test]
fn a_list_hidden_during_a_drag_moves_nothing_when_it_comes_back() {
    let mut list = List::new(&["A", "B", "C", "D"]);
    let at = list.carry("A", 96.0);
    assert_eq!(list.lifted, Some(0));
    // frames without the list: the button is let go meanwhile
    list.frames.run(vec![egui::Event::PointerMoved(at)], |_| {});
    list.frames.run(vec![button(at, false)], |_| {});
    list.frames.run(Vec::new(), |_| {});
    list.frame(Vec::new(), true, None);
    assert_eq!(list.moved, None, "a drop nobody made");
    assert_eq!(list.lifted, None);
    for _ in 0..60 {
        list.frame(Vec::new(), true, None);
    }
    assert!((list.top("A") - list.allocated[0]).abs() < 0.5, "A stayed first");
}
