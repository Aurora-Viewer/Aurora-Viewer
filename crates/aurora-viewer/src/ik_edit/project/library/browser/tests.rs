use super::super::tests::{cleanup, root, run};
use super::*;
use crate::ik_edit::profiles::tests::fixture;
use crate::ik_edit::tests::HeadlessContext as _;

#[test]
fn deleting_a_project_removes_only_its_uuid_file_and_cannot_be_recreated_by_autosave() {
    let (mut world, _, mut editor) = fixture();
    editor.animations.demo(&world.avatar_lib.rig, "animations");
    let root = root();
    let moto = run(&root, editor.project_document(), Command::Rename("Moto".into()));
    let saved = moto.encode().unwrap();
    let moto_id = moto.project.as_ref().unwrap().id;
    let copy = run(&root, moto.clone(), Command::Duplicate("Copy".into()));
    let id = copy.project.as_ref().unwrap().id;
    editor.apply_project(&world, &copy.encode().unwrap()).unwrap();
    editor.project.library.entries = list(&root).unwrap();
    let mut working = copy.clone();
    working.definition.bindings[0].weight = 0.3;
    assert!(execute(&root, working.clone(), Command::Delete(moto_id), "Draft".into(), "Default".into()).is_err());
    assert_eq!(read(&path(&root, moto_id)).unwrap(), saved);
    let output = execute(&root, working, Command::Delete(id), "Draft".into(), "Default".into()).unwrap();
    assert!(!path(&root, id).exists());
    let poses = std::mem::take(&mut world.avatar_poses);
    editor.library_output(&world, output);
    assert!(
        editor.project.library.active.is_none(),
        "clearing an empty project needs no rendered avatar"
    );
    assert!(editor.request().goals.is_empty());
    assert!(editor.animations.rows.is_empty());
    assert_eq!(editor.tab, 5);
    assert_eq!(editor.project.library.entries, list(&root).unwrap());
    world.avatar_poses = poses;
    editor.project.owner = Some(world.agent_id);
    editor.project.checked = true;
    editor.project.library.ready = true;
    editor.project.flush = true;
    let (jobs, _) = Jobs::new();
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    editor.tick_project(&jobs, &egui::Context::default(), &world);
    assert!(
        editor.project.rx.is_empty(),
        "no implicit draft or project is written after deletion"
    );
    assert!(!path(&root, id).exists());
    assert_eq!(list(&root).unwrap(), vec![moto.project.unwrap()]);
    assert_eq!(load(&root, moto_id).unwrap(), saved);
    cleanup(&root);
}

