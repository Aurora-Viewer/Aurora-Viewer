use super::*;

struct Fixture {
    ctx: egui::Context,
    theme: crate::theme::Theme,
    world: crate::world::World,
    imports: Imports,
    prefs: Preferences,
}

impl Fixture {
    fn new() -> Self {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mesh = Entry::new(
            "first mesh.dae".into(),
            Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
            Default::default(),
        );
        let texture = Entry::new(
            "second texture.png".into(),
            Ok(Source::Texture(crate::imports::tests::image(2048, 1024))),
            Default::default(),
        );
        Self {
            ctx,
            theme,
            world: crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load())),
            imports: Imports {
                open: true,
                entries: vec![mesh, texture],
                ..Default::default()
            },
            prefs: Default::default(),
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, Vec<Action>) {
        let mut actions = Vec::new();
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1440.0, 1000.0))),
                events,
                ..Default::default()
            },
            |ui| {
                actions = show(
                    ui.ctx(),
                    &self.theme.palette(),
                    &mut self.imports,
                    &mut self.prefs,
                    &self.world,
                    &Default::default(),
                );
            },
        );
        output.textures_delta.clear();
        (output, actions)
    }

    fn settle(&mut self) -> egui::FullOutput {
        for _ in 0..5 {
            self.frame(Vec::new());
        }
        self.frame(Vec::new()).0
    }

    fn click_row(&mut self, id: Uuid) {
        let rect = self
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("import_queue_row", id))))
            .expect("queue row");
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
    }
}

fn text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos),
            _ => None,
        })
        .unwrap_or_else(|| panic!("visible {label}"))
}

#[test]
fn unified_import_selects_the_first_entry_and_pins_destinations_above_the_existing_mesh_inspector() {
    let mut f = Fixture::new();
    let first = f.imports.entries[0].id;
    let output = f.settle();
    assert_eq!(f.imports.active, Some(first));
    assert!(
        text_position(&output, aurora_i18n::tr!("import-use-default-folders")).y
            < text_position(&output, aurora_i18n::tr!("common-name")).y
    );
    text_position(&output, aurora_i18n::tr!("import-mesh-section-physics"));
    text_position(&output, aurora_i18n::tr!("import-queue"));
    assert!(f.ctx.memory(|m| m.area_rect(egui::Id::unique(WINDOW))).is_some());
    assert!(f.ctx.memory(|m| m.area_rect(egui::Id::unique("import_launcher"))).is_none());
}

#[test]
fn unified_import_queue_clicks_switch_panels_without_changing_mesh_options_or_window_identity() {
    let mut f = Fixture::new();
    let mesh = f.imports.entries[0].id;
    let texture = f.imports.entries[1].id;
    f.settle();
    f.ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("mesh_section", mesh)), 1usize));
    let before = f
        .ctx
        .data_mut(|d| d.get_persisted::<Vec2>(egui::Id::unique(WINDOW).with("expanded_size")))
        .unwrap();
    f.imports.tab = Tab::Folder;
    f.settle();
    f.click_row(texture);
    let output = f.settle();
    assert_eq!(f.imports.active, Some(texture));
    assert_eq!(f.imports.tab, Tab::Batch);
    text_position(&output, aurora_i18n::tr!("import-max-size"));
    assert!(
        !output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == aurora_i18n::tr!("import-mesh-section-physics")))
    );
    f.imports.tab = Tab::Permissions;
    f.settle();
    f.click_row(mesh);
    assert_eq!(f.imports.tab, Tab::Batch);
    let output = f.settle();
    text_position(&output, aurora_i18n::tr!("import-mesh-section-physics"));
    f.imports.tab = Tab::Folder;
    f.settle();
    f.click_row(mesh);
    assert_eq!(f.imports.tab, Tab::Batch, "clicking the already active item also opens Files");
    assert_eq!(
        f.ctx.data(|d| d.get_temp::<usize>(egui::Id::unique(("mesh_section", mesh)))),
        Some(1)
    );
    assert_eq!(
        f.ctx
            .data_mut(|d| d.get_persisted::<Vec2>(egui::Id::unique(WINDOW).with("expanded_size"))),
        Some(before)
    );
}

