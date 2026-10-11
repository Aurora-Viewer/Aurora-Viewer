use super::*;
use aurora_net::inventory::operations::Operation;

struct Fixture {
    ctx: egui::Context,
    inv: Inventory,
    catalog: inventory::Catalog,
    runtime: Runtime,
    account: Account,
    state: AoUi,
}

impl Fixture {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = super::super::icons::Icons::load(&ctx, None);
        let mut inv = Inventory::default();
        let id = crate::ao::demo::seed(&mut inv, Uuid::from_u128(1));
        let mut catalog = inventory::Catalog::default();
        catalog.refresh(&mut inv);
        let mut fixture = Self {
            ctx,
            inv,
            catalog,
            runtime: Runtime::default(),
            account: Account {
                enabled: false,
                selected: id,
            },
            state: AoUi::default(),
        };
        fixture.settle();
        fixture
    }

    fn draw(&mut self, events: Vec<egui::Event>) -> (Vec<Action>, Vec<egui::epaint::ClippedShape>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut output = self.ctx.run_ui(input, |ui| {
            actions = show(
                ui.ctx(),
                &crate::theme::Theme::default().palette(),
                &mut self.inv,
                &self.catalog,
                &mut self.runtime,
                &mut self.account,
                &mut self.state,
                &mut true,
            );
        });
        output.textures_delta.clear();
        (actions, output.shapes)
    }

    fn settle(&mut self) {
        for _ in 0..5 {
            self.draw(Vec::new());
        }
    }

    fn click(&mut self, label: &str) -> Vec<Action> {
        fn position(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + egui::vec2(3.0, 5.0)),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| position(s, label)),
                _ => None,
            }
        }
        let shapes = self.draw(Vec::new()).1;
        let pos = shapes.iter().find_map(|s| position(&s.shape, label)).expect(label);
        let mut out = Vec::new();
        for pressed in [true, false] {
            out.extend(
                self.draw(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ])
                .0,
            );
        }
        out
    }
}

#[test]
fn enable_label_and_pause_change_runtime_without_inventory_writes() {
    let mut f = Fixture::new();
    let out = f.click(aurora_i18n::tr!("ao-enabled"));
    assert!(f.account.enabled);
    assert!(matches!(&out[..], [Action::Preferences]));
    assert!(f.click(aurora_i18n::tr!("ao-pause")).is_empty());
    assert!(f.runtime.paused);
    assert!(f.click(aurora_i18n::tr!("ao-stands")).is_empty());
    assert!(!f.runtime.stands);
}

#[test]
fn rename_updates_existing_folder_and_new_set_creates_a_distinct_set() {
    let mut f = Fixture::new();
    f.state.name = "Renamed".into();
    f.state.new_dialog = Some(SetDialog::Rename);
    f.settle();
    let out = f.click(aurora_i18n::tr!("common-save"));
    assert!(matches!(&out[..], [Action::Edit(change, None)] if matches!(&change.operations[..],
        [Operation::Patch { id, folder: true, .. }] if *id == f.account.selected)));
    assert!(f.state.new_dialog.is_none());
    f.state.name = "New set".into();
    f.state.new_dialog = Some(SetDialog::New);
    f.settle();
    let out = f.click(aurora_i18n::tr!("common-save"));
    assert!(matches!(&out[..], [Action::Create(set)] if set.name == "New set" && set.states.is_empty()));
}

#[test]
fn animation_picker_links_into_selected_state_and_import_blocks_edits() {
    let mut f = Fixture::new();
    f.state.state = State::Walking;
    let folder = f.catalog.selected(f.account.selected).expect("set").states[&State::Walking].folder;
    f.click(aurora_i18n::tr!("ao-add"));
    assert_eq!(f.state.picker, Some(false));
    f.state.filter = "Type".into();
    f.settle();
    let out = f.click("Aurora Type");
    assert!(matches!(&out[..], [Action::Edit(change, None)] if change.operations.iter().any(|op|
        matches!(op, Operation::Links { parent, links } if *parent == folder && links.len() == 1))));
    assert!(f.state.picker.is_none());
    f.state.import_folder = Some(crate::ao::demo::NOTECARD);
    f.settle();
    assert!(f.click(aurora_i18n::tr!("ao-add")).is_empty());
    assert!(f.state.picker.is_none(), "busy controls must not dispatch concurrent changes");
}
