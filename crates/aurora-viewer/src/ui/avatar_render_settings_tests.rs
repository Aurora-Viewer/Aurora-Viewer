use super::*;
use egui::Rect;

fn row(n: u128, name: &str, mode: u8) -> Row {
    Row {
        id: Uuid::from_u128(n),
        name: name.to_owned(),
        mode,
        can_name: true,
    }
}

fn text_rect(shape: &egui::Shape, label: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == label && !t.galley.elided => Some(Rect::from_min_size(t.pos, t.galley.size())),
        egui::Shape::Vec(shapes) => shapes.iter().rev().find_map(|s| text_rect(s, label)),
        _ => None,
    }
}

struct Harness {
    ctx: egui::Context,
    p: Palette,
    editor: AvatarRenderSettings,
    world: World,
    rows: Vec<Row>,
    time: f64,
    open: bool,
}

impl Harness {
    fn new(zoom: f32) -> Self {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        ctx.set_zoom_factor(zoom);
        widgets::place_floater(&ctx, WINDOW, egui::pos2(30.0, 30.0), Vec2::new(460.0, 320.0));
        let mut h = Self {
            ctx,
            p: theme.palette(),
            editor: Default::default(),
            world: World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load())),
            rows: vec![row(11, "Alice Demo", 2), row(12, "Bob Demo", 1), row(13, "Carol Demo", 2)],
            time: 0.0,
            open: true,
        };
        for _ in 0..6 {
            h.frame(Vec::new());
        }
        h
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, Option<Change>) {
        self.time += 0.1;
        let mut change = None;
        let out = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |_| {
                change = self.editor.show(&self.ctx, &self.p, &self.rows, &mut self.world, &mut self.open);
            },
        );
        (crate::ui::login::test_output(out), change)
    }

    fn click(&mut self, label: &str, button: egui::PointerButton, modifiers: egui::Modifiers) -> Option<Change> {
        let (out, _) = self.frame(Vec::new());
        let pos = out
            .shapes
            .iter()
            .rev()
            .find_map(|s| text_rect(&s.shape, label))
            .unwrap_or_else(|| panic!("missing editor control: {label}"))
            .center();
        let events = |pressed| {
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    pressed,
                    button,
                    modifiers,
                },
            ]
        };
        self.frame(events(true));
        self.frame(events(false)).1
    }

    fn primary(&mut self, label: &str) -> Option<Change> {
        self.click(label, egui::PointerButton::Primary, Default::default())
    }
}

#[test]
fn avatar_render_settings_controls_edit_multiple_entries_and_track_external_removal() {
    for zoom in [1.0, 1.5] {
        let mut h = Harness::new(zoom);
        assert_eq!(h.primary(short_label(0)), None);
        h.primary("Alice Demo");
        h.click("Carol Demo", egui::PointerButton::Primary, egui::Modifiers::SHIFT);
        assert_eq!(
            h.primary(short_label(1)),
            Some(Change {
                ids: vec![Uuid::from_u128(11), Uuid::from_u128(12), Uuid::from_u128(13)],
                mode: 1
            })
        );
        // Right-clicking a selected row retains the multi-selection.
        h.click("Bob Demo", egui::PointerButton::Secondary, Default::default());
        assert_eq!(h.primary(label(2)).expect("menu change").ids.len(), 3);
        // A change from the ordinary avatar menu is reflected by the next frame.
        h.rows.remove(1);
        h.frame(Vec::new());
        assert!(!h.editor.selected.contains(&Uuid::from_u128(12)));
        h.editor.filter = "  CAROL  ".into();
        let change = h.primary(short_label(0)).expect("visible selection");
        assert_eq!(
            change,
            Change {
                ids: vec![Uuid::from_u128(13)],
                mode: 0
            }
        );
        h.rows.clear();
        h.frame(Vec::new());
        assert_eq!(h.primary(short_label(0)), None);
    }
}

#[test]
fn avatar_render_settings_filter_sort_and_privacy_use_current_display_names() {
    let mut editor = AvatarRenderSettings::default();
    let mut rows = vec![row(11, "Zulu Demo", 2), row(12, "Alice Demo", 1)];
    rows[0].can_name = false;
    editor.filter = rows[0].id.to_string();
    assert!(editor.rows(&rows).is_empty(), "anonymous UUID cannot be searched");
    editor.filter = rows[1].id.to_string();
    assert_eq!(editor.rows(&rows)[0].name, "Alice Demo");
    editor.filter.clear();
    editor.sort_setting = true;
    assert_eq!(editor.rows(&rows)[0].mode, 1);
    editor.descending = true;
    assert_eq!(editor.rows(&rows)[0].mode, 2);
    editor.filter = " zUlU ".into();
    assert_eq!(editor.rows(&rows).len(), 1);
}

#[test]
fn avatar_render_settings_picker_closes_with_its_parent_and_keeps_the_chosen_mode() {
    let mut h = Harness::new(1.0);
    h.primary(aurora_i18n::tr!("common-add"));
    h.primary(label(1));
    assert!(h.editor.picker.open);
    assert_eq!(h.editor.add_mode, 1);
    h.open = false;
    h.frame(Vec::new());
    assert!(!h.editor.picker.open);
}

#[test]
fn avatar_render_settings_picker_adds_multiple_distant_friends_in_the_chosen_mode() {
    let mut h = Harness::new(1.0);
    for (n, name) in [(21, "Distant Alice"), (22, "Distant Bob")] {
        let id = Uuid::from_u128(n);
        h.world.social.names.insert(id, name.into());
        h.world.social.friends.push(crate::world::social::Friend {
            id,
            online: false,
            rights_given: 1,
            rights_has: 1,
        });
    }
    h.primary(aurora_i18n::tr!("common-add"));
    h.primary(label(2));
    h.primary(aurora_i18n::tr!("common-friends"));
    h.primary("Distant Alice");
    h.click("Distant Bob", egui::PointerButton::Primary, egui::Modifiers::CTRL);
    assert_eq!(
        h.primary(aurora_i18n::tr!("ui-avatar-picker-select")),
        Some(Change {
            ids: vec![Uuid::from_u128(21), Uuid::from_u128(22)],
            mode: 2
        })
    );
    assert!(!h.editor.picker.open);
}
