use super::*;
use egui::{Id, Pos2, Rect};

struct Window {
    id: String,
    group: CascadeGroup,
    open: bool,
    size: Vec2,
    /// The floater's least size and its content's height, when not the
    /// defaults (a line of text).
    least: Option<Vec2>,
    content: f32,
}

#[derive(Default)]
struct Fixture {
    ctx: egui::Context,
    windows: Vec<Window>,
    frame: u64,
}

impl Fixture {
    fn settle(&mut self) {
        self.frames(8);
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            self.frame += 1;
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                    time: Some(self.frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    for w in &mut self.windows {
                        let mut floater = Floater::new(&w.id, &w.id, egui::pos2(100.0, 80.0), w.size)
                            .cascade(w.group)
                            .silent();
                        if let Some(least) = w.least {
                            floater.min_size = least;
                        }
                        let content = w.content;
                        floater.show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut w.open, |ui| {
                            ui.label("Independent document");
                            ui.allocate_space(egui::vec2(10.0, content));
                        });
                    }
                },
            );
            output.textures_delta.clear();
        }
    }

    fn open(&mut self, group: CascadeGroup, size: Vec2) -> usize {
        self.open_with(group, size, None, 0.0)
    }

    fn open_with(&mut self, group: CascadeGroup, size: Vec2, least: Option<Vec2>, content: f32) -> usize {
        let n = self.push(group, size, least, content);
        self.settle();
        n
    }

    fn push(&mut self, group: CascadeGroup, size: Vec2, least: Option<Vec2>, content: f32) -> usize {
        let n = self.windows.len();
        self.windows.push(Window {
            id: format!("cascade_{n}"),
            group,
            open: true,
            size,
            least,
            content,
        });
        n
    }

    fn rect(&self, n: usize) -> Rect {
        self.ctx
            .memory(|m| m.area_rect(Id::unique(&self.windows[n].id)).expect("visible floater"))
    }

    fn move_to(&mut self, n: usize, pos: Pos2) {
        move_floater(&self.ctx, &self.windows[n].id, pos);
        self.ctx
            .move_to_top(egui::LayerId::new(egui::Order::Middle, Id::unique(&self.windows[n].id)));
        self.settle();
    }
}

fn same(actual: Vec2, expected: Vec2) {
    assert!((actual - expected).length() < 1.0, "{actual:?} != {expected:?}");
}

#[test]
fn cascade_follows_last_created_window_in_its_group_and_keeps_each_size() {
    let mut f = Fixture::default();
    let first = f.open(CascadeGroup::Texture, egui::vec2(300.0, 250.0));
    let second = f.open(CascadeGroup::Texture, egui::vec2(400.0, 350.0));
    same(f.rect(second).min - f.rect(first).min, Vec2::splat(32.0));
    assert!(f.rect(second).width() > f.rect(first).width() + 90.0);
    // Raising/moving an older instance does not change the registry order.
    f.move_to(first, egui::pos2(420.0, 260.0));
    let older = f.rect(first);
    let last = f.rect(second);
    let third = f.open(CascadeGroup::Texture, egui::vec2(300.0, 250.0));
    same(f.rect(third).min - last.min, Vec2::splat(32.0));
    assert_eq!(f.rect(first), older);
    assert_eq!(f.rect(second), last);
    f.move_to(third, egui::pos2(300.0, 200.0));
    let fourth = f.open(CascadeGroup::Texture, egui::vec2(300.0, 250.0));
    same(f.rect(fourth).min - f.rect(third).min, Vec2::splat(32.0));
    // The material group starts from its own default, independent of textures.
    let material = f.open(CascadeGroup::Material, egui::vec2(300.0, 250.0));
    same(f.rect(material).min.to_vec2(), egui::vec2(100.0, 80.0));
}

