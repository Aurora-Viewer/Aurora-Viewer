use super::*;

fn text_rect(shape: &egui::Shape, label: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == label => Some(Rect::from_min_size(t.pos, t.galley.size())),
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, label)),
        _ => None,
    }
}

struct Harness {
    ctx: egui::Context,
    p: Palette,
    ui: BlacklistUi,
    list: Derender,
    time: f64,
}

impl Harness {
    fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, Option<BlacklistAction>) {
        self.time += 0.1;
        let mut action = None;
        let output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |_| {
                action = self.ui.show(&self.ctx, &self.p, &self.list, &mut true);
            },
        );
        (crate::ui::login::test_output(output), action)
    }

    fn click(&mut self, label: &str) -> Option<BlacklistAction> {
        let (out, _) = self.frame(Vec::new());
        let pos = out
            .shapes
            .iter()
            .find_map(|s| text_rect(&s.shape, label))
            .unwrap_or_else(|| panic!("missing blacklist control: {label}"))
            .center();
        let mut action = None;
        for pressed in [true, false] {
            action = self
                .frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: Default::default(),
                    },
                ])
                .1;
        }
        action
    }
}

#[test]
fn blacklist_sound_category_row_removes_only_the_clicked_category() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let owner = Uuid::from_u128(45);
    let mut h = Harness {
        ctx,
        p: theme.palette(),
        ui: BlacklistUi::default(),
        list: Derender::default(),
        time: 0.0,
    };
    h.list.add_sound(
        owner,
        Entry {
            name: "Demo resident".into(),
            region: String::new(),
            kind: Kind::Sound(7),
            date: 0,
            permanent: false,
        },
    );
    for _ in 0..6 {
        h.frame(Vec::new());
    }
    h.click(aurora_i18n::tr!("sound-explorer-rezzed"));
    let row = RowId { id: owner, flag: 2 };
    assert_eq!(h.ui.selected, HashSet::from([row]));
    assert_eq!(
        h.click(aurora_i18n::tr!("blacklist-remove-selected")),
        Some(BlacklistAction::Remove(vec![row]))
    );
    h.list.remove_rows(&[row]);
    let (out, _) = h.frame(Vec::new());
    for label in [aurora_i18n::tr!("sound-explorer-worn"), aurora_i18n::tr!("sound-explorer-gestures")] {
        assert!(
            out.shapes.iter().any(|s| text_rect(&s.shape, label).is_some()),
            "remaining category: {label}"
        );
    }
    assert!(!h.list.sound_blocked(Uuid::nil(), owner, 2));
    assert!(h.list.sound_blocked(Uuid::nil(), owner, 1));
    assert!(h.list.sound_blocked(Uuid::nil(), owner, 4));
}
