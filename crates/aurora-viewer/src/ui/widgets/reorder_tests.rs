use super::*;
use crate::ui::widgets::elide_tests::Frames;
use egui::{Pos2, Rect, Sense};

#[test]
fn a_carried_row_takes_the_place_under_it_and_the_others_make_way() {
    // four rows of 36, 4 apart: 40 a row
    let h = [36.0; 4];
    assert_eq!(target_place(0.0, &h, 4.0, 0), 0);
    assert_eq!(target_place(19.0, &h, 4.0, 0), 0, "under half a row: stays");
    assert_eq!(target_place(21.0, &h, 4.0, 0), 1);
    assert_eq!(target_place(-90.0, &h, 4.0, 0), 0, "above the list: first");
    assert_eq!(target_place(800.0, &h, 4.0, 0), 3, "below it: last");
    assert_eq!(target_place(10.0, &[], 4.0, 0), 0);
    // A (0) carried to 2: B and C step up, D stays
    let tops = tops_in(&moved_order(4, 0, 2), &h, 4.0);
    assert_eq!([1, 2, 3].map(|i| tops[i]), [0.0, 40.0, 120.0]);
    // D (3) carried to 1: B and C step down, A stays
    let tops = tops_in(&moved_order(4, 3, 1), &h, 4.0);
    assert_eq!([0, 1, 2].map(|i| tops[i]), [0.0, 80.0, 120.0]);
    // the places the rows take, for the numbers shown while one is held
    assert_eq!([0, 1, 2, 3].map(|i| place_in(i, 0, 2)), [2, 0, 1, 3]);
    assert_eq!([0, 1, 2, 3].map(|i| place_in(i, 3, 1)), [0, 2, 3, 1]);
}

/// Rows of their own heights (a photo filter's card open to its settings):
/// the place taken is the one whose top is nearest, and the others make
/// way by the carried row's own height.
#[test]
fn rows_of_their_own_heights_make_way_by_the_carried_rows_height() {
    // A 40, B open at 200, C 40, 6 apart: the others' places at 0, 206, 252
    let h = [40.0, 200.0, 40.0];
    assert_eq!(target_place(90.0, &h, 6.0, 0), 0, "nearer its own place");
    assert_eq!(target_place(110.0, &h, 6.0, 0), 1, "nearer the place after B");
    assert_eq!(target_place(240.0, &h, 6.0, 0), 2);
    // A carried under B: B steps up by A's height, not by its own
    let tops = tops_in(&moved_order(3, 0, 1), &h, 6.0);
    assert_eq!(tops, [206.0, 0.0, 252.0]);
    // a drop goes before the first row whose middle is below the pointer
    let rows = [
        Rect::from_min_size(Pos2::new(0.0, 100.0), Vec2::new(200.0, 40.0)),
        Rect::from_min_size(Pos2::new(0.0, 146.0), Vec2::new(200.0, 200.0)),
        Rect::from_min_size(Pos2::new(0.0, 352.0), Vec2::new(200.0, 40.0)),
    ];
    assert_eq!(insert_place(105.0, &rows), 0);
    assert_eq!(insert_place(125.0, &rows), 1);
    assert_eq!(insert_place(240.0, &rows), 1, "upper half of the open card");
    assert_eq!(insert_place(250.0, &rows), 2);
    assert_eq!(insert_place(900.0, &rows), 3);
    assert_eq!(insert_place(0.0, &[]), 0);
}

/// The real list with an open card: C carried up over it takes the place
/// between A and B, and B makes way by C's height.
#[test]
fn a_short_row_carried_over_a_tall_one_takes_its_place() {
    let mut list = List::new(&["A", "B", "C"]);
    list.heights.insert("B", 160.0);
    let at = list.carry("C", -100.0);
    for _ in 0..30 {
        list.frame(vec![egui::Event::PointerMoved(at)], true, None);
    }
    let top0 = list.allocated[0];
    assert!((list.top("A") - top0).abs() < 0.5, "A stayed");
    assert!(
        (list.top("B") - (top0 + 2.0 * ROW)).abs() < 0.5,
        "B stepped down by C's height: {}",
        list.top("B") - top0
    );
    list.frame(vec![button(at, false)], true, None);
    assert_eq!(list.moved, Some((2, 1)));
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
    /// Rows of another height than `ROW`, by key.
    heights: std::collections::HashMap<&'static str, f32>,
    /// The visible list (the screen if None), and whether a card may leave
    /// it (`update_sized`'s `free`).
    clip: Option<egui::Rect>,
    free: bool,
    tops: Vec<f32>,
    allocated: Vec<f32>,
    lifted: Option<usize>,
    started: Option<usize>,
    moved: Option<(usize, usize)>,
    out: bool,
}

