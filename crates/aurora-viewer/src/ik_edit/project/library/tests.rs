use super::super::super::profiles::tests::fixture;
use super::*;
use crate::ik_edit::tests::HeadlessContext as _;

pub(super) fn root() -> PathBuf {
    std::env::temp_dir().join(format!("aurora-ik-library-{}", Uuid::new_v4()))
}
pub(super) fn cleanup(root: &Path) {
    assert!(root.starts_with(std::env::temp_dir()));
    assert!(root.file_name().unwrap().to_string_lossy().starts_with("aurora-ik-library-"));
    if root.exists() {
        std::fs::remove_dir_all(root).unwrap();
    }
}
fn output_bytes(output: Output) -> Vec<u8> {
    match output {
        Output::LibraryApplied(bytes) | Output::LibrarySaved(bytes) => bytes,
        _ => panic!("library document"),
    }
}
pub(super) fn run(root: &Path, current: Document, command: Command) -> Document {
    Document::decode(&output_bytes(
        execute(root, current, command, "Draft".into(), "Default profile".into()).unwrap(),
    ))
    .unwrap()
}

#[test]
fn separate_projects_keep_complete_profile_sets_and_switching_preserves_unsaved_work() {
    let (world, frame, mut editor) = fixture();
    editor.animations.demo(&world.avatar_lib.rig, "animations");
    assert!(editor.add_profile(true, &world, &frame));
    let root = root();
    let moto = run(&root, editor.project_document(), Command::Rename("Moto".into()));
    let moto_id = moto.project.as_ref().unwrap().id;
    let mut working = moto.clone();
    working.animations[1].rows[0].frame = 43;
    working.definition.bindings[0].weight = 0.7;
    let glasses = run(&root, working.clone(), Command::New("Glasses".into()));
    assert!(glasses.definition.profiles.is_empty());
    assert!(glasses.definition.bindings.is_empty() && glasses.definition.nodes.is_empty());
    assert!(glasses.animations.is_empty());
    let loaded = run(&root, glasses, Command::Load(moto_id));
    assert_eq!(loaded.definition, working.definition);
    assert_eq!(loaded.animations, working.animations);
    let names: Vec<_> = list(&root).unwrap().iter().map(|i| i.name.clone()).collect();
    assert!(names.contains(&"Moto".to_owned()) && names.contains(&"Glasses".to_owned()));
    cleanup(&root);
}

#[test]
fn duplication_is_independent_and_rename_keeps_the_same_project_identity() {
    let (_, _, editor) = fixture();
    let root = root();
    let moto = run(&root, editor.project_document(), Command::Rename("Moto".into()));
    let id = moto.project.as_ref().unwrap().id;
    let mut copy = run(&root, moto.clone(), Command::Duplicate("Moto copy".into()));
    assert_ne!(copy.project.as_ref().unwrap().id, id);
    copy.definition.bindings[0].weight = 0.25;
    let copy = run(&root, copy, Command::Save);
    let copy_id = copy.project.as_ref().unwrap().id;
    let renamed = run(&root, copy, Command::Rename("Motorcycle / Variant".into()));
    assert_eq!(renamed.project.as_ref().unwrap().id, copy_id);
    assert_eq!(
        Document::decode(&read(&path(&root, id)).unwrap()).unwrap().definition,
        moto.definition
    );
    assert_eq!(list(&root).unwrap().len(), 2, "renaming updates the existing project");
    assert!(!root.join("Motorcycle").exists(), "titles never become directories");
    cleanup(&root);
}

#[test]
fn broken_destinations_and_duplicate_names_leave_the_current_project_intact() {
    let (_, _, editor) = fixture();
    let root = root();
    let moto = run(&root, editor.project_document(), Command::Rename("Moto".into()));
    let bytes = moto.encode().unwrap();
    let id = moto.project.as_ref().unwrap().id;
    assert!(execute(&root, moto.clone(), Command::Load(Uuid::new_v4()), "Draft".into(), "Default".into()).is_err());
    assert!(
        execute(
            &root,
            moto.clone(),
            Command::Duplicate("Moto".into()),
            "Draft".into(),
            "Default".into()
        )
        .is_err()
    );
    assert_eq!(read(&path(&root, id)).unwrap(), bytes);
    assert!(execute(&root, moto, Command::Rename(" ".into()), "Draft".into(), "Default".into()).is_err());
    cleanup(&root);
}