#[test]
fn unified_import_filters_keep_upload_selection_and_reconcile_the_visible_active_entry() {
    let mut f = Fixture::new();
    f.imports.entries[0].name = "Zulu mesh".into();
    f.imports.entries[1].name = "Alpha texture".into();
    f.settle();
    f.imports.type_filters[4] = false;
    f.imports.reconcile_active();
    assert_eq!(f.imports.visible_indices(), vec![1]);
    assert_eq!(f.imports.active, Some(f.imports.entries[1].id));
    assert!(f.imports.entries.iter().all(|e| e.selected));
    assert!(f.imports.entries.iter().all(|e| Choice::Bulk.accepts(e)));
    f.imports.type_filters[4] = true;
    f.imports.reconcile_active();
    assert_eq!(f.imports.visible_indices(), vec![0, 1]);
}

#[test]
fn unified_import_shortcut_opens_one_window_without_opening_a_native_picker() {
    let mut f = Fixture::new();
    f.imports.open = false;
    f.imports.toggle_window(&f.ctx);
    assert!(f.imports.open);
    let (_, actions) = f.frame(Vec::new());
    assert!(!actions.iter().any(|a| matches!(a, Action::Files)));
    let mut windows = std::collections::HashSet::new();
    for choice in [
        Choice::Texture,
        Choice::Sound,
        Choice::Animation,
        Choice::Material,
        Choice::Mesh,
        Choice::Bulk,
        Choice::Local,
    ] {
        assert!(windows.insert(choice.window_id()), "each type has independent geometry");
    }
    assert_eq!(Choice::Bulk.window_id(), WINDOW);
    f.imports.toggle_window(&f.ctx);
    assert!(!f.imports.open);
}

fn click_launcher(f: &mut Fixture, choice: Choice) -> Vec<Action> {
    f.settle();
    let pos = f.ctx.data(|data| {
        data.get_temp::<egui::Rect>(egui::Id::unique(("import_launcher_choice", choice as usize)))
            .expect("launcher icon")
            .center()
    });
    let mut actions = Vec::new();
    for pressed in [true, false] {
        actions.extend(
            f.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ])
            .1,
        );
    }
    actions
}

#[test]
fn import_icon_bar_preference_round_trips_and_toggles_independently_of_the_queue() {
    let mut f = Fixture::new();
    let defaults: Preferences = serde_json::from_str("{}").unwrap();
    assert!(!defaults.icon_bar);
    f.prefs.icon_bar = true;
    let saved = serde_json::to_string(&f.prefs).unwrap();
    f.prefs = serde_json::from_str(&saved).unwrap();
    assert!(f.prefs.icon_bar);
    let ids: Vec<_> = f.imports.entries.iter().map(|e| e.id).collect();
    f.imports.open = false;
    f.imports.toggle_import(&f.ctx, &f.prefs);
    let (_, actions) = f.frame(Vec::new());
    assert!(f.imports.launcher_open);
    assert!(!f.imports.open);
    assert!(actions.is_empty());
    assert!(matches!(click_launcher(&mut f, Choice::Bulk).as_slice(), [Action::Folder]));
    assert!(f.imports.open);
    f.imports.toggle_import(&f.ctx, &f.prefs);
    assert!(!f.imports.launcher_open);
    assert!(f.imports.open);
    assert_eq!(f.imports.entries.iter().map(|e| e.id).collect::<Vec<_>>(), ids);
}

