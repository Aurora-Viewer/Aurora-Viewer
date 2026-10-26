use super::*;
use egui::{Id, Pos2, Rect};

struct Window {
    id: String,
    group: CascadeGroup,
    open: bool,
    size: Vec2,
}

#[derive(Default)]
struct Fixture {
    ctx: egui::Context,
    windows: Vec<Window>,
    frame: u64,
}

impl Fixture {
    fn settle(&mut self) {
        for _ in 0..8 {
            self.frame += 1;
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                    time: Some(self.frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    for w in &mut self.windows {
                        Floater::new(&w.id, &w.id, egui::pos2(100.0, 80.0), w.size)
                            .cascade(w.group)
                            .silent()
                            .show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut w.open, |ui| {
                                ui.label("Independent document");
                            });
                    }
                },
            );
            output.textures_delta.clear();
        }
    }

    fn open(&mut self, group: CascadeGroup, size: Vec2) -> usize {
        let n = self.windows.len();
        self.windows.push(Window {
            id: format!("cascade_{n}"),
            group,
            open: true,
            size,
        });
        self.settle();
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
    hide_windows(&f.ctx, true, false);
    f.settle();
    hide_windows(&f.ctx, false, false);
    f.settle();
    assert_eq!([f.rect(first), f.rect(second)], before);
}
