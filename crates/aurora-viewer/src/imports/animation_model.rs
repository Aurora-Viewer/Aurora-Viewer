//! Local rigged models for animation inspection (#1283). This extends
//! LLPreviewAnimation's dummy avatar (indra/newview/llfloaterbvhpreview.cpp).
//! Only geometry and the preview skeleton change; the upload source stays intact.

use super::*;
use crate::scene::{anim::SkeletonBase, avatar::AvatarLibrary};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub id: Uuid,
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory: Option<super::animation_inventory::Origin>,
}

impl Saved {
    fn remember(models: &mut Vec<Self>, path: PathBuf) -> Uuid {
        if let Some(saved) = models.iter().find(|saved| saved.path == path) {
            return saved.id;
        }
        let saved = Self {
            id: Uuid::new_v4(),
            name: path.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
            path,
            inventory: None,
        };
        let id = saved.id;
        models.push(saved);
        id
    }
}

pub struct Model {
    pub path: PathBuf,
    pub geometry: Arc<aurora_render::preview::Model>,
    pub base: SkeletonBase,
}

#[derive(Default)]
pub struct Selection {
    pub current: Option<Arc<Model>>,
    pub generation: u64,
    pub pending: bool,
    pub error: Option<String>,
    pub saved: Option<Uuid>,
    requested: Option<Uuid>,
}

pub(super) fn load(path: PathBuf, model_limit: u32, avatars: &AvatarLibrary) -> Result<Arc<Model>, String> {
    if !path.is_file() {
        return Err(aurora_i18n::tr!("import-local-file-missing").into());
    }
    let bytes = files::read(&path).map_err(|e| e.to_string())?;
    if path.extension().is_some_and(|ext| ext == "apmodel") {
        return super::animation_inventory::snapshot::decode(path, &bytes, avatars);
    }
    let (scene, images) = files::mesh_scene(&path, &bytes, false, None, model_limit).map_err(|e| e.to_string())?;
    prepare(path, mesh::Draft::new(scene, images), avatars)
}

pub(super) fn prepare(path: PathBuf, mut draft: mesh::Draft, avatars: &AvatarLibrary) -> Result<Arc<Model>, String> {
    let rigged = draft.scene.instances.iter().any(|instance| {
        draft.scene.models.get(instance.model).is_some_and(|model| {
            model.skin.as_ref().is_some_and(|skin| {
                !skin.names.is_empty()
                    && skin.names.iter().all(|name| avatars.rig.names.contains_key(name))
                    && model
                        .faces
                        .iter()
                        .flat_map(|face| &face.vertices)
                        .any(|v| v.weights.iter().any(|w| *w > 0.0))
            })
        })
    });
    if !rigged {
        return Err(aurora_i18n::tr!("import-animation-model-rigged").into());
    }
    draft.preview_weights = true;
    draft.show_overrides = true;
    draft.show_joints = false;
    draft.show_textures = true;
    let base = mesh::preview::imported_base(&draft, &avatars.rig);
    // Preview the supplied high LOD directly: no simplification, physics,
    // fee request or inventory entry is needed for an animation's model.
    let mut geometry =
        preview::build(&Source::Mesh(Box::new(draft)), 0, Default::default(), Default::default(), avatars).map_err(|e| e.to_string())?;
    frame_rotations(&mut geometry, &base, &avatars.rig);
    Ok(Arc::new(Model {
        path,
        geometry: Arc::new(geometry),
        base,
    }))
}

/// Fit rotating tails/wings around the pelvis rather than the mesh's rest
/// bounds. Bone-chain lengths plus each vertex's joint-local distance bound
/// rotations without sampling clips or scanning geometry on the frame thread.
pub(super) fn frame_rotations(geometry: &mut aurora_render::preview::Model, base: &SkeletonBase, rig: &crate::scene::anim::Rig) {
    let world = rig.world_with(&rig.local_rot, glam::Vec3::ZERO, &base.local_pos, &base.scale);
    let center = world[rig.pelvis].w_axis.truncate();
    let mut reach = vec![0.0; rig.len()];
    for &joint in &rig.order {
        if joint != rig.pelvis {
            let parent_scale = rig.parent[joint].map_or(rig.parent_scale[joint], |parent| base.scale[parent]);
            reach[joint] = rig.parent[joint].map_or(0.0, |parent| reach[parent]) + (base.local_pos[joint] * parent_scale).length();
        }
    }
    let mut radius = 0.01f32;
    for part in &geometry.parts {
        for vertex in part.vertices.iter() {
            let point = glam::Vec3::from_array(vertex.position);
            let mut weighted = false;
            if let Some(skin) = &part.skin {
                for (index, weight) in vertex.joints.iter().zip(vertex.weights) {
                    if weight <= 0.0 {
                        continue;
                    }
                    if let (Some(&joint), Some(inverse)) = (skin.joints.get(*index as usize), skin.inverse_bind.get(*index as usize)) {
                        let local = inverse.transform_point3(point) * base.scale[joint];
                        radius = radius.max(reach[joint] + local.length());
                        weighted = true;
                    }
                }
            }
            if !weighted {
                radius = radius.max(point.distance(center));
            }
        }
    }
    geometry.center = center;
    geometry.radius = radius;
}