#[test]
fn opening_a_project_clears_other_project_controls_but_restores_its_saved_layers() {
    let (world, frame, mut editor) = fixture();
    let root = root();
    editor.animations.demo(&world.avatar_lib.rig, "animations");
    let moto = run(&root, editor.project_document(), Command::Rename("Moto".into()));
    let saved = moto.animations.clone();
    let glasses = run(&root, moto.clone(), Command::New("Glasses".into()));
    editor.apply_project(&world, &glasses.encode().unwrap()).unwrap();
    assert!(editor.request().goals.is_empty());
    assert!(editor.animations.rows.is_empty());
    editor.apply_project(&world, &moto.encode().unwrap()).unwrap();
    assert_eq!(editor.animations.save(), saved[0]);
    editor.switch_profile(0, &world, &frame);
    assert_eq!(editor.project.library.active.as_ref().unwrap().name, "Moto");
    cleanup(&root);
}

#[test]
fn a_catalogue_file_with_another_identity_is_rejected_before_saving_current_work() {
    let (_, _, editor) = fixture();
    let root = root();
    let moto = run(&root, editor.project_document(), Command::Rename("Moto".into()));
    let id = moto.project.as_ref().unwrap().id;
    let stored = read(&path(&root, id)).unwrap();
    let wrong = Uuid::new_v4();
    write(&path(&root, wrong), &stored).unwrap();
    let mut unsaved = moto;
    unsaved.definition.bindings[0].weight = 0.3;
    assert!(execute(&root, unsaved, Command::Load(wrong), "Draft".into(), "Default".into()).is_err());
    assert_eq!(read(&path(&root, id)).unwrap(), stored);
    assert_eq!(list(&root).unwrap().len(), 1);
    cleanup(&root);
}

#[test]
fn importing_an_unnamed_file_preserves_the_current_project_and_joins_the_library() {
    let (_, _, editor) = fixture();
    let root = root();
    let original = editor.project_document();
    let mut moto = run(&root, original.clone(), Command::Rename("Moto".into()));
    let id = moto.project.as_ref().unwrap().id;
    moto.definition.bindings[0].weight = 0.6;
    let imported = run(&root, moto.clone(), Command::Import(original.encode().unwrap()));
    assert_ne!(imported.project.as_ref().unwrap().id, id);
    assert_eq!(imported.definition, original.definition);
    assert_eq!(Document::decode(&load(&root, id).unwrap()).unwrap().definition, moto.definition);
    assert_eq!(list(&root).unwrap().len(), 2);
    let again = run(&root, imported.clone(), Command::Import(original.encode().unwrap()));
    assert_ne!(again.project.as_ref().unwrap().name, imported.project.as_ref().unwrap().name);
    assert_eq!(list(&root).unwrap().len(), 3);
    cleanup(&root);
}

