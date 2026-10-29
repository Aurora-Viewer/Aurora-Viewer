//! Local previews, following LLLocalBitmap, LLLocalGLTFMaterial and
//! VJLocalMeshObject (indra/newview, originally LGPL 2.1). File checks,
//! decoding, geometry, mips and staging are worker jobs.

use super::*;
use aurora_render::{StagedTexture, StagingPool};
use std::time::{Duration, Instant, SystemTime};

pub struct Texture {
    pub id: Uuid,
    pub staged: StagedTexture,
    pub alpha: crate::scene::jobs::AlphaKind,
    pub has_alpha: bool,
}
pub struct Prepared {
    pub entry: Uuid,
    pub generation: u64,
    pub textures: Vec<Texture>,
    pub material: Option<PbrMaterial>,
    pub meshes: Vec<Mesh>,
}
pub struct Mesh {
    pub id: Uuid,
    pub data: Arc<Vec<u8>>,
    pub header: aurora_assets::MeshHeader,
    pub skin: Option<Arc<aurora_assets::SkinInfo>>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Target {
    pub entry: Uuid,
    pub model: usize,
    pub use_scale: bool,
}
pub struct Rez {
    pub entry: Uuid,
    pub known: HashSet<Uuid>,
}
pub type Stamp = Vec<(PathBuf, Option<(SystemTime, u64)>)>;
pub fn stamp(paths: &[PathBuf]) -> Stamp {
    paths
        .iter()
        .map(|path| {
            (
                path.clone(),
                std::fs::metadata(path)
                    .ok()
                    .and_then(|m| m.modified().ok().map(|time| (time, m.len()))),
            )
        })
        .collect()
}
pub struct Watch {
    pub paths: Vec<PathBuf>,
    pub stamp: Stamp,
    pub next: Instant,
    pub pending: bool,
}
impl Watch {
    pub fn new(mut paths: Vec<PathBuf>) -> Self {
        paths.sort();
        paths.dedup();
        Self {
            stamp: stamp(&paths),
            paths,
            next: Instant::now() + Duration::from_secs(3),
            pending: false,
        }
    }
}
pub(super) fn prepare(
    id: Uuid,
    generation: u64,
    source: &Source,
    options: import_texture::Options,
    pool: Option<&StagingPool>,
) -> Result<Prepared, String> {
    let mut prepared = Prepared {
        entry: id,
        generation,
        textures: Vec::new(),
        material: None,
        meshes: Vec::new(),
    };
    let mut textures = Vec::new();
    match source {
        Source::Texture(image) => textures.push((id, image.clone())),
        Source::Material { value, images } => {
            let mut value = value.clone();
            let mut ids = [
                value.base_color_texture,
                value.normal_texture,
                value.metallic_roughness_texture,
                value.emissive_texture,
            ];
            for (slot, image) in images.iter().enumerate() {
                if let Some(image) = image
                    && image.asset.is_none()
                {
                    ids[slot] = Some(image.key);
                    textures.push((image.key, image.pixels.clone()));
                }
            }
            value.base_color_texture = ids[0];
            value.normal_texture = ids[1];
            value.metallic_roughness_texture = ids[2];
            value.emissive_texture = ids[3];
            prepared.material = Some(value);
        }
        Source::Mesh(draft) => {
            use aurora_assets::import_mesh::{encode, process};
            use sha2::{Digest, Sha256};
            let models = draft
                .prepared
                .as_ref()
                .map_or_else(|| super::mesh::prepare(draft).map(Arc::new), |p| Ok(p.clone()))
                .map_err(|e| e.to_string())?;
            for (index, lods) in models.lods.iter().enumerate() {
                let data = encode::asset(
                    lods,
                    None,
                    &process::Physics::default(),
                    encode::Options {
                        skin: draft.options.skin,
                        joints: draft.options.joints,
                        lock_scale: draft.options.lock_scale,
                        pelvis_offset: draft.options.pelvis,
                        ..Default::default()
                    },
                )
                .map_err(|e| e.to_string())?;
                let header = aurora_assets::parse_mesh_header(&data).map_err(|e| e.to_string())?;
                let skin = header
                    .skin
                    .and_then(|range| aurora_assets::mesh::section_bytes(&data, range))
                    .map(aurora_assets::decode_skin)
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .map(Arc::new);
                let mut hash = Sha256::new();
                hash.update(id.as_bytes());
                hash.update(generation.to_le_bytes());
                hash.update(index.to_le_bytes());
                let digest = hash.finalize();
                let mut bytes = [0u8; 16];
                bytes.copy_from_slice(&digest[..16]);
                prepared.meshes.push(Mesh {
                    id: Uuid::from_bytes(bytes),
                    data: Arc::new(data),
                    header,
                    skin,
                });
            }
        }
        _ => return Err(aurora_i18n::tr!("import-local-images-only").into()),
    }
    let mut seen = HashSet::new();
    for (id, pixels) in textures {
        let pool = pool.ok_or_else(|| aurora_i18n::tr!("import-local-staging-busy").to_owned())?;
        if !seen.insert(id) {
            continue;
        }
        let pixels = import_texture::prepare(&pixels, options).map_err(|e| e.to_string())?;
        let rgba = crate::scene::jobs::to_rgba(pixels.components, &pixels.data, pixels.width as usize * pixels.height as usize);
        let alpha = crate::scene::jobs::classify_alpha(&rgba);
        let has_alpha = pixels.components == 4;
        let mips = aurora_render::build_mips(pixels.width, pixels.height, rgba, 12);
        let refs = mips.iter().map(|(w, h, data)| (*w, *h, data.as_slice())).collect::<Vec<_>>();
        let staged = StagedTexture::new(pool, &refs).ok_or_else(|| aurora_i18n::tr!("import-local-staging-busy").to_owned())?;
        prepared.textures.push(Texture {
            id,
            staged,
            alpha,
            has_alpha,
        });
    }
    Ok(prepared)
}
impl Imports {
    pub fn target_local(&mut self, entry_id: Uuid, object: Uuid, faces: &[u8], use_scale: bool) {
        use crate::world::objects::LocalAppearance;
        let Some(entry) = self.entries.iter().find(|entry| entry.id == entry_id) else {
            return;
        };
        let appearance = match &entry.source {
            Some(Source::Mesh(draft)) => {
                self.local_targets.insert(
                    object,
                    Target {
                        entry: entry_id,
                        model: draft.scene.instances.get(entry.local_model).map_or(0, |instance| instance.model),
                        use_scale,
                    },
                );
                return;
            }
            Some(Source::Texture(_)) => LocalAppearance::Texture(entry_id),
            Some(Source::Material { .. }) => LocalAppearance::Material(entry_id),
            _ => return,
        };
        if faces.is_empty() {
            self.local_appearance_targets.retain(|(id, _), _| *id != object);
            self.local_appearance_targets.insert((object, None), appearance);
        } else {
            for face in faces {
                self.local_appearance_targets.insert((object, Some(*face)), appearance);
            }
        }
    }
    pub fn apply_appearance_targets(&self, objects: &mut crate::world::objects::ObjectStore) {
        for ((id, face), appearance) in &self.local_appearance_targets {
            if !self
                .entries
                .iter()
                .any(|entry| entry.id == appearance.entry() && entry.local_published)
            {
                continue;
            }
            let Some(index) = objects.index_of_uuid(id) else { continue };
            if objects.get(index).is_some_and(|object| {
                object.render.local_appearance.get(face) == Some(appearance)
                    && (face.is_some()
                        || object
                            .render
                            .local_appearance
                            .keys()
                            .all(|face| face.is_none() || self.local_appearance_targets.contains_key(&(*id, *face))))
            }) {
                continue;
            }
            if let Some(object) = objects.get_mut(index) {
                if face.is_none() {
                    object
                        .render
                        .local_appearance
                        .retain(|face, _| face.is_none() || self.local_appearance_targets.contains_key(&(*id, *face)));
                }
                object.render.local_appearance.insert(*face, *appearance);
                object.material_dirty = true;
                object.render.needs_records = true;
            }
        }
    }
    pub fn clear_appearance_targets(&mut self, objects: &mut crate::world::objects::ObjectStore, ids: &[Uuid], entry: Option<Uuid>) {
        self.local_appearance_targets
            .retain(|(id, _), appearance| !ids.contains(id) || entry.is_some_and(|entry| appearance.entry() != entry));
        for id in ids {
            let Some(index) = objects.index_of_uuid(id) else { continue };
            if let Some(object) = objects.get_mut(index) {
                let before = object.render.local_appearance.len();
                object
                    .render
                    .local_appearance
                    .retain(|_, appearance| entry.is_some_and(|entry| appearance.entry() != entry));
                if before != object.render.local_appearance.len() {
                    object.material_dirty = true;
                    object.render.needs_records = true;
                }
            }
        }
    }
    pub fn publish_local(&mut self, id: Uuid, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context, pool: Option<Arc<StagingPool>>) {
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.local && !e.local_pending) else {
            return;
        };
        let Some(source) = entry.source.clone() else {
            return;
        };
        entry.local_pending = true;
        entry.local_failed_generation = None;
        let generation = entry.preview_generation;
        let options = entry.options;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let result = prepare(id, generation, &source, options, pool.as_deref());
            let _ = tx.send(Worker::Local(id, generation, result));
            wake.request_repaint();
        });
    }
    /// Mesh assets are CPU data, so installing them must not depend on the
    /// graphics handle temporarily owned by App::frame during UI callbacks.
    pub fn install_local_meshes(&mut self, meshes: &mut crate::scene::meshes::MeshStreamer, jobs: &crate::scene::jobs::Jobs) -> bool {
        if self.local_uploads.is_empty() {
            return false;
        }
        let mut remaining = Vec::new();
        let mut changed = false;
        for prepared in self.local_uploads.drain(..) {
            if prepared.meshes.is_empty() || !prepared.textures.is_empty() || prepared.material.is_some() {
                remaining.push(prepared);
                continue;
            }
            let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == prepared.entry) else {
                jobs.discard(prepared);
                continue;
            };
            if entry.preview_generation != prepared.generation {
                entry.local_pending = false;
                jobs.discard(prepared);
                continue;
            }
            for id in entry.local_mesh_ids.drain(..) {
                meshes.remove_local(&id);
            }
            for mesh in prepared.meshes {
                entry.local_mesh_ids.push(mesh.id);
                meshes.insert_local(mesh.id, mesh.data, mesh.header, mesh.skin);
            }
            entry.local_published = true;
            entry.local_generation = prepared.generation;
            entry.local_pending = false;
            changed = true;
        }
        self.local_uploads = remaining;
        changed
    }
    pub fn watch_local(&mut self, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        let now = Instant::now();
        for entry in &mut self.entries {
            if !entry.local_live {
                continue;
            }
            let Some(watch) = &mut entry.watch else {
                continue;
            };
            if watch.pending || watch.next > now {
                continue;
            }
            watch.pending = true;
            watch.next = now + Duration::from_secs_f32(self.load_preferences.local_reload_seconds.clamp(0.1, 3600.0));
            let paths = watch.paths.clone();
            let id = entry.id;
            let tx = self.tx.clone();
            let wake = ctx.clone();
            jobs.execute(move || {
                let _ = tx.send(Worker::Watch(id, stamp(&paths)));
                wake.request_repaint();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_appearance_applies_selected_faces_and_restores_latest_server_state() {
        use crate::world::objects::{LocalAppearance, ObjectStore};
        let mut update = crate::demo::action_avatar(false);
        update.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        let id = update.full_id;
        let server_material = Uuid::new_v4();
        update.extra.render_materials = vec![(0, server_material)];
        let mut objects = ObjectStore::default();
        let index = objects.upsert(0, update.clone());
        let mut texture = Entry::new(
            "image.png".into(),
            Ok(Source::Texture(Arc::new(DecodedImage {
                width: 1,
                height: 1,
                components: 3,
                data: vec![255; 3],
                discard: 0,
            }))),
            Default::default(),
        );
        let texture_id = texture.id;
        let mut material = Entry::new(
            "material.gltf".into(),
            Ok(Source::Material {
                value: PbrMaterial::default(),
                images: Box::new(std::array::from_fn(|_| None)),
            }),
            Default::default(),
        );
        material.local_published = true;
        let material_id = material.id;
        texture.local = true;
        let mut imports = Imports {
            entries: vec![texture, material],
            ..Default::default()
        };
        imports.target_local(texture_id, id, &[], false);
        imports.apply_appearance_targets(&mut objects);
        assert!(
            objects.get(index).expect("object").render.local_appearance.is_empty(),
            "wait until the local asset is installed"
        );
        imports.entries[0].local_published = true;
        imports.apply_appearance_targets(&mut objects);
        let object = objects.get(index).expect("object");
        assert_eq!(object.visual_texture_face(0).texture, texture_id);
        assert_eq!(
            object.visual_material(0),
            None,
            "a texture preview replaces the visible PBR material"
        );
        assert_eq!(object.extra.render_materials, update.extra.render_materials);
        assert_ne!(object.te.as_ref().expect("server TE").face(0).texture, texture_id);
        imports.target_local(material_id, id, &[1], false);
        imports.apply_appearance_targets(&mut objects);
        assert_eq!(
            objects.get(index).expect("object").local_appearance(0),
            Some(LocalAppearance::Texture(texture_id))
        );
        assert_eq!(objects.get(index).expect("object").visual_material(1), Some(material_id));
        // Simulator updates remain authoritative underneath both local previews.
        update.extra.render_materials = vec![(0, Uuid::new_v4()), (1, server_material)];
        objects.upsert(0, update.clone());
        assert_eq!(objects.get(index).expect("object").visual_material(1), Some(material_id));
        imports.clear_appearance_targets(&mut objects, &[id], Some(material_id));
        assert_eq!(
            objects.get(index).expect("object").visual_material(1),
            None,
            "the underlying local texture remains"
        );
        imports.clear_appearance_targets(&mut objects, &[id], None);
        let object = objects.get(index).expect("object");
        assert_eq!(object.visual_material(0), Some(update.extra.render_materials[0].1));
        assert_eq!(object.visual_material(1), Some(server_material));
        assert_eq!(object.visual_texture_face(0), *update.texture_entry.as_ref().expect("TE").face(0));
        assert!(object.material_dirty && object.render.needs_records);
        assert!(imports.local_appearance_targets.is_empty());
    }

    #[test]
    fn local_mesh_jobs_encode_and_install_without_a_graphics_handle_and_replace_only_current_results() {
        let ctx = egui::Context::default();
        let (jobs, _results) = crate::scene::jobs::Jobs::new();
        let mut entry = Entry::new(
            "mesh.dae".into(),
            Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
            Default::default(),
        );
        entry.local = true;
        let id = entry.id;
        let mut imports = Imports {
            entries: vec![entry],
            ..Default::default()
        };
        let mut streamer =
            crate::scene::meshes::MeshStreamer::new(std::env::temp_dir().join(format!("aurora-local-mesh-{}", Uuid::new_v4())));
        let receive = |imports: &mut Imports| {
            let Worker::Local(received, generation, result) =
                imports.rx.recv_timeout(Duration::from_secs(10)).expect("local worker finishes")
            else {
                panic!("local mesh result")
            };
            assert_eq!(received, id);
            let prepared = result.expect("mesh preparation needs no GPU staging");
            assert_eq!(prepared.generation, generation);
            assert!(prepared.textures.is_empty() && prepared.material.is_none());
            let mesh = &prepared.meshes[0];
            let section = aurora_assets::mesh::section_bytes(&mesh.data, mesh.header.lods[3].expect("high LOD")).expect("high section");
            let faces = aurora_assets::decode_mesh_lod(section).expect("runtime decoder accepts the local mesh");
            assert_eq!(faces.iter().map(|face| face.indices.len() / 3).sum::<usize>(), 12);
            imports.local_uploads.push(prepared);
        };
        imports.publish_local(id, &jobs, &ctx, None);
        assert!(imports.entries[0].local_pending);
        receive(&mut imports);
        assert!(imports.install_local_meshes(&mut streamer, &jobs));
        let original = imports.entries[0].local_mesh_ids[0];
        assert!(streamer.meta(&original).is_some());
        assert!(imports.entries[0].local_published);
        assert!(!imports.entries[0].local_pending);

        imports.publish_local(id, &jobs, &ctx, None);
        receive(&mut imports);
        imports.entries[0].invalidate_preview();
        assert!(
            !imports.install_local_meshes(&mut streamer, &jobs),
            "an obsolete result cannot replace the active mesh"
        );
        assert_eq!(imports.entries[0].local_mesh_ids, [original]);
        assert!(
            !imports.entries[0].local_pending,
            "an obsolete result must not prevent a new request"
        );

        imports.publish_local(id, &jobs, &ctx, None);
        receive(&mut imports);
        assert!(imports.install_local_meshes(&mut streamer, &jobs));
        let replacement = imports.entries[0].local_mesh_ids[0];
        assert_ne!(replacement, original);
        assert!(streamer.meta(&replacement).is_some());
        assert!(streamer.meta(&original).is_none(), "reloading retires the former local mesh");
        assert_eq!(imports.entries[0].local_generation, imports.entries[0].preview_generation);
        assert!(imports.local_uploads.is_empty());
    }
}