#[test]
fn cascade_skips_minimized_and_closed_windows_and_preserves_existing_instances() {
    let mut f = Fixture::default();
    let first = f.open(CascadeGroup::Texture, egui::vec2(300.0, 250.0));
    let second = f.open(CascadeGroup::Texture, egui::vec2(300.0, 250.0));
    minimize_floater(&f.ctx, &f.windows[second].id);
    f.settle();
    let third = f.open(CascadeGroup::Texture, egui::vec2(300.0, 250.0));
    same(f.rect(third).min - f.rect(first).min, Vec2::splat(32.0));
    let before = f.rect(third);
    let w = &mut f.windows[third];
    open_floater(&f.ctx, &w.id, &mut w.open);
    f.settle();
    assert_eq!(f.rect(third), before);
    f.windows[third].open = false;
    f.settle();
    let fourth = f.open(CascadeGroup::Texture, egui::vec2(300.0, 250.0));
    same(f.rect(fourth).min - f.rect(first).min, Vec2::splat(32.0));
    let w = &mut f.windows[second];
    open_floater(&f.ctx, &w.id, &mut w.open);
    f.settle();
    assert!(!floater_minimized(&f.ctx, &f.windows[second].id));
    same(f.rect(second).min - f.rect(first).min, Vec2::splat(32.0));
}

#[test]
fn cascade_fits_actual_geometry_at_screen_edges_without_periodic_wrap() {
    let mut f = Fixture::default();
    for _ in 0..12 {
        let n = f.open(CascadeGroup::Texture, egui::vec2(300.0, 250.0));
        same(f.rect(n).min.to_vec2(), egui::vec2(100.0, 80.0) + Vec2::splat(n as f32 * 32.0));
    }
    f.move_to(11, egui::pos2(690.0, 540.0));
    let n = f.open(CascadeGroup::Texture, egui::vec2(410.0, 390.0));
    let area = crate::ui::layout::work_area(&f.ctx);
    assert!(
        area.expand(1.0).contains_rect(f.rect(n)),
        "{area:?} does not contain {:?}",
        f.rect(n)
    );
    same(f.rect(n).min.to_vec2(), (area.max - f.rect(n).size()).to_vec2());
}

#[test]
fn cascade_document_groups_remember_primary_geometry_without_size_drift() {
    for group in [
        CascadeGroup::Notecard,
        CascadeGroup::Script,
        CascadeGroup::Animation,
        CascadeGroup::Gesture,
    ] {
        let mut f = Fixture::default();
        let first = f.open(group, egui::vec2(320.0, 260.0));
        place_floater(&f.ctx, &f.windows[first].id, egui::pos2(240.0, 180.0), egui::vec2(380.0, 300.0));
        f.settle();
        let saved = f.rect(first);
        let second = f.open(group, egui::vec2(320.0, 260.0));
        same(f.rect(second).size(), saved.size());
        same(f.rect(second).min - saved.min, Vec2::splat(32.0));
        // A secondary cascaded window must not overwrite the group's saved rect.
        f.move_to(second, egui::pos2(480.0, 360.0));
        f.windows[first].open = false;
        f.windows[second].open = false;
        f.settle();
        for _ in 0..3 {
            let n = f.open(group, egui::vec2(320.0, 260.0));
            same(f.rect(n).size(), saved.size());
            same(f.rect(n).min.to_vec2(), saved.min.to_vec2());
            f.windows[n].open = false;
            f.settle();
        }
    }
}

#[test]
fn cascade_temporarily_hidden_documents_keep_their_positions() {
    let mut f = Fixture::default();
    let first = f.open(CascadeGroup::Script, egui::vec2(320.0, 260.0));
    let second = f.open(CascadeGroup::Script, egui::vec2(320.0, 260.0));
    let before = [f.rect(first), f.rect(second)];
    hide_windows(&f.ctx, true, HideKeep::None);
    f.settle();
    hide_windows(&f.ctx, false, HideKeep::None);
    f.settle();
    assert_eq!([f.rect(first), f.rect(second)], before);
}

