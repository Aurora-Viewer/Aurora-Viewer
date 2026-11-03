//! Offline files exercise the ordinary loaders, reload/watch paths, previews
//! and upload queue. Each launch owns a directory below the ignored .scratch.

use super::*;
use std::path::Path;

const BVH: &[u8] = b"HIERARCHY ROOT hip { OFFSET 0 0 0 CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation End Site { OFFSET 0 1 0 } } MOTION Frames: 6 Frame Time: 0.5 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 15 0 0 0 0 0 -15 0 0 0 0 0 15 0 0 0 0 0 0 0 0";
pub(super) const DAE: &str = r##"<COLLADA><asset><unit meter="1"/><up_axis>Z_UP</up_axis></asset>
<library_geometries><geometry id="cube" name="Cube"><mesh>
<source id="positions"><float_array count="24">-0.5 -0.5 -0.5 0.5 -0.5 -0.5 0.5 0.5 -0.5 -0.5 0.5 -0.5 -0.5 -0.5 0.5 0.5 -0.5 0.5 0.5 0.5 0.5 -0.5 0.5 0.5</float_array><technique_common><accessor count="8" stride="3"/></technique_common></source>
<vertices id="vertices"><input semantic="POSITION" source="#positions"/></vertices>
<triangles count="12"><input semantic="VERTEX" source="#vertices" offset="0"/><p>0 2 1 0 3 2 4 5 6 4 6 7 0 1 5 0 5 4 3 7 6 3 6 2 0 4 7 0 7 3 1 2 6 1 6 5</p></triangles>
</mesh></geometry></library_geometries><library_visual_scenes><visual_scene id="scene"><node id="node" name="Cube"><instance_geometry url="#cube"/></node></visual_scene></library_visual_scenes><scene><instance_visual_scene url="#scene"/></scene></COLLADA>"##;

pub(super) fn multiple_dae() -> String {
    DAE.replace(
        r##"<node id="node" name="Cube"><instance_geometry url="#cube"/></node>"##,
        r##"<node name="Assembly"><translate>1 2 0</translate>
<node name="Left"><translate>-2 0 0</translate><instance_geometry url="#cube"/></node>
<node name="Middle"><instance_geometry url="#cube"/></node>
<node name="Right"><translate>2 0 0</translate><instance_geometry url="#cube"/></node></node>"##,
    )
}

/// A local Bento tail fixture, intentionally independent of inventory assets.
pub(super) fn tail_dae() -> String {
    let controller = r##"<library_controllers><controller id="tail-skin"><skin source="#cube">
<source id="tail-joints"><Name_array count="1">mTail1</Name_array></source>
<source id="tail-bind"><float_array count="16">1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1</float_array><technique_common><accessor count="1" stride="16"/></technique_common></source>
<source id="tail-weights"><float_array count="1">1</float_array><technique_common><accessor count="1" stride="1"/></technique_common></source>
<joints><input semantic="JOINT" source="#tail-joints"/><input semantic="INV_BIND_MATRIX" source="#tail-bind"/></joints>
<vertex_weights count="8"><input semantic="JOINT" source="#tail-joints" offset="0"/><input semantic="WEIGHT" source="#tail-weights" offset="1"/><vcount>1 1 1 1 1 1 1 1</vcount><v>0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0</v></vertex_weights>
</skin></controller></library_controllers>"##;
    DAE.replace(
        "-0.5 -0.5 -0.5 0.5 -0.5 -0.5 0.5 0.5 -0.5 -0.5 0.5 -0.5 -0.5 -0.5 0.5 0.5 -0.5 0.5 0.5 0.5 0.5 -0.5 0.5 0.5",
        "0 -0.10 -0.10 -0.9 -0.04 -0.04 -0.9 0.04 -0.04 0 0.10 -0.10 0 -0.10 0.10 -0.9 -0.04 0.04 -0.9 0.04 0.04 0 0.10 0.10",
    )
    .replace("<library_visual_scenes>", &format!("{controller}<library_visual_scenes>"))
    .replace(
        r##"<instance_geometry url="#cube"/>"##,
        r##"<instance_controller url="#tail-skin"/>"##,
    )
}