impl List {
    fn new(keys: &[&'static str]) -> Self {
        Self {
            frames: Frames::new(),
            keys: keys.to_vec(),
            heights: std::collections::HashMap::new(),
            clip: None,
            free: false,
            tops: Vec::new(),
            allocated: Vec::new(),
            lifted: None,
            started: None,
            moved: None,
            out: false,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>, enabled: bool, gap: Option<usize>) {
        let keys = self.keys.clone();
        let heights: Vec<f32> = keys.iter().map(|k| self.heights.get(k).copied().unwrap_or(ROW)).collect();
        let mut out = None;
        let (clip, free) = (self.clip, self.free);
        self.frames.run(events, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if let Some(clip) = clip {
                ui.set_clip_rect(clip);
            }
            let rows: Vec<egui::Response> = heights
                .iter()
                .map(|&h| ui.allocate_exact_size(Vec2::new(200.0, h), Sense::click_and_drag()).1)
                .collect();
            let layout = update_sized(
                ui,
                egui::Id::unique("reorder-test"),
                &keys,
                &rows,
                enabled,
                gap.map(|g| (g, ROW)),
                free,
            );
            out = Some((layout, rows.iter().map(|r| r.rect.top()).collect::<Vec<_>>()));
        });
        let (layout, allocated) = out.expect("the frame ran");
        self.tops = layout.tops;
        self.lifted = layout.lifted;
        self.out = layout.out;
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

/// A free list (the photo filters' cards): carried out of the visible list,
/// the card follows the pointer there and the others go back to their
/// places; let go there, nothing moves (the caller takes it out, or not).
#[test]
fn a_free_card_leaves_its_list_and_the_others_go_back() {
    let mut list = List::new(&["A", "B", "C"]);
    list.free = true;
    list.frame(Vec::new(), true, None);
    let top0 = list.allocated[0];
    list.clip = Some(egui::Rect::from_min_max(Pos2::new(0.0, top0), Pos2::new(200.0, top0 + 3.0 * ROW)));
    let at = list.carry("A", 3.0 * ROW + 120.0);
    // held 20 below its top: its bottom at the list's end, still in it;
    // a little further it touches what lies past the end, and is out
    let end = top0 + 3.0 * ROW;
    list.frame(vec![egui::Event::PointerMoved(Pos2::new(at.x, end - ROW + 20.0))], true, None);
    assert!(!list.out, "its bottom at the list's end: still in its list");
    list.frame(vec![egui::Event::PointerMoved(Pos2::new(at.x, end - ROW + 24.0))], true, None);
    assert!(list.out, "touching what lies past the end: out");
    for _ in 0..30 {
        list.frame(vec![egui::Event::PointerMoved(at)], true, None);
    }
    assert!(list.out, "past it, out");
    assert!((list.top("A") - (at.y - 20.0)).abs() < 1.0, "under the pointer, out of the list");
    assert!((list.top("B") - (top0 + ROW)).abs() < 0.5, "B back in its place");
    assert!((list.top("C") - (top0 + 2.0 * ROW)).abs() < 0.5, "C back in its place");
    list.frame(vec![button(at, false)], true, None);
    assert_eq!(list.moved, None, "let go out of the list: no move asked");
    // a list that is not free keeps the card in it
    let mut kept = List::new(&["A", "B", "C"]);
    kept.frame(Vec::new(), true, None);
    let top0 = kept.allocated[0];
    kept.clip = Some(egui::Rect::from_min_max(Pos2::new(0.0, top0), Pos2::new(200.0, top0 + 3.0 * ROW)));
    let at = kept.carry("A", 3.0 * ROW + 120.0);
    kept.frame(vec![egui::Event::PointerMoved(at)], true, None);
    assert!(kept.top("A") <= top0 + 2.0 * ROW + 0.5, "kept inside the list");
}

/// A free list given more room than its rows take (the photo filters'
/// cards, the catalogue under them): the card leaves it as soon as it
/// touches what lies past its last row, not past its room.
#[test]
fn a_free_card_leaves_at_its_last_row_not_at_its_room() {
    let mut list = List::new(&["A", "B", "C"]);
    list.free = true;
    list.frame(Vec::new(), true, None);
    let top0 = list.allocated[0];
    list.clip = Some(egui::Rect::from_min_max(Pos2::new(0.0, top0), Pos2::new(200.0, top0 + 6.0 * ROW)));
    let at = list.carry("A", 2.0 * ROW);
    let end = top0 + 3.0 * ROW;
    list.frame(vec![egui::Event::PointerMoved(Pos2::new(at.x, end - ROW + 20.0))], true, None);
    assert!(!list.out, "its bottom at the last row's: still in its list");
    list.frame(vec![egui::Event::PointerMoved(Pos2::new(at.x, end - ROW + 24.0))], true, None);
    assert!(list.out, "a little past it, out, well inside its room");
}

/// A free list that scrolls further: at its visible end the card stays in
/// it, the band at the edge scrolling the list under it, until the pointer
/// goes past that band.
#[test]
fn a_free_card_stays_in_a_list_that_scrolls_further() {
    let mut list = List::new(&["A", "B", "C"]);
    list.free = true;
    list.frame(Vec::new(), true, None);
    let top0 = list.allocated[0];
    // two rows of three visible: C lies past the visible end
    list.clip = Some(egui::Rect::from_min_max(Pos2::new(0.0, top0), Pos2::new(200.0, top0 + 2.0 * ROW)));
    let at = list.carry("A", 2.0 * ROW - 8.0);
    let end = top0 + 2.0 * ROW;
    list.frame(vec![egui::Event::PointerMoved(Pos2::new(at.x, end - 10.0))], true, None);
    assert!(!list.out, "in the band that scrolls it: still in the list");
    list.frame(vec![egui::Event::PointerMoved(Pos2::new(at.x, end + 40.0))], true, None);
    assert!(list.out, "past the band: out");
}