#[test]
fn projects_tab_selects_and_manages_complete_projects_through_real_pointer_input() {
    let (world, frame, mut editor) = fixture();
    editor.animations.demo(&world.avatar_lib.rig, "animations");
    assert!(editor.add_profile(true, &world, &frame));
    let root = root();
    let moto = run(&root, editor.project_document(), Command::Rename("Moto".into()));
    let mut glasses = run(&root, moto.clone(), Command::Duplicate("Glasses".into()));
    glasses.animations[0].rows[0].frame = 42;
    glasses.profile = 0;
    glasses = run(&root, glasses, Command::Save);
    editor.apply_project(&world, &moto.encode().unwrap()).unwrap();
    editor.project.library.entries = list(&root).unwrap();
    editor.project.library.ready = true;
    editor.graph.definition.bindings[0].weight = 0.7;
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let proj = Projector {
        view_proj: Mat4::IDENTITY,
        width: 1600.0,
        height: 900.0,
        ppp: 1.0,
    };
    let mut time = 0.0;
    let mut draw = |editor: &mut Editor, events| {
        time += 1.0 / 60.0;
        ctx.run_headless(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| editor.show(ui.ctx(), &palette, &world, &proj),
        )
    };
    let centre = |output: &egui::FullOutput, text: &str| {
        output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + t.galley.size() * 0.5),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing visible control {text}"))
    };
    let row_action = |output: &egui::FullOutput, name: &str, index: usize| {
        let row = output
            .shapes
            .iter()
            .rev()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == name => Some(t.pos + t.galley.size() * 0.5),
                _ => None,
            })
            .unwrap();
        let mut buttons: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if (r.rect.size() - egui::Vec2::splat(26.0)).length() < 0.1 && (r.rect.center().y - row.y).abs() < 4.0 =>
                {
                    Some(r.rect.center())
                }
                _ => None,
            })
            .collect();
        buttons.sort_by(|a, b| a.x.total_cmp(&b.x));
        assert_eq!(buttons.len(), 5, "each project row owns five separate actions");
        buttons[index]
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
    let mut output = draw(&mut editor, vec![]);
    for tab in [
        "ik-edit-tab-bindings",
        "ik-edit-tab-virtual",
        "ik-edit-tab-animations",
        "ik-edit-tab-visible",
    ] {
        let pos = centre(&output, aurora_i18n::text(tab));
        let _ = draw(&mut editor, click(pos, true));
        let _ = draw(&mut editor, click(pos, false));
        output = draw(&mut editor, vec![]);
        centre(&output, "Moto");
        for key in [
            "ik-project-target",
            "ik-project-file-export",
            "ik-library-notecard",
            "ik-project-title",
        ] {
            assert!(
                !output
                    .shapes
                    .iter()
                    .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text(key))),
                "project management {key} leaked into {tab}"
            );
        }
    }
    let tab = centre(&output, aurora_i18n::text("ik-edit-tab-projects"));
    let _ = draw(&mut editor, click(tab, true));
    let _ = draw(&mut editor, click(tab, false));
    assert_eq!(editor.tab, 5);
    let output = draw(&mut editor, vec![]);
    centre(&output, aurora_i18n::text("ik-project-target"));
    assert!(
        !output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text("ik-project-file-export"))),
        "the standalone export button is removed"
    );
    for name in ["Moto", "Glasses"] {
        row_action(&output, name, 0);
    }
    let row = centre(&output, "Glasses");
    let _ = draw(&mut editor, click(row, true));
    let _ = draw(&mut editor, click(row, false));
    let command = editor.project.library.command.take().unwrap();
    assert!(matches!(command, Command::Load(id) if id == glasses.project.as_ref().unwrap().id));
    let output = execute(&root, editor.project_document(), command, "Draft".into(), "Default".into()).unwrap();
    editor.library_output(&world, output);
    editor.project.library.ready = true;
    assert_eq!(editor.tab, 1, "loading a project by name opens Bindings");
    assert_eq!(editor.project.library.active, glasses.project);
    assert_eq!(editor.animations.rows[0].frame, 42);
    assert_eq!(
        Document::decode(&load(&root, moto.project.as_ref().unwrap().id).unwrap())
            .unwrap()
            .definition
            .bindings[0]
            .weight,
        0.7
    );
    editor.tab = 5;
    let _ = draw(&mut editor, vec![]);
    let output = draw(&mut editor, vec![]);
    let rename = row_action(&output, "Glasses", 0);
    let _ = draw(&mut editor, click(rename, true));
    let _ = draw(&mut editor, click(rename, false));
    let _ = draw(&mut editor, vec![egui::Event::Text("Eyewear".into())]);
    let _ = draw(&mut editor, key(egui::Key::Enter));
    let command = editor.project.library.command.take().unwrap();
    assert!(
        matches!(&command, Command::Manage(id, Management::Rename(name)) if Some(*id) == glasses.project.as_ref().map(|p| p.id) && name == "Eyewear")
    );
    let output = execute(&root, editor.project_document(), command, "Draft".into(), "Default".into()).unwrap();
    editor.library_output(&world, output);
    assert_eq!(
        editor.project.library.active.as_ref().unwrap().id,
        glasses.project.as_ref().unwrap().id
    );
    let output = draw(&mut editor, vec![]);
    let duplicate = row_action(&output, "Eyewear", 1);
    let _ = draw(&mut editor, click(duplicate, true));
    let _ = draw(&mut editor, click(duplicate, false));
    let _ = draw(&mut editor, vec![egui::Event::Text("Eyewear copy".into())]);
    let _ = draw(&mut editor, key(egui::Key::Enter));
    let command = editor.project.library.command.take().unwrap();
    let output = execute(&root, editor.project_document(), command, "Draft".into(), "Default".into()).unwrap();
    editor.library_output(&world, output);
    editor.project.library.ready = true;
    editor.project.library.entries = list(&root).unwrap();
    let copy_id = editor.project.library.entries.iter().find(|p| p.name == "Eyewear copy").unwrap().id;
    assert_eq!(
        editor.project.library.active.as_ref().unwrap().id,
        glasses.project.as_ref().unwrap().id
    );
    assert_ne!(Some(copy_id), glasses.project.as_ref().map(|p| p.id));
    assert_eq!(editor.project_document().animations, glasses.animations);
    assert_eq!(editor.stored_definition(), glasses.definition);
    let output = draw(&mut editor, vec![]);
    let file = row_action(&output, "Eyewear copy", 2);
    let _ = draw(&mut editor, click(file, true));
    let _ = draw(&mut editor, click(file, false));
    assert!(matches!(editor.project.library.command.take(), Some(Command::Manage(id, Management::File)) if id == copy_id));
    assert_eq!(editor.tab, 5);
    let output = draw(&mut editor, vec![]);
    let notecard = row_action(&output, "Eyewear copy", 3);
    let _ = draw(&mut editor, click(notecard, true));
    let _ = draw(&mut editor, click(notecard, false));
    let command = editor.project.library.command.take().unwrap();
    assert!(matches!(command, Command::Manage(id, Management::Notecard) if id == copy_id));
    let output = execute(&root, editor.project_document(), command, "Draft".into(), "Default".into()).unwrap();
    editor.library_output(&world, output);
    editor.project.owner = Some(world.agent_id);
    editor.project.checked = true;
    let commands = editor.take_project_commands(world.agent_id);
    let card = commands
        .iter()
        .find_map(|command| match command {
            NetCommand::CreateNotecardItem { name, data, .. } => Some((name, data)),
            _ => None,
        })
        .unwrap();
    assert!(card.0.ends_with("Eyewear copy"));
    let exported = Document::decode(Notecard::decode(card.1).unwrap().text.as_bytes()).unwrap();
    assert_eq!(exported.project.as_ref().unwrap().id, copy_id);
    assert_eq!(exported.definition, glasses.definition);
    assert_eq!(exported.animations, glasses.animations);
    let output = draw(&mut editor, vec![]);
    let delete = row_action(&output, "Eyewear copy", 4);
    let _ = draw(&mut editor, click(delete, true));
    let _ = draw(&mut editor, click(delete, false));
    assert!(
        editor.project.library.command.is_none(),
        "delete waits for its explicit confirmation"
    );
    let output = draw(&mut editor, vec![]);
    let cancel = centre(&output, aurora_i18n::text("common-cancel"));
    let _ = draw(&mut editor, click(cancel, true));
    let _ = draw(&mut editor, click(cancel, false));
    assert!(editor.project.library.delete.is_none());
    assert!(path(&root, copy_id).exists());
    let output = draw(&mut editor, vec![]);
    let delete = row_action(&output, "Eyewear copy", 4);
    let _ = draw(&mut editor, click(delete, true));
    let _ = draw(&mut editor, click(delete, false));
    let output = draw(&mut editor, vec![]);
    let confirm = centre(&output, aurora_i18n::text("ik-library-delete-confirm-button"));
    let _ = draw(&mut editor, click(confirm, true));
    let _ = draw(&mut editor, click(confirm, false));
    let command = editor.project.library.command.take().unwrap();
    assert!(matches!(command, Command::Manage(id, Management::Delete) if id == copy_id));
    let output = execute(&root, editor.project_document(), command, "Draft".into(), "Default".into()).unwrap();
    editor.library_output(&world, output);
    assert_eq!(editor.project.library.active.as_ref().unwrap().name, "Eyewear");
    assert_eq!(editor.tab, 5);
    assert!(!path(&root, copy_id).exists());
    assert_eq!(list(&root).unwrap().len(), 2);
    let output = draw(&mut editor, vec![]);
    let name = output
        .shapes
        .iter()
        .rev()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Eyewear" => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .unwrap();
    let _ = draw(&mut editor, click(name, true));
    let _ = draw(&mut editor, click(name, false));
    assert_eq!(editor.tab, 1, "clicking the current project also opens Bindings");
    assert!(editor.project.library.command.is_none());
    cleanup(&root);
}
