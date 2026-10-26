use super::*;

fn text_rect(shape: &egui::Shape, label: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == label && !t.galley.elided => Some(Rect::from_min_size(t.pos, t.galley.size())),
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
    fn new() -> Self {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        Self {
            ctx,
            p: theme.palette(),
            ui: BlacklistUi::default(),
            list: Derender::default(),
            time: 0.0,
        }
    }

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

    fn click_output(&mut self, label: &str, button: egui::PointerButton) -> (egui::FullOutput, Option<BlacklistAction>) {
        let (out, _) = self.frame(Vec::new());
        let pos = out
            .shapes
            .iter()
            .find_map(|s| text_rect(&s.shape, label))
            .unwrap_or_else(|| panic!("missing blacklist control: {label}"))
            .center();
        let event = |pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    pressed,
                    button,
                    modifiers: Default::default(),
                },
            ]
        };
        self.frame(event(true));
        self.frame(event(false))
    }

    fn click(&mut self, label: &str) -> Option<BlacklistAction> {
        self.click_output(label, egui::PointerButton::Primary).1
    }
}

#[test]
fn blacklist_sound_uuid_is_visible_and_copyable_without_confusing_it_with_its_owner() {
    let asset = Uuid::parse_str("fedcba98-7654-4321-9abc-0123456789ab").unwrap();
    for zoom in [1.0, 1.5] {
        let mut h = Harness::new();
        h.ctx.set_zoom_factor(zoom);
        widgets::place_floater(&h.ctx, WINDOW, egui::pos2(40.0, 40.0), Vec2::new(680.0, 320.0));
        h.list.add_sound(
            asset,
            Entry {
                name: "Demo owner".into(),
                region: String::new(),
                kind: Kind::Sound(0),
                date: 0,
                permanent: true,
            },
        );
        for _ in 0..6 {
            h.frame(Vec::new());
        }
        let (out, _) = h.frame(Vec::new());
        assert!(
            out.shapes.iter().any(|s| text_rect(&s.shape, &asset.to_string()).is_some()),
            "complete asset UUID at zoom {zoom}"
        );
        h.click_output(&asset.to_string(), egui::PointerButton::Secondary);
        let (out, _) = h.click_output(aurora_i18n::tr!("build-ui-general-copy-uuid"), egui::PointerButton::Primary);
        assert!(
            out.platform_output
                .commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(s) if *s == asset.to_string()))
        );
        assert_eq!(h.click(aurora_i18n::tr!("blacklist-ignored")), Some(BlacklistAction::OpenIgnored));
    }
}

#[test]
fn blacklist_copying_multiple_categories_copies_the_owner_uuid_once() {
    let owner = Uuid::from_u128(45);
    let mut h = Harness::new();
    widgets::place_floater(&h.ctx, WINDOW, egui::pos2(40.0, 40.0), Vec2::new(680.0, 320.0));
    h.list.add_sound(
        owner,
        Entry {
            name: "Demo owner".into(),
            region: String::new(),
            kind: Kind::Sound(7),
            date: 0,
            permanent: true,
        },
    );
    for _ in 0..6 {
        h.frame(Vec::new());
    }
    let (out, _) = h.frame(Vec::new());
    for label in [
        aurora_i18n::tr!("sound-explorer-worn"),
        aurora_i18n::tr!("sound-explorer-rezzed"),
        aurora_i18n::tr!("sound-explorer-gestures"),
    ] {
        assert!(
            out.shapes.iter().any(|s| text_rect(&s.shape, label).is_some()),
            "complete category label: {label}"
        );
    }
    h.ui.selected = [1, 2, 4].map(|flag| RowId { id: owner, flag }).into_iter().collect();
    h.click_output(aurora_i18n::tr!("sound-explorer-worn"), egui::PointerButton::Secondary);
    let (out, _) = h.click_output(aurora_i18n::tr!("build-ui-general-copy-uuid"), egui::PointerButton::Primary);
    assert!(
        out.platform_output
            .commands
            .iter()
            .any(|c| matches!(c, egui::OutputCommand::CopyText(s) if *s == owner.to_string()))
    );
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