#[test]
fn unified_import_switch_updates_the_saved_entry_point_and_bar_without_changing_the_queue() {
    let mut f = Fixture::new();
    let output = f.settle();
    let pos = text_position(&output, aurora_i18n::tr!("import-icon-bar")) + egui::vec2(5.0, 5.0);
    for pressed in [true, false] {
        let (_, actions) = f.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert!(actions.is_empty());
    }
    assert!(f.imports.launcher_open && f.imports.open);
    assert!(f.prefs.icon_bar);
    assert_eq!(f.imports.entries.len(), 2);
    f.settle();
    let mut first = None::<egui::Rect>;
    for choice in [
        Choice::Texture,
        Choice::Sound,
        Choice::Animation,
        Choice::Material,
        Choice::Mesh,
        Choice::Bulk,
        Choice::Local,
    ] {
        let rect = f.ctx.data(|data| {
            data.get_temp::<egui::Rect>(egui::Id::unique(("import_launcher_choice", choice as usize)))
                .unwrap()
        });
        if let Some(first) = first {
            assert_eq!(rect.center().y, first.center().y, "all types share one row");
            assert!(rect.left() > first.right());
        } else {
            first = Some(rect);
        }
    }
    let output = f.settle();
    let pos = text_position(&output, aurora_i18n::tr!("import-icon-bar")) + egui::vec2(5.0, 5.0);
    for pressed in [true, false] {
        let (_, actions) = f.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert!(actions.is_empty());
    }
    assert!(!f.prefs.icon_bar);
    assert!(!f.imports.launcher_open);
    assert!(f.imports.open);
    assert_eq!(f.imports.entries.len(), 2);
    f.imports.open = false;
    f.imports.toggle_import(&f.ctx, &f.prefs);
    assert!(f.imports.open && !f.imports.launcher_open, "off restores the window entry point");
}

#[test]
fn import_icon_bar_reuses_typed_drafts_and_only_empty_types_request_a_filtered_picker() {
    let mut f = Fixture::new();
    f.imports.open = false;
    f.imports.launcher_open = true;
    let mesh = f.imports.entries[0].id;
    assert!(click_launcher(&mut f, Choice::Mesh).is_empty());
    assert!(!f.imports.open);
    assert!(f.imports.window_open(Choice::Mesh));
    f.imports.with_window(Choice::Mesh, |imports| {
        assert_eq!(imports.active, Some(mesh));
        assert_eq!(imports.visible_indices(), vec![0]);
    });
    assert!(f.imports.entries.iter().all(|e| e.selected));
    for choice in [Choice::Sound, Choice::Animation, Choice::Material, Choice::Local] {
        let actions = click_launcher(&mut f, choice);
        assert!(matches!(actions.as_slice(), [Action::Choose(picked)] if *picked == choice));
        assert!(!f.imports.open);
        assert!(f.imports.window_open(choice));
        assert!(f.imports.launcher_open);
    }
    assert!(click_launcher(&mut f, Choice::Texture).is_empty());
    let texture = f.imports.entries[1].id;
    f.imports.with_window(Choice::Texture, |imports| {
        assert_eq!(imports.active, Some(texture));
        assert_eq!(imports.visible_indices(), vec![1]);
    });
    assert!(f.imports.window_open(Choice::Mesh) && f.imports.window_open(Choice::Texture));
    f.settle();
    let mesh_rect = f
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(Choice::Mesh.window_id())))
        .unwrap();
    let texture_rect = f
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(Choice::Texture.window_id())))
        .unwrap();
    assert_ne!(mesh_rect, texture_rect);
    assert!(matches!(click_launcher(&mut f, Choice::Bulk).as_slice(), [Action::Folder]));
    assert!(f.imports.open);
    assert_eq!(f.imports.visible_indices(), vec![0, 1]);
    assert_eq!(f.imports.entries.len(), 2);
}

#[test]
fn import_icon_bar_blocks_another_picker_while_loading_and_escape_keeps_drafts() {
    let mut f = Fixture::new();
    f.imports.open = false;
    f.imports.launcher_open = true;
    f.imports.pending_files = Some(Uuid::new_v4());
    assert!(click_launcher(&mut f, Choice::Sound).is_empty());
    assert!(!f.imports.open);
    f.imports.windows[Choice::Sound.window_index().unwrap()].open = false;
    f.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(!f.imports.launcher_open);
    assert_eq!(f.imports.entries.len(), 2);
    assert!(f.imports.pending_files.is_some());
}

