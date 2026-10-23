use super::super::tests::{cleanup, root, run};
use super::*;
use crate::ik_edit::profiles::tests::fixture;

#[test]
fn row_management_preserves_the_open_project_and_uses_the_named_project_data() {
    let (world, _, mut editor) = fixture();
    editor.animations.demo(&world.avatar_lib.rig, "animations");
    let root = root();
    let moto = run(&root, editor.project_document(), Command::Rename("Moto".into()));
    let mut glasses = run(&root, moto.clone(), Command::Duplicate("Glasses".into()));
    glasses.animations[0].rows[0].frame = 42;
    glasses = run(&root, glasses, Command::Save);
    let glasses_id = glasses.project.as_ref().unwrap().id;
    editor.apply_project(&world, &moto.encode().unwrap()).unwrap();
    editor.graph.definition.bindings[0].weight = 0.3;
    let working = editor.project_document().encode().unwrap();
    let stored = load(&root, moto.project.as_ref().unwrap().id).unwrap();
    for action in [
        Management::Rename("Eyewear".into()),
        Management::Duplicate("Eyewear copy".into()),
        Management::Notecard,
    ] {
        let output = execute(
            &root,
            editor.project_document(),
            glasses_id,
            action,
            "Draft".into(),
            "Default".into(),
        )
        .unwrap();
        editor.library_output(&world, output);
        assert_eq!(editor.project_document().encode().unwrap(), working);
        assert_eq!(load(&root, moto.project.as_ref().unwrap().id).unwrap(), stored);
    }
    let destination = root.join("export.aurora-ik.json");
    for id in [glasses_id, moto.project.as_ref().unwrap().id] {
        let source = document(&root, editor.project_document(), id).unwrap();
        let expected = source.encode().unwrap();
        assert!(matches!(
            export_file(source.clone(), || Some(destination.clone())).unwrap(),
            Output::Saved
        ));
        assert_eq!(read(&destination).unwrap(), expected);
        assert!(matches!(export_file(source, || None).unwrap(), Output::Cancelled));
        assert_eq!(read(&destination).unwrap(), expected, "cancelling keeps the existing export");
        assert_eq!(editor.project_document().encode().unwrap(), working);
        assert_eq!(load(&root, moto.project.as_ref().unwrap().id).unwrap(), stored);
    }
    let copy = list(&root).unwrap().into_iter().find(|p| p.name == "Eyewear copy").unwrap();
    let doc = Document::decode(&load(&root, copy.id).unwrap()).unwrap();
    assert_eq!(doc.animations, glasses.animations);
    assert_eq!(doc.definition, glasses.definition);
    let commands = editor.take_project_commands(world.agent_id);
    let data = commands
        .iter()
        .find_map(|c| match c {
            NetCommand::CreateNotecardItem { name, data, .. } if name.ends_with("Eyewear") => Some(data),
            _ => None,
        })
        .unwrap();
    let exported = Document::decode(Notecard::decode(data).unwrap().text.as_bytes()).unwrap();
    assert_eq!(exported.project.unwrap().id, glasses_id);
    assert_eq!(exported.animations, glasses.animations);
    let output = execute(
        &root,
        editor.project_document(),
        glasses_id,
        Management::Delete,
        "Draft".into(),
        "Default".into(),
    )
    .unwrap();
    editor.library_output(&world, output);
    assert!(!path(&root, glasses_id).exists());
    assert_eq!(editor.project_document().encode().unwrap(), working);
    assert_eq!(load(&root, moto.project.as_ref().unwrap().id).unwrap(), stored);
    for action in [
        Management::Rename("Missing".into()),
        Management::Duplicate("Missing copy".into()),
        Management::File,
        Management::Notecard,
        Management::Delete,
    ] {
        assert!(
            execute(
                &root,
                editor.project_document(),
                glasses_id,
                action,
                "Draft".into(),
                "Default".into()
            )
            .is_err()
        );
    }
    cleanup(&root);
}
