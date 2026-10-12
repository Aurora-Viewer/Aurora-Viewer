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
    write("animation-demo.bvh", BVH)?;
    let skeleton =
        aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml")).map_err(|e| e.to_string())?;
    let animation = aurora_assets::import_animation::parse_bvh(BVH, &skeleton).map_err(|e| e.to_string())?;
    write(
        "animation-demo.anim",
        &aurora_assets::import_animation::encode(&animation.animation).map_err(|e| e.to_string())?,
    )?;
    let samples: Vec<i16> = (0..44100)
        .map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / 44100.0).sin() * 0.15 * i16::MAX as f32) as i16)
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
    let Ok(mode) = std::env::var("AURORA_DEMO_IMPORTS") else { return };
    imports.open = true;
    imports.demo_mode = mode.clone();
    imports.choice = match mode.as_str() {
        "texture" | "destination" => Choice::Texture,
        "material" => Choice::Material,
        "mesh" | "mesh-low" | "mesh-physics" | "mesh-degenerate" | "mesh-options" => Choice::Mesh,
        "animation" | "animation-own" => Choice::Animation,
        "sound" => Choice::Sound,
        "local" | "local-mesh" | "local-mesh-applied" => Choice::Local,
        _ => Choice::Bulk,
    };
    let choice = imports.choice;
    let local_mesh = mode.starts_with("local-mesh");
    imports.load_preferences.local_meshes = local_mesh;
    let prefs = imports.load_preferences.clone();
    let request = Uuid::new_v4();
    imports.pending_files = Some(request);
    let tx = imports.tx.clone();
    let wake = ctx.clone();
    jobs.execute(move || {
        let root = PathBuf::from(".scratch").join("import-demo").join(request.to_string());
        let external = std::env::var_os("AURORA_DEMO_IMPORT_FILE").map(PathBuf::from);
        let fixture_result = if external.is_some() { Ok(()) } else { fixtures(&root) };
        let entries = match fixture_result {
            Ok(()) => {
                let files: &[(&str, Choice)] = match choice {
                    Choice::Texture => &[("texture-demo.png", Choice::Texture)],
                    Choice::Material => &[("material-demo.gltf", Choice::Material)],
                    Choice::Mesh => &[("mesh-demo.dae", Choice::Mesh)],
                    Choice::Animation => &[
                        ("animation-demo.bvh", Choice::Animation),
                        ("animation-demo.anim", Choice::Animation),
                    ],
                    Choice::Sound => &[("sound-demo.wav", Choice::Sound)],
                    Choice::Local if local_mesh => &[("mesh-demo.dae", Choice::Mesh)],
                    Choice::Local => &[("texture-demo.png", Choice::Local), ("material-demo.gltf", Choice::Local)],
                    _ => &[
                        ("texture-demo.png", Choice::Bulk),
                        ("material-demo.gltf", Choice::Bulk),
                        ("animation-demo.anim", Choice::Bulk),
                        ("sound-demo.wav", Choice::Bulk),
                    ],
                };
                let paths: Vec<_> = if let Some(path) = external {
                    vec![(path, if local_mesh { Choice::Mesh } else { choice })]
                } else {
                    files.iter().map(|(name, loader)| (root.join(name), *loader)).collect()
                };
                let mut entries = Vec::new();
                for (path, loader) in paths {
                    match files::load(&path, loader, prefs.texture(), 30, &prefs, choice == Choice::Local) {
                        Ok(mut loaded) => {
                            if mode == "mesh-low" {
                                for entry in &mut loaded {
                                    if let Some(Source::Mesh(draft)) = &mut entry.source {
                                        draft.preview_lod = 0;
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
                            if mode == "mesh-options" {
                                for entry in &loaded {
                                    wake.data_mut(|d| d.insert_temp(egui::Id::new(("mesh_section", entry.id)), 2usize));
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
        wake.request_repaint();
    });
}