#[test]
fn type_queue_actions_wrap_between_buttons_with_large_text_instead_of_breaking_their_labels() {
    let mut f = Fixture::new();
    f.theme.apply(&f.ctx, 1.5);
    f.imports.open = false;
    f.imports.open_type_window(&f.ctx, Choice::Texture);
    let output = f.settle();
    for label in [aurora_i18n::tr!("import-reload"), aurora_i18n::tr!("common-remove")] {
        let text = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some(text),
                _ => None,
            })
            .expect("queue action label");
        assert_eq!(text.galley.rows.len(), 1, "{label} stays on one line");
    }
}

#[test]
fn type_window_confirmation_keeps_its_price_and_action_when_another_type_is_opened() {
    let mut f = Fixture::new();
    f.world.inventory.root = Uuid::from_u128(1);
    f.world.inventory.created_folders(vec![aurora_net::inventory::InvFolder {
        id: f.world.inventory.root,
        name: "Inventory".into(),
        type_default: 8,
        ..Default::default()
    }]);
    f.world.balance = Some(1000);
    f.imports.demo_started = true;
    f.imports.open = false;
    f.imports.launcher_open = true;
    assert!(click_launcher(&mut f, Choice::Texture).is_empty());
    f.settle();
    let (rect, clip, enabled) = f.ctx.data(|data| {
        data.get_temp::<(egui::Rect, egui::Rect, bool)>(egui::Id::unique("import_upload_action"))
            .expect("type upload action")
    });
    assert!(
        enabled && clip.contains_rect(rect),
        "an unquoted mesh cannot block the texture window"
    );
    let click = |f: &mut Fixture, rect: egui::Rect| {
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(
                f.frame(vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ])
                .1,
            );
        }
        actions
    };
    assert!(click(&mut f, rect).is_empty());
    assert!(f.imports.confirm);
    assert_eq!(f.imports.confirm_choice, Some(Choice::Texture));
    assert_eq!(f.imports.choice, Choice::Bulk);
    f.imports.open_type_window(&f.ctx, Choice::Mesh);
    f.settle();
    let rect = f.ctx.data(|data| {
        data.get_temp::<egui::Rect>(egui::Id::unique("import_confirm_upload"))
            .expect("confirmation stays above the type windows")
    });
    let actions = click(&mut f, rect);
    assert!(matches!(actions.as_slice(), [Action::StartFor(Choice::Texture)]));
    assert!(!f.imports.confirm && f.imports.confirm_choice.is_none());
    assert!(f.imports.window_open(Choice::Mesh) && f.imports.window_open(Choice::Texture));
}

#[test]
fn confirmation_mentions_shared_textures_only_when_the_selected_type_actually_reuses_them() {
    let mut f = Fixture::new();
    let map = LocalImage {
        path: None,
        key: Uuid::new_v4(),
        name: "Shared".into(),
        pixels: crate::imports::tests::image(4, 4),
        asset: None,
        empty_alpha: false,
    };
    let material = Source::Material {
        value: PbrMaterial::default(),
        images: Box::new([Some(map.clone()), Some(map), None, None]),
    };
    f.imports.entries = vec![
        Entry::new(
            "tone.wav".into(),
            Ok(Source::Sound(Arc::new(Sound {
                source_channels: 1,
                source_bits: 16,
                mono: vec![0.0; 44100],
            }))),
            Default::default(),
        ),
        Entry::new("shared.gltf".into(), Ok(material), Default::default()),
    ];
    f.imports.demo_started = true;
    f.imports.open = false;
    f.imports.confirm = true;
    for (choice, selected, shared) in [
        (Choice::Sound, true, false),
        (Choice::Material, true, true),
        (Choice::Material, false, false),
    ] {
        f.imports.confirm_choice = Some(choice);
        f.imports.entries[1].selected = selected;
        let output = f.settle();
        assert_eq!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("import-confirm-shared-textures"))), shared);
    }
}