fn refresh(entry: &mut Entry) {
    entry.invalidate_preview();
    // Publish geometry and its rest skeleton together, including while paused.
    // A previous worker's dummy avatar cannot replace this newer generation.
    entry.model = entry.animation_model.current.as_ref().map(|model| model.geometry.clone());
    entry.model_generation = entry.model.as_ref().map(|_| entry.preview_generation);
    entry.preview_pending = false;
    entry.player.pose = None;
    entry.view = Default::default();
}

pub fn reset(entry: &mut Entry) {
    entry.animation_model.generation += 1;
    entry.animation_model.current = None;
    entry.animation_model.pending = false;
    entry.animation_model.error = None;
    entry.animation_model.saved = None;
    entry.animation_model.requested = None;
    refresh(entry);
}

pub(super) fn complete(entry: &mut Entry, generation: u64, result: Option<Result<Arc<Model>, String>>) {
    if entry.animation_model.generation != generation || !matches!(entry.source, Some(Source::Animation { .. })) {
        return;
    }
    entry.animation_model.pending = false;
    match result {
        Some(Ok(model)) => {
            entry.animation_model.current = Some(model);
            entry.animation_model.error = None;
            refresh(entry);
        }
        Some(Err(error)) => entry.animation_model.error = Some(error),
        None => {}
    }
}

impl Imports {
    pub(super) fn complete_animation_model(&mut self, id: Uuid, generation: u64, result: Option<Result<Arc<Model>, String>>) {
        let Some(entry) = self.entries.iter_mut().find(|entry| {
            entry.id == id && entry.animation_model.generation == generation && matches!(entry.source, Some(Source::Animation { .. }))
        }) else {
            return;
        };
        if let Some(Ok(model)) = &result {
            let saved = entry.animation_model.requested.take().unwrap_or_else(|| {
                self.animation_models_dirty = true;
                Saved::remember(&mut self.load_preferences.animation_models, model.path.clone())
            });
            self.animation_model_cache.insert(saved, Arc::downgrade(model));
            entry.animation_model.saved = Some(saved);
        }
        entry.animation_model.requested = None;
        complete(entry, generation, result);
    }

    pub fn select_animation_model(
        &mut self,
        id: Uuid,
        saved: Uuid,
        jobs: &crate::scene::jobs::Jobs,
        ctx: &egui::Context,
        avatars: Arc<AvatarLibrary>,
    ) {
        let Some(model) = self.load_preferences.animation_models.iter().find(|model| model.id == saved) else {
            return;
        };
        if model
            .inventory
            .as_ref()
            .is_some_and(|origin| origin.profile != self.animation_inventory.profile)
        {
            return;
        }
        let path = model.path.clone();
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id && matches!(entry.source, Some(Source::Animation { .. })))
        else {
            return;
        };
        entry.animation_model.generation += 1;
        let generation = entry.animation_model.generation;
        entry.animation_model.requested = Some(saved);
        entry.animation_model.saved = Some(saved);
        entry.animation_model.error = None;
        if let Some(model) = self.animation_model_cache.get(&saved).and_then(std::sync::Weak::upgrade) {
            self.complete_animation_model(id, generation, Some(Ok(model)));
            return;
        }
        entry.animation_model.pending = true;
        let model_limit = self.load_preferences.mesh_model_limit;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let result = load(path, model_limit, &avatars);
            let _ = tx.send(Worker::AnimationModel(id, generation, Some(result)));
            wake.request_repaint();
        });
    }

    pub fn remove_animation_model(&mut self, saved: Uuid, jobs: &crate::scene::jobs::Jobs) {
        let path = self
            .load_preferences
            .animation_models
            .iter()
            .find(|model| model.id == saved)
            .map(|model| model.path.clone());
        self.load_preferences.animation_models.retain(|model| model.id != saved);
        self.animation_models_dirty = true;
        for entry in &mut self.entries {
            if entry.animation_model.saved == Some(saved)
                || entry.animation_model.requested == Some(saved)
                || entry
                    .animation_model
                    .current
                    .as_ref()
                    .is_some_and(|model| Some(&model.path) == path.as_ref())
            {
                reset(entry);
            }
        }
        jobs.discard(self.animation_model_cache.remove(&saved));
    }

    pub fn animation_model_file(&mut self, id: Uuid, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context, avatars: Arc<AvatarLibrary>) {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.id == id && matches!(e.source, Some(Source::Animation { .. })))
        else {
            return;
        };
        if entry.animation_model.pending {
            return;
        }
        entry.animation_model.generation += 1;
        let generation = entry.animation_model.generation;
        entry.animation_model.pending = true;
        entry.animation_model.error = None;
        entry.animation_model.requested = None;
        let model_limit = self.load_preferences.mesh_model_limit;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let result = rfd::FileDialog::new()
                .set_title(aurora_i18n::tr!("import-animation-model-choose"))
                .add_filter(aurora_i18n::tr!("import-mesh"), &["dae", "gltf", "glb"])
                .pick_file()
                .map(|path| load(path, model_limit, &avatars));
            let _ = tx.send(Worker::AnimationModel(id, generation, result));
            wake.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests;