pub(super) fn tail_animation() -> Animation {
    let mut animation = crate::demo::idle_animation();
    animation.base_priority = 4;
    animation.duration = 2.0;
    animation.loop_in = 0.0;
    animation.loop_out = 2.0;
    animation.looping = true;
    animation.ease_in = 0.0;
    animation.ease_out = 0.0;
    animation.joints = vec![aurora_assets::anim::JointMotion {
        joint_name: "mTail1".into(),
        priority: 4,
        rot_keys: [0.0, 0.7, 0.0, -0.7, 0.0]
            .into_iter()
            .enumerate()
            .map(|(i, angle)| aurora_assets::anim::RotKey {
                time: i as f32 * 0.5,
                rotation: glam::Quat::from_rotation_z(angle),
            })
            .collect(),
        pos_keys: Vec::new(),
    }];
    animation
}

pub(super) fn fixtures(root: &Path) -> Result<(), String> {
    let write = |name: &str, data: &[u8]| std::fs::write(root.join(name), data).map_err(|e| e.to_string());
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let mut pixels = Vec::new();
    for y in 0..256 {
        for x in 0..512 {
            pixels.extend(if (x / 32 + y / 32) % 2 == 0 {
                [120, 65, 220, 255]
            } else {
                [55, 220, 190, 190]
            });
        }
    }
    image::save_buffer(root.join("texture-demo.png"), &pixels, 512, 256, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    let material = serde_json::json!({"asset":{"version":"2.0"},
        "images":[{"uri":"texture-demo.png"}],"textures":[{"source":0}],
        "materials":[{"name":"Demo", "pbrMetallicRoughness":{"baseColorTexture":{"index":0},"roughnessFactor":0.25,"metallicFactor":0.65}}]});
    write("material-demo.gltf", material.to_string().as_bytes())?;
    write("mesh-demo.dae", DAE.as_bytes())?;
    write("mesh-multiple.dae", multiple_dae().as_bytes())?;
    write("animation-demo.bvh", BVH)?;
    write("tail-preview.dae", tail_dae().as_bytes())?;
    write(
        "tail-preview.anim",
        &aurora_assets::import_animation::encode(&tail_animation()).map_err(|e| e.to_string())?,
    )?;
    let skeleton =
        aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml")).map_err(|e| e.to_string())?;
    let animation = aurora_assets::import_animation::parse_bvh(BVH, &skeleton).map_err(|e| e.to_string())?;
    write(
        "animation-demo.anim",
        &aurora_assets::import_animation::encode(&animation.animation).map_err(|e| e.to_string())?,
    )?;
    let samples: Vec<i16> = (0..44100 * 6)
        .map(|i| {
            let time = i as f32 / 44100.0;
            let envelope = 0.1 + 0.7 * (1.0 - (time * 2.0).fract());
            ((time * 440.0 * std::f32::consts::TAU).sin() * envelope * i16::MAX as f32) as i16
        })
        .collect();
    let length = samples.len() as u32 * 4;
    let mut wav = b"RIFF".to_vec();
    wav.extend((length + 36).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(44100u32.to_le_bytes());
    wav.extend((44100u32 * 4).to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(length.to_le_bytes());
    for sample in samples {
        wav.extend(sample.to_le_bytes());
        wav.extend(sample.to_le_bytes());
    }
    write("sound-demo.wav", &wav)
}

pub fn start(imports: &mut Imports, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
    let Ok(mut mode) = std::env::var("AURORA_DEMO_IMPORTS") else {
        return;
    };
    let type_windows = mode.starts_with("type-");
    if type_windows {
        mode = mode.trim_start_matches("type-").to_owned();
        if mode == "windows" {
            mode = "unified".into();
        }
    }
    if mode == "icon-bar" {
        imports.demo_mode = mode;
        imports.launcher_open = true;
        return;
    }
    if matches!(mode.as_str(), "launcher" | "settings" | "permissions") {
        imports.demo_mode = mode.clone();
        imports.open = true;
        imports.tab = if mode == "launcher" { ui::Tab::Batch } else { ui::Tab::Permissions };
        return;
    }
    imports.open = true;
    imports.demo_mode = mode.clone();
    imports.choice = match mode.as_str() {
        "texture" | "destination" => Choice::Texture,
        "material"
        | "material-cube"
        | "material-aurora"
        | "material-maps"
        | "material-zoom"
        | "material-texture-zoom"
        | "material-loading"
        | "material-confirm" => Choice::Material,
        "mesh" | "mesh-low" | "mesh-error" | "mesh-physics" | "mesh-degenerate" | "mesh-options" | "mesh-zoom" | "mesh-normals"
        | "mesh-colors" | "mesh-avatar" | "mesh-avatar-rigged" | "mesh-journal" | "mesh-multiple" => Choice::Mesh,
        "animation" | "animation-own" | "animation-custom" => Choice::Animation,
        "sound" | "sound-split" | "sound-folder" | "sound-confirm" => Choice::Sound,
        mode if mode.starts_with("local") => Choice::Local,
        _ => Choice::Bulk,
    };
    imports.mesh_permission_denied = mode == "bulk-mesh-denied";
    let choice = imports.choice;
    if type_windows {
        imports.open = false;
        imports.choice = Choice::Bulk;
        imports.launcher_open = true;
        let types: &[Choice] = if mode == "unified" {
            &[Choice::Texture, Choice::Sound]
        } else {
            std::slice::from_ref(&choice)
        };
        for (index, choice) in types.iter().enumerate() {
            imports.open_type_window(ctx, *choice);
            if types.len() > 1 {
                crate::ui::widgets::place_floater(
                    ctx,
                    choice.window_id(),
                    egui::pos2(16.0 + index as f32 * 776.0, 112.0),
                    egui::vec2(760.0, 780.0),
                );
            }
        }
    }
    if mode.starts_with("folders") {
        imports.folders.enabled = true;
        imports.tab = ui::Tab::Folder;
        if mode == "folders-type" {
            imports.folders.layout = folders::Layout::Type;
        } else if mode == "folders-flat" {
            imports.folders.layout = folders::Layout::Flat;
        }
        let request = Uuid::new_v4();
        imports.pending_files = Some(request);
        let tx = imports.tx.clone();
        let wake = ctx.clone();
        let prefs = imports.load_preferences.clone();
        jobs.execute(move || {
            let root = PathBuf::from(".scratch").join("import-demo").join(request.to_string());
            let entries = folder_fixtures(&root, &prefs).unwrap_or_else(|reason| vec![Entry::new(root, Err(reason), prefs.texture())]);
            let _ = tx.send(Worker::Loaded(request, entries));
            wake.request_repaint();
        });
        return;
    }
    if mode == "local-restored" {
        return;
    }
    if mode == "mesh-error" {
        imports.message = ui::error(Error::Refused(aurora_net::inventory::upload::Rejection {
            status: Some(400),
            identifier: "MAV_FOUND_DEGENERATE_TRIANGLES".into(),
            messages: vec!["MAV_FOUND_DEGENERATE_TRIANGLES".into()],
        }));
    }
    let local_mesh = mode.starts_with("local-mesh");
    let prefs = imports.load_preferences.clone();
    let request = Uuid::new_v4();
    imports.pending_files = Some(request);
    let tx = imports.tx.clone();
    let wake = ctx.clone();
    jobs.execute(move || {
        let root = PathBuf::from(".scratch").join("import-demo").join(request.to_string());
        let external = std::env::var_os("AURORA_DEMO_IMPORT_FILE").map(PathBuf::from);
        let fixture_result = if external.is_some() && mode != "animation-custom" {
            Ok(())
        } else {
            fixtures(&root)
        };
        let mut animation_models = Vec::new();
        let entries = match fixture_result {
            Ok(()) => {
                let files: &[(&str, Choice)] = match choice {
                    Choice::Bulk if mode == "inspectors" => &[
                        ("texture-demo.png", Choice::Texture),
                        ("material-demo.gltf", Choice::Material),
                        ("animation-demo.bvh", Choice::Animation),
                        ("animation-demo.anim", Choice::Animation),
                        ("sound-demo.wav", Choice::Sound),
                        ("mesh-demo.dae", Choice::Mesh),
                        ("texture-demo.png", Choice::Local),
                        ("material-demo.gltf", Choice::Local),
                        ("mesh-demo.dae", Choice::Local),
                    ],
                    Choice::Texture => &[("texture-demo.png", Choice::Texture)],
                    Choice::Material => &[("material-demo.gltf", Choice::Material)],
                    Choice::Mesh if mode == "mesh-multiple" => &[("mesh-multiple.dae", Choice::Mesh)],
                    Choice::Mesh => &[("mesh-demo.dae", Choice::Mesh)],
                    Choice::Animation if mode == "animation-custom" => &[("tail-preview.anim", Choice::Animation)],
                    Choice::Animation => &[
                        ("animation-demo.bvh", Choice::Animation),
                        ("animation-demo.anim", Choice::Animation),
                    ],
                    Choice::Sound => &[("sound-demo.wav", Choice::Sound)],
                    Choice::Bulk if mode.starts_with("unified") => &[
                        ("mesh-demo.dae", Choice::Bulk),
                        ("texture-demo.png", Choice::Bulk),
                        ("sound-demo.wav", Choice::Bulk),
                        ("animation-demo.bvh", Choice::Bulk),
                        ("material-demo.gltf", Choice::Bulk),
                    ],
                    Choice::Bulk if mode == "bulk-mesh-denied" => &[("mesh-demo.dae", Choice::Bulk), ("texture-demo.png", Choice::Bulk)],
                    Choice::Local if local_mesh => &[("mesh-demo.dae", Choice::Local)],
                    Choice::Local if mode.starts_with("local-material") => &[("material-demo.gltf", Choice::Local)],
                    Choice::Local if mode.starts_with("local-texture") => &[("texture-demo.png", Choice::Local)],
                    Choice::Local => &[
                        ("texture-demo.png", Choice::Local),
                        ("material-demo.gltf", Choice::Local),
                        ("mesh-demo.dae", Choice::Local),
                    ],
                    _ => &[
                        ("texture-demo.png", Choice::Bulk),
                        ("material-demo.gltf", Choice::Bulk),
                        ("animation-demo.anim", Choice::Bulk),
                        ("sound-demo.wav", Choice::Bulk),
                        ("mesh-demo.dae", Choice::Bulk),
                    ],
                };
                let paths: Vec<_> = if let Some(path) = external {
                    vec![(path, choice)]
                } else {
                    files.iter().map(|(name, loader)| (root.join(name), *loader)).collect()
                };
                let mut entries = Vec::new();
                for (path, loader) in paths {
                    match files::load(
                        &path,
                        loader,
                        prefs.texture(),
                        30,
                        &prefs,
                        choice == Choice::Local || loader == Choice::Local,
                    ) {
                        Ok(mut loaded) => {
                            if mode == "animation-custom" {
                                let avatars = crate::scene::avatar::AvatarLibrary::load();
                                for entry in &mut loaded {
                                    wake.data_mut(|data| data.insert_temp(egui::Id::unique(("animation_model_list_open", entry.id)), true));
                                    match animation_model::load(root.join("tail-preview.dae"), prefs.mesh_model_limit, &avatars) {
                                        Ok(model) => {
                                            // Feed the ordinary completion path after Loaded so the
                                            // synthetic source exercises the saved model selector too.
                                            animation_models.push(model);
                                        }
                                        Err(error) => entry.animation_model.error = Some(error),
                                    }
                                }
                            }
                            if mode == "mesh-colors"
                                && let Some(entry) = loaded.first()
                            {
                                wake.data_mut(|data| data.insert_temp(egui::Id::unique(("import_preview_colors_open", entry.id)), true));
                            }
                            if mode == "sound-split" {
                                for entry in &mut loaded {
                                    entry.sound.options = sound::Options {
                                        gain: 2.0,
                                        start: 0.25,
                                        end: Some(5.5),
                                        segment: Some(2.0),
                                        ..Default::default()
                                    };
                                }
                            }
                            if matches!(mode.as_str(), "material-cube" | "material-aurora") {
                                for entry in &mut loaded {
                                    entry.material_shape = if mode == "material-cube" {
                                        preview::MaterialShape::Cube
                                    } else {
                                        preview::MaterialShape::Aurora
                                    };
                                }
                            }
                            if matches!(
                                mode.as_str(),
                                "material-maps" | "material-zoom" | "material-texture-zoom" | "material-loading" | "material-confirm"
                            ) {
                                for entry in &mut loaded {
                                    if let Some(Source::Material { images, .. }) = &mut entry.source {
                                        for slot in 1..4 {
                                            images[slot] = images[0].clone();
                                        }
                                    }
                                }
                            }
                            if matches!(mode.as_str(), "mesh-zoom" | "material-zoom" | "material-texture-zoom") {
                                // The ordinary UI reads this one-shot request after Loaded arrives.
                                if let Some(entry) = loaded.first() {
                                    wake.data_mut(|data| {
                                        data.insert_temp(
                                            egui::Id::unique("import_demo_zoom"),
                                            (entry.id, (mode == "material-texture-zoom").then_some(0usize)),
                                        );
                                    });
                                }
                            }
                            if mode == "mesh-low" {
                                for entry in &mut loaded {
                                    if let Some(Source::Mesh(draft)) = &mut entry.source {
                                        draft.preview_lod = 0;
                                    }
                                }
                            }
                            if mode == "mesh-normals" {
                                for entry in &mut loaded {
                                    if let Some(Source::Mesh(draft)) = &mut entry.source {
                                        draft.options.normals = true;
                                        draft.options.crease = 105.0;
                                    }
                                }
                            }
                            if matches!(mode.as_str(), "mesh-physics" | "mesh-degenerate") {
                                for entry in &mut loaded {
                                    if let Some(Source::Mesh(draft)) = &mut entry.source {
                                        draft.options.physics_source = 3;
                                        draft.preview_physics = true;
                                        entry.view.edges = true;
                                        if mode == "mesh-degenerate" {
                                            let scene = Arc::make_mut(&mut draft.scene);
                                            let face = &mut scene.models[0].faces[0];
                                            face.vertices[1].position = face.vertices[2].position;
                                        }
                                    }
                                }
                            }
                            if matches!(
                                mode.as_str(),
                                "mesh-options" | "mesh-avatar" | "mesh-avatar-rigged" | "mesh-journal"
                            ) {
                                for entry in &mut loaded {
                                    if mode == "mesh-avatar-rigged"
                                        && let Some(Source::Mesh(draft)) = &mut entry.source
                                    {
                                        let scene = Arc::make_mut(&mut draft.scene);
                                        for model in &mut scene.models {
                                            model.skin = Some(aurora_assets::import_mesh::Skin {
                                                names: vec!["mPelvis".into()],
                                                inverse_bind: vec![glam::Mat4::IDENTITY],
                                                alternate_bind: vec![glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 0.02))],
                                                bind_shape: glam::Mat4::IDENTITY,
                                                pelvis_offset: 0.0,
                                            });
                                            for vertex in model.faces.iter_mut().flat_map(|f| &mut f.vertices) {
                                                vertex.joints = [0; 4];
                                                vertex.weights = [1.0, 0.0, 0.0, 0.0];
                                            }
                                        }
                                        draft.inspector = mesh::inspector::Inspector::new(&draft.scene);
                                        draft.options.skin = true;
                                        draft.options.joints = true;
                                        draft.show_overrides = true;
                                        draft.preview_weights = true;
                                    }
                                    let section = match mode.as_str() {
                                        "mesh-options" => 2usize,
                                        "mesh-journal" => 4,
                                        _ => 3,
                                    };
                                    wake.data_mut(|d| d.insert_temp(egui::Id::unique(("mesh_section", entry.id)), section));
                                }
                            }
                            entries.append(&mut loaded);
                        }
                        Err(error) => entries.push(Entry::new(path, Err(error.to_string()), prefs.texture())),
                    }
                }
                if choice == Choice::Local {
                    files::localize(&mut entries, &prefs);
                }
                entries
            }
            Err(error) => vec![Entry::new(root, Err(error), prefs.texture())],
        };
        let _ = tx.send(Worker::Loaded(request, entries));
        for model in animation_models {
            let _ = tx.send(Worker::AnimationModel(0, Some(Ok(model))));
        }
        wake.request_repaint();
    });
}

fn folder_fixtures(root: &Path, prefs: &Preferences) -> Result<Vec<Entry>, String> {
    if let Some(folder) = std::env::var_os("AURORA_DEMO_IMPORT_FILE").map(PathBuf::from)
        && folder.is_dir()
    {
        return Ok(files::load_folder(&folder, prefs, prefs.texture(), 30, 0, 0));
    }
    fixtures(root)?;
    let pack = root.join("Creator Pack");
    for directory in ["Coat/Empty", "Hat", "Animations", "Sounds"] {
        std::fs::create_dir_all(pack.join(directory)).map_err(|e| e.to_string())?;
    }
    for (from, to) in [
        ("texture-demo.png", "Coat/Fabric.png"),
        ("texture-demo.png", "Hat/Fabric.png"),
        ("mesh-demo.dae", "Coat/Coat.dae"),
        ("animation-demo.anim", "Animations/Idle.anim"),
        ("sound-demo.wav", "Sounds/Bell.wav"),
    ] {
        std::fs::copy(root.join(from), pack.join(to)).map_err(|e| e.to_string())?;
    }
    std::fs::write(pack.join("Readme.txt"), b"Not an uploadable asset").map_err(|e| e.to_string())?;
    for suffix in prefs
        .mesh_suffixes
        .iter()
        .enumerate()
        .filter_map(|(level, suffix)| (level != 3).then_some(suffix))
    {
        if !suffix.is_empty() && !suffix.contains(['/', '\\', ':']) {
            std::fs::write(pack.join(format!("Coat/Coat_{suffix}.dae")), DAE).map_err(|error| error.to_string())?;
        }
    }
    let entries = files::load_folder(&pack, prefs, prefs.texture(), 30, 0, 0);
    Ok(entries)
}