#[test]
fn unified_import_add_files_requests_the_picker_only_after_an_explicit_click() {
    let mut f = Fixture::new();
    let output = f.settle();
    let pos = text_position(&output, aurora_i18n::tr!("import-choose-files")) + egui::vec2(5.0, 5.0);
    for pressed in [true, false] {
        let (_, actions) = f.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert_eq!(
            actions.iter().filter(|action| matches!(action, Action::Files)).count(),
            usize::from(!pressed)
        );
    }
    assert!(!f.frame(Vec::new()).1.iter().any(|action| matches!(action, Action::Files)));
}

#[test]
fn unified_import_keeps_its_size_when_the_queue_empties_and_a_receipt_appears() {
    let mut f = Fixture::new();
    f.settle();
    let size = f.ctx.memory(|memory| memory.area_rect(egui::Id::unique(WINDOW))).unwrap().size();
    f.imports.entries.clear();
    f.imports.spent = 51;
    f.prefs.show_payment = true;
    for _ in 0..120 {
        f.frame(Vec::new());
        assert_eq!(
            f.ctx.memory(|memory| memory.area_rect(egui::Id::unique(WINDOW))).unwrap().size(),
            size,
            "retiring the last upload and displaying its receipt retain the window geometry"
        );
    }
}

#[test]
fn clearing_the_queue_removes_hidden_and_unchecked_drafts_without_removing_local_previews() {
    fn click_clear(f: &mut Fixture) -> Vec<Uuid> {
        f.settle();
        let pos = f
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("import_queue_clear")))
            .expect("queue trash icon")
            .center();
        let mut removed = Vec::new();
        for pressed in [true, false] {
            let (_, actions) = f.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            removed.extend(actions.into_iter().filter_map(|action| match action {
                Action::Remove(id) => Some(id),
                _ => None,
            }));
        }
        removed
    }

    let mut f = Fixture::new();
    let ids: Vec<_> = f.imports.entries.iter().map(|entry| entry.id).collect();
    f.imports.entries[0].selected = false;
    f.imports.type_filters[0] = false;
    let mut local = Entry::new(
        "local.png".into(),
        Ok(Source::Texture(crate::imports::tests::image(4, 4))),
        Default::default(),
    );
    local.local = true;
    f.imports.entries.push(local);
    assert_eq!(
        click_clear(&mut f),
        ids,
        "clear the whole upload queue, including filtered and unchecked entries"
    );

    f.imports.pending_files = Some(Uuid::new_v4());
    assert!(click_clear(&mut f).is_empty(), "wait for the file loader before clearing the queue");
    f.imports.pending_files = None;
    f.imports.entries.retain(|entry| entry.local);
    assert!(click_clear(&mut f).is_empty(), "an empty upload queue cannot remove local previews");
}

