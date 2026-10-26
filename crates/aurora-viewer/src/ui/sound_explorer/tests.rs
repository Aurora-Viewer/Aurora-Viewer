use super::*;
use glam::DVec3;
use std::time::Duration;
use uuid::Uuid;

fn row(id: u64, asset: u128, avatar: bool, stopped: bool) -> Row {
    let now = Instant::now();
    Row {
        owner: format!("Owner {id}"),
        entry: Entry {
            id,
            asset: Uuid::from_u128(asset),
            source: Uuid::from_u128(if avatar { 10 } else { 20 }),
            owner: Uuid::from_u128(10),
            position: DVec3::ZERO,
            started: now - Duration::from_secs(60),
            stopped: stopped.then_some(now),
            triggered: avatar,
            looped: !avatar,
        },
    }
}

#[test]
fn sound_explorer_filters_choose_the_newest_visible_asset_and_keep_playing_first() {
    let ui = SoundExplorer {
        rows: vec![
            row(1, 100, false, true),
            row(2, 100, false, false),
            row(3, 200, true, false),
            row(4, crate::scene::sounds::COLLISION[30], false, false),
        ],
        ..Default::default()
    };
    let mut f = Filters::default();
    assert_eq!(ui.visible(&f), [1]);
    f.avatars = true;
    assert_eq!(ui.visible(&f), [2, 1]);
    f.repeats = true;
    assert_eq!(ui.visible(&f), [2, 1, 0]);
    f.collisions = true;
    assert_eq!(ui.visible(&f), [3, 2, 1, 0]);
    f.objects = false;
    assert_eq!(ui.visible(&f), [2]);
    let encoded = serde_json::to_string(&f).unwrap();
    assert_eq!(serde_json::from_str::<Filters>(&encoded).unwrap(), f);
}

#[test]
fn sound_explorer_selection_extends_in_display_order_and_toggles() {
    let mut ui = SoundExplorer::default();
    let order = [4, 3, 2, 1];
    ui.click(3, &order, Default::default());
    ui.click(
        1,
        &order,
        egui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(ui.selected, HashSet::from([3, 2, 1]));
    ui.click(
        2,
        &order,
        egui::Modifiers {
            command: true,
            ..Default::default()
        },
    );
    assert_eq!(ui.selected, HashSet::from([3, 1]));
}

#[test]
fn sound_explorer_history_tracks_stops_and_bounds_stopped_rows_without_evicting_loops() {
    let mut history = crate::scene::sounds::history::History::default();
    let active = row(1, 100, false, false).entry;
    let now = Instant::now();
    history.sync(std::iter::once(active.clone()), now);
    for id in 2..400 {
        history.sync([active.clone(), row(id, id as u128, false, false).entry].into_iter(), now);
    }
    assert_eq!(history.entries().count(), 256);
    assert!(history.entries().any(|e| e.id == 1 && e.stopped.is_none()));
    history.sync(std::iter::empty(), now);
    assert!(history.entries().all(|e| e.stopped == Some(now)));
}

fn text_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == label => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, label)),
        _ => None,
    }
}

struct Harness {
    ctx: egui::Context,
    p: Palette,
    explorer: SoundExplorer,
    filters: Filters,
    time: f64,
}

impl Harness {
    fn new(scale: f32) -> Self {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, scale);
        let mut h = Self {
            ctx,
            p: theme.palette(),
            explorer: SoundExplorer::default(),
            filters: Filters::default(),
            time: 0.0,
        };
        h.explorer.rows = vec![row(1, 100, false, false), row(2, 200, false, true)];
        for _ in 0..6 {
            h.frame(Vec::new());
        }
        h
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, Option<Action>) {
        self.time += 0.1;
        let mut action = None;
        let out = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1100.0, 760.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |_| {
                action = self.explorer.show(&self.ctx, &self.p, &mut true, &mut self.filters, true);
            },
        );
        (crate::ui::login::test_output(out), action)
    }

    fn click(&mut self, label: &str) -> Option<Action> {
        let (out, _) = self.frame(Vec::new());
        let pos = out
            .shapes
            .iter()
            .find_map(|s| text_rect(&s.shape, label))
            .unwrap_or_else(|| panic!("missing sound explorer control {label}"))
            .center();
        let mut action = None;
        for pressed in [true, false] {
            action = self
                .frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ])
                .1;
        }
        action
    }
}

#[test]
fn sound_explorer_native_widgets_select_pause_filter_and_dispatch_all_actions() {
    for scale in [1.0, 1.5] {
        let mut h = Harness::new(scale);
        h.click("Owner 1");
        assert_eq!(h.explorer.selected, HashSet::from([1]));
        assert_eq!(h.click(aurora_i18n::tr!("sound-explorer-play")), Some(Action::Play));
        assert_eq!(h.click(aurora_i18n::tr!("sound-explorer-stop-local")), Some(Action::StopLocal));
        assert_eq!(h.click(aurora_i18n::tr!("sound-explorer-look")), Some(Action::LookAt(1)));
        assert_eq!(h.click(aurora_i18n::tr!("common-stop")), Some(Action::Stop));
        assert_eq!(h.click(aurora_i18n::tr!("sound-explorer-blacklist")), Some(Action::Blacklist(0)));
        assert_eq!(h.click(aurora_i18n::tr!("sound-explorer-block-worn")), Some(Action::Blacklist(1)));
        assert_eq!(h.click(aurora_i18n::tr!("sound-explorer-block-rezzed")), Some(Action::Blacklist(2)));
        assert_eq!(
            h.click(aurora_i18n::tr!("sound-explorer-block-gestures")),
            Some(Action::Blacklist(4))
        );
        h.click(aurora_i18n::tr!("sound-explorer-pause"));
        assert!(h.explorer.paused);
        h.click(aurora_i18n::tr!("sound-explorer-objects"));
        assert!(!h.filters.objects);
        h.frame(Vec::new());
        assert!(h.explorer.selected.is_empty(), "hidden rows cannot retain actionable selection");
        assert_eq!(h.click(aurora_i18n::tr!("sound-explorer-play")), None);
        assert_eq!(h.click(aurora_i18n::tr!("blacklist-title")), Some(Action::BlacklistWindow));
    }
}
