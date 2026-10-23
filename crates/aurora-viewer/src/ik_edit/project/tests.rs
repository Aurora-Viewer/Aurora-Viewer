use super::super::profiles::tests::fixture;
use super::*;

#[test]
fn project_round_trip_restores_all_profiles_frames_order_and_missing_inventory_references() {
    let (world, frame, mut editor) = fixture();
    editor.animations.demo(&world.avatar_lib.rig, "animations-reordered");
    let item = Uuid::new_v4();
    let mut snapshot = editor.animations.save();
    snapshot.rows[0].item = Some(item);
    editor.animations.restore(&snapshot);
    editor.animations.suppress_live = true;
    assert!(editor.add_profile(true, &world, &frame));
    editor.animations.rows[0].frame = 42;
    editor.graph.time = 1.25;
    let bytes = editor.project_document().encode().unwrap();
    let card = Notecard::default().encode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    let mut loaded = Editor::default();
    loaded
        .apply_project(&world, Notecard::decode(&card).unwrap().text.as_bytes())
        .unwrap();
    assert_eq!(loaded.graph.definition, editor.stored_definition());
    assert_eq!(loaded.animations.save(), editor.animations.save());
    assert_eq!(loaded.graph.profile, Some(1));
    assert_eq!(loaded.graph.time, 1.25);
    loaded.switch_profile(0, &world, &frame);
    assert_eq!(loaded.animations.save(), snapshot_with_suppression(snapshot));
    assert_eq!(loaded.animations.save().rows[0].item, Some(item));
}
fn snapshot_with_suppression(mut saved: animations::SavedStack) -> animations::SavedStack {
    saved.suppress_live = true;
    saved
}

#[test]
fn project_rejects_invalid_or_oversized_files_atomically_and_writes_a_replaceable_draft() {
    let (world, _, mut editor) = fixture();
    let before = editor.graph.definition.clone();
    let mut doc = editor.project_document();
    doc.aurora_ik_project = 2;
    assert!(editor.apply_project(&world, &serde_json::to_vec(&doc).unwrap()).is_err());
    assert!(Document::decode(&vec![b' '; MAX_BYTES + 1]).is_err());
    assert_eq!(editor.graph.definition, before);
    let root = std::env::temp_dir().join(format!("aurora-ik-project-{}", Uuid::new_v4()));
    let path = root.join("draft.json");
    let first = editor.project_document().encode().unwrap();
    write(&path, &first).unwrap();
    assert_eq!(read(&path).unwrap(), first);
    editor.graph.definition.profiles[0].name = "Replacement".into();
    let next = editor.project_document().encode().unwrap();
    write(&path, &next).unwrap();
    assert_eq!(read(&path).unwrap(), next);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn closing_flushes_unsaved_changes_but_never_overwrites_an_unrestored_draft() {
    let (world, _, mut editor) = fixture();
    let (jobs, _) = Jobs::new();
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    let ctx = egui::Context::default();
    editor.project.owner = Some(world.agent_id);
    editor.project.checked = true;
    editor.project.library.ready = true;
    editor.project.last = editor.project_document().encode().unwrap();
    editor.project.next = Some(Instant::now() + Duration::from_secs(60));
    editor.project.was_open = true;
    editor.open = false;
    editor.graph.definition.profiles[0].name = "Last edit".into();
    editor.tick_project(&jobs, &ctx, &world);
    let Output::Autosaved(bytes) = editor.project.rx.try_recv().unwrap() else {
        panic!("draft write")
    };
    assert_eq!(Document::decode(&bytes).unwrap().definition.profiles[0].name, "Last edit");
    editor.project.busy = false;
    editor.project.draft = Some(bytes);
    editor.project.was_open = true;
    editor.tick_project(&jobs, &ctx, &world);
    assert!(editor.project.rx.is_empty());
    assert!(editor.project.draft.is_some());
}

#[test]
fn only_the_requested_notecard_can_replace_the_project_and_malformed_data_leaves_it_intact() {
    let (world, _, mut editor) = fixture();
    let before = editor.graph.definition.clone();
    let item = Uuid::new_v4();
    editor.project.loading = Some((item, Instant::now()));
    assert!(!editor.project_preview(&world, Uuid::new_v4(), &Ok(Vec::new())));
    assert!(editor.project.loading.is_some());
    assert!(editor.project_preview(&world, item, &Ok(b"not a project".to_vec())));
    assert!(editor.project.loading.is_none());
    assert_eq!(editor.graph.definition, before);
}

#[test]
fn a_temporarily_missing_avatar_does_not_consume_the_recoverable_local_draft() {
    let (mut world, _, mut editor) = fixture();
    let bytes = editor.project_document().encode().unwrap();
    editor.project.owner = Some(world.agent_id);
    editor.project.checked = true;
    editor.project.library.ready = true;
    editor.project.draft = Some(bytes.clone());
    editor.project.action = Some(Action::Restore);
    world.avatar_poses.clear();
    let (jobs, _) = Jobs::new();
    editor.tick_project(&jobs, &egui::Context::default(), &world);
    assert_eq!(editor.project.draft, Some(bytes));
    assert!(editor.project.autosave_blocked || editor.project.draft.is_some());
}