#[test]
fn upload_tabs_embed_folders_and_permissions_and_preserve_the_active_mesh() {
    let mut f = Fixture::new();
    f.settle();
    let mesh = f.imports.entries[0].id;
    f.ctx
        .data_mut(|data| data.insert_temp(egui::Id::unique(("mesh_section", mesh)), 1usize));
    let size = f.ctx.memory(|memory| memory.area_rect(egui::Id::unique(WINDOW))).unwrap().size();
    for (label, tab, body) in [
        (
            aurora_i18n::tr!("import-tab-permissions"),
            Tab::Permissions,
            aurora_i18n::tr!("import-show-payment"),
        ),
        (
            aurora_i18n::tr!("import-tab-folder"),
            Tab::Folder,
            aurora_i18n::tr!("import-folders-preview"),
        ),
        (
            aurora_i18n::tr!("import-tab-batch"),
            Tab::Batch,
            aurora_i18n::tr!("import-mesh-section-physics"),
        ),
    ] {
        let output = f.settle();
        let pos = text_position(&output, label) + egui::vec2(5.0, 5.0);
        for pressed in [true, false] {
            f.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        let output = f.settle();
        assert_eq!(f.imports.tab, tab);
        text_position(&output, body);
        let upload = f
            .ctx
            .data(|data| data.get_temp::<(egui::Rect, egui::Rect, bool)>(egui::Id::unique("import_upload_action")));
        if tab == Tab::Permissions {
            assert!(upload.is_none(), "permissions has no import actions");
        } else {
            let (button, clip, _) = upload.expect("tab import footer");
            let window = f.ctx.memory(|memory| memory.area_rect(egui::Id::unique(WINDOW))).unwrap();
            assert!(clip.contains_rect(button));
            assert!(button.left() > window.left() + 228.0, "actions belong to the right pane");
            assert!(window.bottom() - button.bottom() < 40.0, "actions stay at the bottom of the tab");
        }
        assert_eq!(f.imports.active, Some(mesh));
        assert_eq!(
            f.ctx.memory(|memory| memory.area_rect(egui::Id::unique(WINDOW))).unwrap().size(),
            size
        );
        assert!(
            f.ctx
                .memory(|memory| memory.area_rect(egui::Id::unique("import_settings")))
                .is_none()
        );
    }
    assert_eq!(
        f.ctx.data(|data| data.get_temp::<usize>(egui::Id::unique(("mesh_section", mesh)))),
        Some(1)
    );
}

#[test]
fn batch_default_folders_choice_routes_every_row_and_survives_tab_navigation() {
    use aurora_net::inventory::InvFolder;
    let mut f = Fixture::new();
    f.world.inventory.root = Uuid::from_u128(1);
    let common = Uuid::from_u128(2);
    f.world.inventory.created_folders(vec![
        InvFolder {
            id: f.world.inventory.root,
            name: "Inventory".into(),
            type_default: 8,
            ..Default::default()
        },
        InvFolder {
            id: common,
            parent: f.world.inventory.root,
            name: "Common".into(),
            type_default: -1,
            ..Default::default()
        },
    ]);
    f.prefs.batch_folder = common;
    f.prefs.folder_layout = Some(folders::Layout::Flat);
    let output = f.settle();
    assert!(f.prefs.use_default_folders);
    assert!(f.imports.entries.iter().all(|entry| entry.folder == f.world.inventory.root));
    let pos = text_position(&output, aurora_i18n::tr!("import-use-default-folders")) + egui::vec2(5.0, 5.0);
    for pressed in [true, false] {
        f.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    f.settle();
    assert!(!f.prefs.use_default_folders);
    assert!(f.imports.entries.iter().all(|entry| entry.folder == common));
    for tab in [Tab::Permissions, Tab::Batch] {
        f.imports.tab = tab;
        f.settle();
        assert!(!f.prefs.use_default_folders);
        assert!(f.imports.entries.iter().all(|entry| entry.folder == common));
    }
}

#[test]
fn shared_routing_header_retains_layout_and_edited_paths_in_both_views() {
    let mut f = Fixture::new();
    f.prefs.use_default_folders = false;
    f.prefs.folder_layout = Some(folders::Layout::Mirror);
    f.imports.entries[0].source_folders = vec!["Pack".into()];
    f.settle();
    let edited = vec!["Edited".into(), "Mesh".into()];
    f.imports.entries[0].upload_path = Some(edited.clone());
    for tab in [Tab::Folder, Tab::Batch, Tab::Folder] {
        f.imports.tab = tab;
        let output = f.settle();
        text_position(&output, aurora_i18n::tr!("import-use-default-folders"));
        text_position(&output, aurora_i18n::tr!("import-folders-mirror"));
        text_position(&output, aurora_i18n::tr!("import-folders-type"));
        text_position(&output, aurora_i18n::tr!("import-folders-flat"));
        assert_eq!(f.imports.entries[0].upload_path.as_ref(), Some(&edited));
    }
    f.prefs.use_default_folders = true;
    f.settle();
    f.prefs.use_default_folders = false;
    f.settle();
    assert_eq!(f.imports.entries[0].upload_path.as_ref(), Some(&edited));
}
