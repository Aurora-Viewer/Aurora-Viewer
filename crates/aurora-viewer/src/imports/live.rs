//! Freeze one live Blender source into the ordinary mesh import workflow.
//! No quote or paid request is sent here (LLFloaterModelPreview's separate
//! calculate/upload steps, indra/newview/llfloatermodelpreview.cpp).
use super::*;
use crate::blender_live::protocol::{PreparedImage, PreparedSource};

fn entry(source: PreparedSource, images: Vec<PreparedImage>, prefs: &Preferences) -> Entry {
    let used: HashSet<_> = source.scene.materials.iter().flat_map(|m| m.images.into_iter().flatten()).collect();
    let images = images
        .into_iter()
        .enumerate()
        .filter(|(index, _)| used.contains(index))
        .map(|(index, image)| {
            (
                index,
                LocalImage {
                    path: None,
                    key: image.id,
                    empty_alpha: import_texture::empty_alpha(&image.pixels),
                    pixels: image.pixels,
                    asset: None,
                    name: format!("{} {}", source.name, index + 1),
                },
            )
        })
        .collect();
    let mut draft = mesh::Draft::from_shared(source.scene, images);
    draft.lod_suffixes = prefs.mesh_suffixes.clone();
    draft.upload_timeout = prefs.mesh_upload_timeout;
    draft.options.textures = draft.scene.materials.iter().any(|m| m.images[0].is_some());
    // Keep the captured weights and bind matrices; joint-position overrides
    // remain off. This is the same rest mesh as the live attachment.
    if prefs.mesh_reliable_default {
        for lod in &mut draft.lods[..3] {
            lod.source = mesh::LodSource::Reliable;
        }
    }
    let mut entry = Entry::new(PathBuf::new(), Ok(Source::Mesh(Box::new(draft))), prefs.texture());
    entry.name = source.name;
    if entry.name.len() > 63 {
        let end = entry.name.floor_char_boundary(63);
        entry.name.truncate(end);
    }
    entry.from_live = true;
    entry
}

impl Imports {
    pub fn open_live_mesh(
        &mut self,
        source: PreparedSource,
        images: Vec<PreparedImage>,
        jobs: &crate::scene::jobs::Jobs,
        ctx: &egui::Context,
    ) {
        self.open_live_mesh_lods(source, images, None, jobs, ctx);
    }

    pub fn open_blender_lods(&mut self, bundle: crate::blender_live::import::Bundle, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        self.open_live_mesh_lods(bundle.high, bundle.images, Some(bundle.lods), jobs, ctx);
    }

    fn open_live_mesh_lods(
        &mut self,
        source: PreparedSource,
        images: Vec<PreparedImage>,
        lods: Option<[Arc<aurora_assets::import_mesh::Scene>; 3]>,
        jobs: &crate::scene::jobs::Jobs,
        ctx: &egui::Context,
    ) {
        if self.pending_files.is_some() || self.busy() || self.entries.len() >= 512 {
            return;
        }
        let request = Uuid::new_v4();
        let id = Uuid::new_v4();
        self.pending_files = Some(request);
        self.open(Choice::Mesh);
        self.active = Some(id);
        for entry in &mut self.entries {
            entry.selected = false;
        }
        let prefs = self.load_preferences.clone();
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let mut entry = entry_lods(source, images, &prefs, lods);
            entry.id = id;
            let _ = tx.send(Worker::Loaded(request, vec![entry]));
            wake.request_repaint();
        });
    }
}

