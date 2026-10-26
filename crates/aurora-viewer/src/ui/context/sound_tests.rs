use super::*;
use crate::world::derender::{Derender, Entry as BlockEntry, Kind as BlockKind};
use entries::{Entry, Kind};

fn sound_menu(mask: u8, attachment: Option<Uuid>) -> Vec<Entry> {
    let target = Target::Avatar {
        id: Uuid::from_u128(7),
        own: false,
        attachment,
    };
    let facts = Facts {
        sound_blocks: mask,
        ..Default::default()
    };
    let menu = entries::world_menu(&target, &facts, Vec3::ZERO);
    let entry = menu
        .into_iter()
        .find(|e| e.label == aurora_i18n::tr!("ui-context-disturbance"))
        .unwrap();
    let Kind::Submenu(rows) = entry.kind else { panic!("not a submenu") };
    rows
}

fn label(flag: u8) -> &'static str {
    match flag {
        1 => aurora_i18n::tr!("ui-context-block-worn-sounds"),
        2 => aurora_i18n::tr!("ui-context-block-rezzed-sounds"),
        _ => aurora_i18n::tr!("ui-context-block-gesture-sounds"),
    }
}

#[test]
fn avatar_and_attachment_disturbance_menus_show_each_category_state_and_inverse_action() {
    for attachment in [None, Some(Uuid::from_u128(8))] {
        for mask in 0..=7 {
            let entries = sound_menu(mask, attachment);
            let rows: Vec<_> = entries
                .iter()
                .filter(|e| matches!(e.kind, Kind::Action(CtxAction::SetOwnerSoundBlock { .. })))
                .collect();
            assert_eq!(rows.len(), 3);
            for (row, flag) in rows.iter().zip([1, 2, 4]) {
                let on = mask & flag != 0;
                assert!(row.enabled);
                assert_eq!(row.checked, Some(on));
                assert_eq!(row.label, label(flag));
                assert!(
                    matches!(row.kind, Kind::Action(CtxAction::SetOwnerSoundBlock { owner, flag: f, blocked }) if owner == Uuid::from_u128(7) && f == flag && blocked != on)
                );
            }
        }
    }
}

fn text_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == label && !t.galley.elided => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, label)),
        _ => None,
    }
}

struct Harness {
    ctx: egui::Context,
    p: Palette,
    list: Derender,
    time: f64,
    open: bool,
}

impl Harness {
    fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, Vec<CtxAction>) {
        self.time += 0.1;
        let entries = sound_menu(self.list.owner_sound_flags(Uuid::from_u128(7)), None);
        let output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |_| {
                menu::popup_at(
                    &self.ctx,
                    egui::Id::unique("sound_menu_test"),
                    egui::pos2(40.0, 40.0),
                    &mut self.open,
                    &self.p,
                    |ui| {
                        entries::draw_list(ui, &self.p, &entries);
                    },
                );
            },
        );
        (crate::ui::login::test_output(output), take_requests(&self.ctx))
    }

    fn click(&mut self, flag: u8, blocked: bool) {
        let (out, _) = self.frame(Vec::new());
        let pos = out
            .shapes
            .iter()
            .find_map(|s| text_rect(&s.shape, label(flag)))
            .expect("complete sound control label")
            .center();
        let event = |pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: Default::default(),
                },
            ]
        };
        self.frame(event(true));
        let (_, actions) = self.frame(event(false));
        assert_eq!(actions.len(), 1);
        let CtxAction::SetOwnerSoundBlock {
            owner,
            flag: actual,
            blocked: next,
        } = actions[0]
        else {
            panic!("wrong action")
        };
        assert_eq!(actual, flag);
        assert_ne!(next, blocked);
        self.list.set_sound_category(
            owner,
            actual,
            next,
            BlockEntry {
                name: "Demo owner".into(),
                region: String::new(),
                kind: BlockKind::Sound(actual),
                date: 0,
                permanent: false,
            },
        );
        assert!(self.open, "the menu stays open for another category");
    }
}

#[test]
fn sound_menu_controls_toggle_independently_without_closing_the_menu() {
    for zoom in [1.0, 1.5] {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        ctx.set_zoom_factor(zoom);
        let mut h = Harness {
            ctx,
            p: theme.palette(),
            list: Derender::default(),
            time: 0.0,
            open: true,
        };
        for _ in 0..6 {
            h.frame(Vec::new());
        }
        for flag in [1, 2, 4] {
            h.click(flag, false);
        }
        assert_eq!(h.list.owner_sound_flags(Uuid::from_u128(7)), 7);
        h.click(2, true);
        assert_eq!(h.list.owner_sound_flags(Uuid::from_u128(7)), 5);
        h.click(1, true);
        h.click(4, true);
        assert_eq!(h.list.owner_sound_flags(Uuid::from_u128(7)), 0);
    }
}
