use super::*;

#[test]
fn folder_glb_extracts_mesh_and_materials_with_independent_filters() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.0.join("preview.glb"),
        include_bytes!("../../../../../../assets/models/aurora-3d.glb"),
    )
    .unwrap();
    let entries = fixture.load(&Preferences::default());
    assert!(entries.iter().any(|entry| matches!(entry.source, Some(Source::Mesh(_)))));
    assert!(entries.iter().any(|entry| matches!(entry.source, Some(Source::Material { .. }))));
    assert!(entries.iter().all(|entry| entry.selected && entry.source.is_some()));
    assert!(entries.iter().all(|entry| entry.source_folders == entries[0].source_folders));
    let mut imports = Imports {
        entries,
        ..Default::default()
    };
    imports.type_filters[4] = false;
    let visible = imports.visible_indices();
    assert!(!visible.is_empty());
    assert!(
        visible
            .iter()
            .all(|&i| matches!(imports.entries[i].source, Some(Source::Material { .. })))
    );
    imports.type_filters[4] = true;
    imports.type_filters[3] = false;
    assert!(
        imports
            .visible_indices()
            .iter()
            .all(|&i| matches!(imports.entries[i].source, Some(Source::Mesh(_))))
    );
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("aurora-folder-lods-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn mesh(&self, name: &str) {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, crate::imports::demo::DAE).unwrap();
    }
    fn load(&self, prefs: &Preferences) -> Vec<Entry> {
        load(&self.0, prefs, prefs.texture(), 30, 0, 0)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        assert_eq!(root.parent(), Some(std::env::temp_dir().canonicalize().unwrap().as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn folder_mesh_families_select_file_lods_and_hide_only_the_successful_companions() {
    for suffixes in [
        ["LOD0", "LOD1", "LOD2", "", "PHYS"],
        ["LOD3", "LOD2", "LOD1", "LOD0", "PHYS"],
        ["LOWEST", "LOW", "MED", "HIGH", "PHYS"],
        ["a", "b", "c", "z", "physics"],
    ] {
        let fixture = Fixture::new();
        let prefs = Preferences {
            mesh_suffixes: suffixes.map(String::from),
            ..Default::default()
        };
        let high = if suffixes[3].is_empty() {
            "Chair.dae".into()
        } else {
            format!("Chair_{}.dae", suffixes[3])
        };
        fixture.mesh(&high);
        for level in [0, 1, 2, 4] {
            fixture.mesh(&format!("Chair_{}.dae", suffixes[level]));
        }
        let entries = fixture.load(&prefs);
        assert_eq!(entries.len(), 1, "one inventory draft for {suffixes:?}");
        assert!(entries[0].path.ends_with(&high));
        assert_eq!(
            entries[0].source_folders,
            vec![fixture.0.file_name().unwrap().to_string_lossy().to_string()]
        );
        let Some(Source::Mesh(draft)) = &entries[0].source else {
            panic!("mesh")
        };
        for lod in &draft.lods[..3] {
            assert!(lod.source == mesh::LodSource::File && lod.file.is_some() && lod.path.is_some());
        }
        assert!(draft.physics_file.is_some() && draft.options.physics_source == 5);
        mesh::prepare(draft).expect("associated file LODs prepare together");
    }
}

#[test]
fn folder_association_keeps_orphans_failures_and_equal_names_in_other_directories_visible() {
    let fixture = Fixture::new();
    fixture.mesh("A/Chair.dae");
    fixture.mesh("A/Chair_LOD2.dae");
    fixture.mesh("B/Chair_LOD2.dae");
    fixture.mesh("Orphan_LOD1.dae");
    std::fs::write(fixture.0.join("A/Chair_LOD0.dae"), b"invalid mesh").unwrap();
    std::fs::write(fixture.0.join("Broken.dae"), b"invalid mesh").unwrap();
    fixture.mesh("Broken_LOD1.dae");
    let entries = fixture.load(&Preferences::default());
    assert_eq!(entries.len(), 6);
    assert!(!entries.iter().any(|entry| entry.path.ends_with("A/Chair_LOD2.dae")));
    assert!(entries.iter().any(|entry| entry.path.ends_with("B/Chair_LOD2.dae")));
    assert!(entries.iter().any(|entry| entry.path.ends_with("Orphan_LOD1.dae")));
    assert!(entries.iter().any(|entry| entry.path.ends_with("Broken_LOD1.dae")));
    assert_eq!(entries.iter().filter(|entry| matches!(entry.status, Status::Error(_))).count(), 2);
}