#[test]
fn cascade_group_profiles_keep_their_step_when_taller_than_the_room() {
    let mut f = Fixture::default();
    let tall = egui::vec2(600.0, 1000.0);
    let profile = CascadeGroup::Profile { classic: false };
    let first = f.open(profile, tall);
    let second = f.open(profile, tall);
    let (a, b) = (f.rect(first), f.rect(second));
    same(b.min - a.min, Vec2::splat(32.0));
    assert!((b.max.y - a.max.y).abs() < 1.0, "smaller by its step, same bottom: {a:?} {b:?}");
    // other groups are still placed back inside, as the Legacy Viewer does
    let texture = f.open(CascadeGroup::Texture, tall);
    let again = f.open(CascadeGroup::Texture, tall);
    assert!((f.rect(again).min.y - f.rect(texture).min.y).abs() < 1.0);
}

#[test]
fn cascade_group_profiles_start_a_new_column_before_getting_too_small() {
    let mut f = Fixture::default();
    let profile = CascadeGroup::Profile { classic: false };
    let opened: Vec<usize> = (0..40).map(|_| f.open(profile, egui::vec2(500.0, 600.0))).collect();
    let first = f.rect(opened[0]);
    let mut wrapped = false;
    let mut at_edge = false;
    for pair in opened.windows(2) {
        let (a, b) = (f.rect(pair[0]), f.rect(pair[1]));
        assert!(b.height() >= first.height() * 0.5 - 1.0, "never under half the first: {b:?}");
        assert!((b.width() - first.width()).abs() < 1.0, "the width is kept: {b:?}");
        assert!(b.right() <= 1000.5 && b.bottom() <= 800.5, "on screen: {b:?}");
        if (b.min.y - first.min.y).abs() < 1.0 {
            wrapped = true;
        } else {
            same(egui::vec2(0.0, b.min.y - a.min.y), egui::vec2(0.0, 32.0));
        }
        if (b.right() - 1000.0).abs() < 1.0 && (a.right() - 1000.0).abs() < 1.0 {
            // at the right edge: down only
            assert!((b.min.x - a.min.x).abs() < 1.0);
            at_edge = true;
        }
    }
    assert!(wrapped, "40 windows on an 800 px screen start a new column");
    assert!(at_edge, "and reach the right edge");
}

/// A group profile's real least size and content (#4964 review: past about
/// 20 windows they piled at one spot instead of starting a new column, the
/// windows never getting as small as half the first one).
#[test]
fn cascade_group_profiles_with_a_real_least_size_start_a_new_column_instead_of_piling() {
    for (least, content) in [(egui::vec2(320.0, 300.0), 420.0), (egui::vec2(400.0, 380.0), 200.0)] {
        let mut f = Fixture::default();
        let profile = CascadeGroup::Profile { classic: false };
        let opened: Vec<usize> = (0..30)
            .map(|_| f.open_with(profile, egui::vec2(500.0, 600.0), Some(least), content))
            .collect();
        let first = f.rect(opened[0]);
        let mut columns = 1;
        for pair in opened.windows(2) {
            let (a, b) = (f.rect(pair[0]), f.rect(pair[1]));
            assert!(b.bottom() <= 800.5, "on screen: {b:?}");
            assert!((b.min - a.min).length() > 1.0, "never on the window before: {a:?} {b:?}");
            if (b.min.y - first.min.y).abs() < 1.0 {
                columns += 1;
            } else {
                // a step down, at most one window put back up as the column
                // learns how small its windows really get
                assert!(b.min.y > a.min.y, "below the window before: {a:?} {b:?}");
            }
        }
        assert!(columns >= 3, "30 windows on an 800 px screen take several columns: {columns}");
    }
}