#[test]
fn first_open_and_project_creation_require_explicit_names_without_implicit_profiles() {
    let (world, frame, _) = fixture();
    let mut editor = Editor {
        open: true,
        ..Default::default()
    };
    editor.project.library.ready = true;
    editor.ensure_profiles(&world, &frame);
    assert!(editor.graph.definition.profiles.is_empty());
    assert!(editor.project_document().encode().is_ok());
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let mut time = 0.0;
    let mut draw = |editor: &mut Editor, events| {
        time += 1.0 / 60.0;
        ctx.run_headless(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 1000.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                assert!(editor.library_start_ui(ui.ctx(), &palette, &world));
            },
        )
    };
    let centre = |output: &egui::FullOutput, key: &'static str| {
        output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text(key) => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
    };
    let click = |pos, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    let key = |key| {
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    };
    let _ = draw(&mut editor, vec![]);
    let initial = draw(&mut editor, vec![]);
    let create = centre(&initial, "ik-library-new").unwrap();
    assert!(centre(&initial, "ik-library-default").is_none());
    assert!(centre(&initial, "ik-profile-add").is_none());
    let _ = draw(&mut editor, click(create, true));
    let _ = draw(&mut editor, click(create, false));
    let menu = draw(&mut editor, vec![]);
    for key in [
        "ik-project-new-blank",
        "ik-project-new-object",
        "ik-project-new-file",
        "ik-project-new-notecard",
    ] {
        assert!(centre(&menu, key).is_some(), "missing project source {key}");
    }
    let mut menu = menu;
    for key in ["ik-project-new-object", "ik-project-new-file", "ik-project-new-notecard"] {
        let source = centre(&menu, key).unwrap();
        let _ = draw(&mut editor, click(source, true));
        let _ = draw(&mut editor, click(source, false));
        match key {
            "ik-project-new-object" => assert!(editor.object_import_picking()),
            "ik-project-new-file" => assert!(matches!(editor.project.action, Some(Action::Open))),
            _ => assert!(editor.project.picker),
        }
        assert!(editor.project.library.active.is_none());
        editor.begin_project_creation();
        let _ = draw(&mut editor, click(create, true));
        let _ = draw(&mut editor, click(create, false));
        menu = draw(&mut editor, vec![]);
    }
    assert!(editor.project.library.edit.is_none());
    let blank = centre(&menu, "ik-project-new-blank").unwrap();
    let _ = draw(&mut editor, click(blank, true));
    let _ = draw(&mut editor, click(blank, false));
    assert_eq!(
        editor.project.library.edit.as_ref().unwrap().name,
        aurora_i18n::text("ik-library-default")
    );
    assert!(editor.project.library.active.is_none());
    let _ = draw(&mut editor, key(egui::Key::Escape));
    assert!(editor.project.library.edit.is_none() && editor.project.library.command.is_none());
    let root = root();
    let empty = run(&root, editor.project_document(), Command::New("Moto".into()));
    editor.apply_project(&world, &empty.encode().unwrap()).unwrap();
    editor.project.library.ready = true;
    editor.ensure_profiles(&world, &frame);
    assert!(editor.graph.definition.profiles.is_empty());
    let _ = draw(&mut editor, vec![]);
    let output = draw(&mut editor, vec![]);
    let bindings = centre(&output, "ik-edit-tab-bindings").unwrap();
    let _ = draw(&mut editor, click(bindings, true));
    let _ = draw(&mut editor, click(bindings, false));
    let output = draw(&mut editor, vec![]);
    let create = centre(&output, "ik-profile-add").unwrap();
    assert!(centre(&output, "ik-profile-default").is_none());
    let _ = draw(&mut editor, click(create, true));
    let _ = draw(&mut editor, click(create, false));
    assert!(editor.graph.definition.profiles.is_empty());
    let _ = draw(&mut editor, vec![egui::Event::Text("drive".into())]);
    let _ = draw(&mut editor, key(egui::Key::Enter));
    assert_eq!(editor.graph.definition.profiles.len(), 1);
    assert_eq!(editor.graph.definition.profiles[0].name, "drive");
    assert!(editor.project_document().encode().is_ok());
    cleanup(&root);
}

#[test]
fn importing_a_named_project_copy_cannot_overwrite_its_existing_library_identity() {
    let (_, _, mut editor) = fixture();
    let root = root();
    let saved = run(&root, editor.project_document(), Command::Rename("Moto".into()));
    let old_id = saved.project.as_ref().unwrap().id;
    let mut working = saved.clone();
    working.definition.bindings[0].weight = 0.4;
    editor.project.library.import(saved.encode().unwrap()).unwrap();
    let command = editor.project.library.command.take().unwrap();
    let imported = run(&root, working.clone(), command);
    assert_ne!(imported.project.as_ref().unwrap().id, old_id);
    assert_eq!(imported.definition, saved.definition);
    assert_eq!(
        Document::decode(&load(&root, old_id).unwrap()).unwrap().definition,
        working.definition
    );
    assert_eq!(list(&root).unwrap().len(), 2);
    cleanup(&root);
}