fn entry_lods(
    source: PreparedSource,
    images: Vec<PreparedImage>,
    prefs: &Preferences,
    lods: Option<[Arc<aurora_assets::import_mesh::Scene>; 3]>,
) -> Entry {
    let mut entry = entry(source, images, prefs);
    if let Some(lods) = lods
        && let Some(Source::Mesh(draft)) = &mut entry.source
    {
        for (level, scene) in lods.into_iter().enumerate() {
            draft.lods[level].triangles = scene.models.iter().map(|model| model.triangles()).sum();
            draft.lods[level].source = mesh::LodSource::File;
            draft.lods[level].file = Some(scene);
        }
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use glam::Mat4;

    #[test]
    fn blender_lods_are_assigned_to_file_levels_and_remain_unquoted() {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let prepared = crate::blender_live::demo_fixture(&lib.rig, false).expect("live input");
        let high = prepared.sources[0].clone();
        let lods = std::array::from_fn(|_| high.scene.clone());
        let entry = entry_lods(high, prepared.images, &Preferences::default(), Some(lods));
        let Some(Source::Mesh(draft)) = &entry.source else {
            panic!("mesh draft")
        };
        assert!(
            draft.lods[..3]
                .iter()
                .all(|lod| lod.source == mesh::LodSource::File && lod.file.is_some() && lod.path.is_none())
        );
        let prepared = mesh::prepare(draft).expect("prepared levels");
        assert!(prepared.lods.iter().all(|lods| lods.iter().all(Option::is_some)));
        assert!(draft.quote.is_none() && draft.pending.is_none());
    }

    #[test]
    fn blender_live_upload_freezes_all_parts_skin_materials_and_pixels_into_an_unquoted_draft() {
        let vertices: Vec<_> = [[0.0, 0.0, 0.0], [0.4, 0.0, 0.0], [0.0, 0.4, 0.4]]
            .into_iter()
            .map(|position| serde_json::json!({"position":position,"normal":[0,0,1],"uv":[0,0],"joints":[0,0,0,0],"weights":[1,0,0,0]}))
            .collect();
        let face = serde_json::json!({"color":[0.3,0.5,0.8,1],"vertices":vertices,"indices":[0,1,2],
            "material":{"metallic":0.7,"roughness":0.2,"emissive":[0,0,0],"double_sided":false,
                "alpha_mode":"OPAQUE","textures":[0,null,null,null]}});
        let mut wire = serde_json::json!({"protocol":1,"revision":1,"images":[{"width":2,"height":2,"rgba":STANDARD.encode([255;16])}],
            "meshes":[{"name":"Frozen shirt","joints":["mPelvis"],"inverse_bind":[Mat4::IDENTITY.to_cols_array()],"faces":vec![face;9]}]});
        let decode = |wire: &serde_json::Value| {
            crate::blender_live::protocol::prepare(&serde_json::to_vec(wire).expect("JSON"), &HashSet::from(["mPelvis".into()]))
                .expect("valid capture")
        };
        let first = decode(&wire);
        let frozen_scene = first.sources[0].scene.clone();
        let entry = entry(first.sources[0].clone(), first.images, &Preferences::default());
        let Some(Source::Mesh(draft)) = &entry.source else {
            panic!("mesh draft");
        };
        assert!(entry.from_live && entry.path.as_os_str().is_empty());
        assert!(entry.selected && matches!(entry.status, Status::Ready));
        assert!(draft.quote.is_none() && draft.pending.is_none());
        assert!(draft.options.skin && draft.options.textures && !draft.options.joints);
        assert!(Arc::ptr_eq(&draft.scene, &frozen_scene));
        assert_eq!(draft.scene.materials[0].value.metallic_factor, 0.7);
        assert_eq!(draft.images[&0].pixels.data, [255; 16]);
        wire["revision"] = 2.into();
        wire["images"][0]["rgba"] = STANDARD.encode([128; 16]).into();
        wire["meshes"][0]["faces"][0]["vertices"][0]["position"] = serde_json::json!([1, 0, 0]);
        let latest = decode(&wire);
        assert_ne!(latest.sources[0].scene.models[0].center, draft.scene.models[0].center);
        assert_eq!(
            draft.images[&0].pixels.data, [255; 16],
            "live painting cannot alter a pending draft"
        );
        let prepared = mesh::prepare(draft).expect("prepare all parts");
        let request = mesh::request(
            draft,
            &prepared,
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            &entry.name,
            "",
            Permissions::default(),
            entry.options,
        )
        .expect("ordinary upload resources");
        let query = aurora_llsd::from_xml(&request.query).expect("fee query");
        let resources = aurora_llsd::from_xml(&request.resources).expect("resources");
        assert_eq!(resources["mesh_list"].as_array().len(), 2);
        assert_eq!(resources["instance_list"].as_array().len(), 2);
        assert_eq!(
            resources["texture_list"].as_array().len(),
            1,
            "diffuse texture deduplicated across all faces/parts"
        );
        assert_eq!(query["asset_resources"]["instance_list"], resources["instance_list"]);
        for model in &draft.scene.models {
            assert_eq!(model.skin.as_ref().expect("skin").names, ["mPelvis"]);
        }
    }
}