/// A copy opens its group windows one frame after the other, so the window
/// before has only had its first draw when the next is placed, and a window
/// taller than its room is pushed up on that draw (#4964 review, round 2:
/// from the 14th on, each window anchored on that higher rect, landed at the
/// same spot as the one before and the wrap never came).
#[test]
fn cascade_group_profiles_opened_a_frame_apart_still_wrap_into_columns() {
    for (least, content) in [(egui::vec2(320.0, 300.0), 420.0), (egui::vec2(400.0, 380.0), 200.0)] {
        let mut f = Fixture::default();
        let profile = CascadeGroup::Profile { classic: false };
        let opened: Vec<usize> = (0..30)
            .map(|_| {
                let n = f.push(profile, egui::vec2(500.0, 600.0), Some(least), content);
                f.frames(1);
                n
            })
            .collect();
        f.settle();
        let first = f.rect(opened[0]);
        let mut columns = 1;
        for (i, &n) in opened.iter().enumerate() {
            let b = f.rect(n);
            assert!(b.bottom() <= 800.5, "on screen: {b:?}");
            if i > 0 {
                let a = f.rect(opened[i - 1]);
                assert!((b.min - a.min).length() > 1.0, "{n} lands on the window before: {a:?} {b:?}");
            }
            if i > 0 && (b.min.y - first.min.y).abs() < 1.0 {
                columns += 1;
            }
        }
        assert!(columns >= 3, "30 windows on an 800 px screen take several columns: {columns}");
    }
}

/// A window still opening is anchored where the cascade put it, not where
/// its first draw was shown (#4964 review, round 2: a window taller than its
/// room is pushed up on that draw, and the next one stepped from there).
#[test]
fn cascade_steps_from_where_an_opening_window_was_placed_not_its_first_draw() {
    let ctx = egui::Context::default();
    let work = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 800.0));
    let default = Rect::from_min_size(egui::pos2(100.0, 80.0), egui::vec2(500.0, 600.0));
    let profile = CascadeGroup::Profile { classic: false };
    let least = egui::vec2(220.0, 120.0);
    let (a, b) = (Id::unique("a"), Id::unique("b"));
    let placed = super::cascade::prepare(&ctx, a, profile, default, work, least).expect("placed");
    // the first draw shows it 32 px higher than it was placed
    super::cascade::shown(
        &ctx,
        a,
        Rect::from_min_size(placed.pos - egui::vec2(0.0, 32.0), egui::vec2(500.0, 600.0)),
        false,
    );
    let next = super::cascade::prepare(&ctx, b, profile, default, work, least).expect("placed");
    same(next.pos.to_vec2() - placed.pos.to_vec2(), Vec2::splat(32.0));
}

/// A notice copy opens its ten windows in one frame, before any of them is
/// settled: each is stacked on the one opened just before, whose first draw
/// egui reports before the window is fitted where the cascade put it
/// (#4964 review: from the 15th window on, all landed at one spot).
#[test]
fn cascade_group_profiles_opened_in_one_frame_start_a_new_column_instead_of_piling() {
    let mut f = Fixture::default();
    let profile = CascadeGroup::Profile { classic: false };
    let mut opened = Vec::new();
    for _ in 0..3 {
        for _ in 0..10 {
            opened.push(f.push(profile, egui::vec2(500.0, 600.0), Some(egui::vec2(320.0, 300.0)), 420.0));
        }
        f.settle();
    }
    let first = f.rect(opened[0]);
    let mut columns = 1;
    let mut restarted = false;
    for (i, &n) in opened.iter().enumerate() {
        let b = f.rect(n);
        assert!(b.bottom() <= 800.5 && b.right() <= 1000.5, "on screen: {b:?}");
        if i == 0 {
            continue;
        }
        for &m in &opened[..i] {
            let a = f.rect(m);
            assert!((a.min - b.min).length() > 1.0, "never on an earlier window: {a:?} {b:?}");
        }
        let a = f.rect(opened[i - 1]);
        if (b.min.y - first.min.y).abs() < 1.0 {
            columns += 1;
            if a.right() + 32.0 > 1000.5 {
                // no room to step right: the new column starts at the left edge
                assert!(b.min.x.abs() < 1.0, "back at the left edge: {a:?} {b:?}");
                restarted = true;
            }
        } else {
            assert!(b.min.y > a.min.y, "below the window before: {a:?} {b:?}");
        }
    }
    assert!(columns >= 3, "30 windows on an 800 px screen take several columns: {columns}");
    assert!(restarted, "and a column that reached the right edge restarts at the left");
}
