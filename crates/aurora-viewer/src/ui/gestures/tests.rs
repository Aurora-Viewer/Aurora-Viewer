use super::*;
use aurora_assets::gesture::Step;

#[test]
fn cascade_gesture_editors_follow_open_order_instead_of_uuid_sort_order() {
    let mut f = Fixture::new();
    let first = Uuid::from_u128(8106);
    let second = Uuid::from_u128(8006);
    let mut other = f.inv.items[&first].clone();
    other.id = second;
    f.inv.add_items(vec![other]);
    // Both requests before a frame: the BTreeMap must not reverse creation order.
    f.state.open(&f.inv, first);
    f.state.open(&f.inv, second);
    f.settle();
    let rect = |id| {
        f.ctx
            .memory(|m| m.area_rect(egui::Id::unique(format!("gesture_{id}"))).expect("gesture editor"))
    };
    assert!(((rect(second).min - rect(first).min) - Vec2::splat(32.0)).length() < 1.0);
}

struct Fixture {
    ctx: egui::Context,
    inv: Inventory,
    manager: Manager,
    state: GesturesUi,
    prefs: Preferences,
}
impl Fixture {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let (inv, manager) = Self::library();
        let mut fixture = Self {
            ctx,
            inv,
            manager,
            state: GesturesUi::default(),
            prefs: Preferences::default(),
        };
        fixture.settle();
        fixture
    }
    /// The demo inventory, with a chat gesture 8106.
    fn library() -> (Inventory, Manager) {
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let id = Uuid::from_u128(8106);
        let definition = Gesture {
            trigger: "/hello".into(),
            key: 0xa8,
            steps: vec![Step::Chat {
                text: "/28574 T4".into(),
                flags: 0,
            }],
            ..Default::default()
        };
        let mut manager = Manager::default();
        manager.entries.insert(
            id,
            crate::gestures::Entry {
                asset: inv.items[&id].asset_id,
                definition,
            },
        );
        (inv, manager)
    }
    fn draw(&mut self, events: Vec<egui::Event>) -> (Vec<Action>, Vec<egui::epaint::ClippedShape>) {
        let mut out = Vec::new();
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                out = show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &self.inv,
                    &self.manager,
                    &mut self.state,
                    &mut self.prefs,
                    &mut true,
                );
            },
        );
        output.textures_delta.clear();
        (out, output.shapes)
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
        let pos = self.draw(Vec::new()).1.iter().find_map(|s| position(&s.shape, label)).expect(label);
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
    fn editor(&mut self) {
        let id = Uuid::from_u128(8106);
        self.state.open(&self.inv, id);
        self.state
            .editors
            .get_mut(&id)
            .expect("editor")
            .loaded(self.manager.entries[&id].definition.clone());
        self.settle();
    }
}

#[test]
fn manager_filters_selection_play_and_activation() {
    let mut f = Fixture::new();
    let id = Uuid::from_u128(8106);
    let name = f.inv.items[&id].name.clone();
    f.click(&name);
    assert_eq!(f.state.selected, BTreeSet::from([id]));
    assert!(matches!(&f.click(aurora_i18n::tr!("common-play"))[..], [Action::Play(value)] if *value == id));
    f.click(aurora_i18n::tr!("gesture-only-active"));
    assert!(f.prefs.only_active);
    f.click(aurora_i18n::tr!("gesture-enabled"));
    assert!(!f.prefs.enabled);
}

#[test]
fn editor_save_and_preview_carry_draft_without_mutating_loaded_asset() {
    let mut f = Fixture::new();
    f.editor();
    let id = Uuid::from_u128(8106);
    let e = f.state.editors.get_mut(&id).expect("editor");
    e.definition.as_mut().expect("definition").trigger = "/changed".into();
    assert!(
        matches!(&f.click(aurora_i18n::tr!("gesture-save"))[..], [Action::Save { item, definition, close: false, .. }] if *item == id && definition.trigger == "/changed")
    );
    assert_eq!(f.manager.entries[&id].definition.trigger, "/hello");
    assert!(f.state.editors[&id].dirty());
    let preview = f.state.editors[&id].preview;
    assert!(
        matches!(&f.click(aurora_i18n::tr!("gesture-preview"))[..], [Action::Preview(value, definition)] if *value == preview && definition.trigger == "/changed")
    );
}

#[test]
fn clicking_another_name_selects_its_row_instead_of_selecting_text() {
    let mut f = Fixture::new();
    let item = Uuid::from_u128(951005);
    let mut gesture = f.inv.items[&Uuid::from_u128(8106)].clone();
    gesture.id = item;
    gesture.name = "Z other gesture".into();
    f.inv.add_items(vec![gesture]);
    f.settle();
    f.click("Z other gesture");
    assert_eq!(f.state.selected, BTreeSet::from([item]));
}

#[test]
fn pending_save_disables_duplicate_requests_and_failure_keeps_draft() {
    let mut f = Fixture::new();
    f.editor();
    let id = Uuid::from_u128(8106);
    let e = f.state.editors.get_mut(&id).expect("editor");
    e.definition.as_mut().expect("definition").replacement = "custom".into();
    e.saving = Some(Uuid::from_u128(42));
    assert!(f.click(aurora_i18n::tr!("gesture-save")).is_empty());
    let e = f.state.editors.get_mut(&id).expect("editor");
    e.saving = None;
    e.message = "Rejected".into();
    assert!(f.state.editors[&id].dirty());
    assert_eq!(f.state.editors[&id].definition.as_ref().expect("draft").replacement, "custom");
}

#[test]
fn unsaved_close_cancel_and_discard() {
    let mut f = Fixture::new();
    f.editor();
    let id = Uuid::from_u128(8106);
    let e = f.state.editors.get_mut(&id).expect("editor");
    e.description = "changed".into();
    f.state.discard = Some(id);
    f.settle();
    f.click(aurora_i18n::tr!("common-cancel"));
    assert!(f.state.discard.is_none());
    assert!(f.state.editors[&id].dirty());
    f.state.discard = Some(id);
    f.settle();
    f.click(aurora_i18n::tr!("gesture-discard"));
    assert!(!f.state.editors.contains_key(&id));
}

#[test]
fn per_account_defaults_match_firestorm() {
    let mut settings = Settings::default();
    let id = Uuid::from_u128(1);
    settings.accounts.entry(id).or_default().enabled = false;
    assert!(!settings.accounts[&id].enabled);
    assert!(!settings.accounts.contains_key(&Uuid::from_u128(2)));
    assert!(Preferences::default().enabled);
    assert!(!Preferences::default().only_active);
    assert_eq!(crate::gestures::builtins::ANIMATIONS.len(), 69);
}

#[test]
fn closing_editor_stops_only_its_preview_and_keeps_active_playback() {
    let mut f = Fixture::new();
    f.editor();
    let item = Uuid::from_u128(8106);
    let preview = f.state.editors[&item].preview;
    assert_ne!(preview, item);
    let definition = f.manager.entries[&item].definition.clone();
    f.manager.play(item, definition, None);
    f.state.editors.get_mut(&item).expect("editor").open = false;
    assert!(matches!(&f.draw(Vec::new()).0[..], [Action::Stop(id)] if *id == preview));
    assert!(f.manager.is_playing(item));
}

/// The manager keeps its size with every line under its list (an empty
/// filter result, a message).
#[test]
fn the_manager_keeps_its_size() {
    let (inv, manager) = Fixture::library();
    let message = "The gesture could not be saved: the inventory did not answer in time.";
    crate::ui::widgets::steady_tests::assert_window_steady(
        "gestures",
        &[Vec2::new(460.0, 510.0), Vec2::new(300.0, 360.0)],
        message,
        || {
            let state = GesturesUi {
                message: message.into(),
                ..Default::default()
            };
            (state, Preferences::default())
        },
        |(state, prefs), ctx, p| {
            show(ctx, p, &inv, &manager, state, prefs, &mut true);
        },
    );
}

/// A key that a build shortcut keeps says so under the shortcut (#915):
/// Ctrl+B always, Ctrl+Z only while the build tools are open, F8 never.
#[test]
fn the_editor_warns_about_a_key_the_build_tools_keep() {
    let taken = |key, mask| super::editor::shortcut_taken(key, mask);
    assert_eq!(
        taken(b'B', 1).as_deref(),
        Some(aurora_i18n::tr!("gesture-shortcut-taken", shortcut = "Ctrl+B").as_ref())
    );
    assert_eq!(
        taken(b'Z', 1).as_deref(),
        Some(aurora_i18n::tr!("gesture-shortcut-taken-building", shortcut = "Ctrl+Z").as_ref())
    );
    assert_eq!(taken(0xa8, 0), None);
    assert_eq!(taken(b'Z', 0), None);
    let mut f = Fixture::new();
    f.editor();
    let id = Uuid::from_u128(8106);
    let g = f
        .state
        .editors
        .get_mut(&id)
        .expect("editor")
        .definition
        .as_mut()
        .expect("definition");
    g.key = b'B';
    g.mask = 1;
    let warning = taken(b'B', 1).expect("Ctrl+B is taken");
    fn has(shape: &egui::Shape, text: &str) -> bool {
        match shape {
            egui::Shape::Text(t) => t.galley.text() == text,
            egui::Shape::Vec(shapes) => shapes.iter().any(|s| has(s, text)),
            _ => false,
        }
    }
    assert!(f.draw(Vec::new()).1.iter().any(|s| has(&s.shape, &warning)), "the warning is drawn");
}

/// An editor keeps its size with a wait step chosen, a save under way, a
/// message and the shortcut warning, its tallest content.
#[test]
fn the_editor_keeps_its_size() {
    let (inv, manager) = Fixture::library();
    let id = Uuid::from_u128(8106);
    let message = "The gesture could not be saved: the inventory did not answer in time.";
    crate::ui::widgets::steady_tests::assert_window_steady(
        &format!("gesture_{id}"),
        &[Vec2::new(400.0, 590.0), Vec2::new(320.0, 420.0)],
        message,
        || {
            let mut state = GesturesUi::default();
            state.open(&inv, id);
            let editor = state.editors.get_mut(&id).expect("editor");
            editor.loaded(Gesture {
                key: b'B',
                mask: 1,
                steps: vec![Step::Wait { seconds: 1.0, flags: 0 }],
                ..Default::default()
            });
            editor.step = Some(0);
            editor.saving = Some(Uuid::from_u128(1));
            editor.message = message.into();
            (state, Preferences::default())
        },
        |(state, prefs), ctx, p| {
            show(ctx, p, &inv, &manager, state, prefs, &mut true);
        },
    );
}
