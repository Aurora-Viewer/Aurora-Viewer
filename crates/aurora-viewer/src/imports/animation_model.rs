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
}

pub(super) fn deserialize_local_models<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<Saved>, D::Error> {
    let mut models = Vec::<Saved>::deserialize(deserializer)?;
    // Retire references created by the removed inventory-preview experiment.
    // Their private format is no longer loadable; local source files stay put.
    models.retain(|model| !model.path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("apmodel")));
    Ok(models)
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

#[derive(Clone, Default)]
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
fn frame_rotations(geometry: &mut aurora_render::preview::Model, base: &SkeletonBase, rig: &crate::scene::anim::Rig) {
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

impl Imports {
    /// A single revision covers the queue, so removing the originating row
    /// cannot orphan a load, and a late worker cannot undo a newer choice.
    pub(super) fn complete_animation_model(&mut self, generation: u64, result: Option<Result<Arc<Model>, String>>) {
        if self.animation_model_selection.generation != generation {
            return;
        }
        let selection = &mut self.animation_model_selection;
        selection.pending = false;
        match result {
            Some(Ok(model)) => {
                let saved = selection
                    .requested
                    .unwrap_or_else(|| Saved::remember(&mut self.load_preferences.animation_models, model.path.clone()));
                self.animation_model_cache.insert(saved, Arc::downgrade(&model));
                selection.saved = Some(saved);
                selection.current = Some(model);
                selection.error = None;
                self.load_preferences.animation_model_last = Some(saved);
                self.animation_models_dirty = true;
            }
            Some(Err(error)) => selection.error = Some(error),
            None => {}
        }
        selection.requested = None;
        self.sync_animation_models();
    }

    pub(super) fn sync_animation_models(&mut self) {
        for entry in &mut self.entries {
            if !matches!(entry.source, Some(Source::Animation { .. })) {
                continue;
            }
            let changed = match (&entry.animation_model.current, &self.animation_model_selection.current) {
                (Some(current), Some(next)) => !Arc::ptr_eq(current, next),
                (None, None) => false,
                _ => true,
            };
            entry.animation_model = self.animation_model_selection.clone();
            if changed {
                refresh(entry);
            }
        }
    }

    pub(super) fn restore_animation_model(&mut self, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context, avatars: Arc<AvatarLibrary>) {
        if self.animation_model_selection.generation == 0
            && self.animation_model_selection.current.is_none()
            && let Some(saved) = self.load_preferences.animation_model_last
        {
            if !self.load_preferences.animation_models.iter().any(|model| model.id == saved) {
                self.load_preferences.animation_model_last = None;
                self.animation_models_dirty = true;
            } else if let Some(entry) = self
                .entries
                .iter()
                .find(|entry| matches!(entry.source, Some(Source::Animation { .. })))
            {
                self.select_animation_model(entry.id, saved, jobs, ctx, avatars);
            }
        }
    }

    pub fn reset_animation_models(&mut self, id: Uuid) {
        if !self
            .entries
            .iter()
            .any(|entry| entry.id == id && matches!(entry.source, Some(Source::Animation { .. })))
        {
            return;
        }
        self.reset_animation_model_selection();
    }

    fn reset_animation_model_selection(&mut self) {
        self.animation_model_selection = Selection {
            generation: self.animation_model_selection.generation + 1,
            ..Default::default()
        };
        self.load_preferences.animation_model_last = None;
        self.animation_models_dirty = true;
        self.sync_animation_models();
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
        let path = model.path.clone();
        if !self
            .entries
            .iter()
            .any(|entry| entry.id == id && matches!(entry.source, Some(Source::Animation { .. })))
        {
            return;
        }
        let selection = &mut self.animation_model_selection;
        selection.generation += 1;
        let generation = selection.generation;
        selection.requested = Some(saved);
        selection.saved = Some(saved);
        selection.error = None;
        selection.pending = true;
        if let Some(model) = self.animation_model_cache.get(&saved).and_then(std::sync::Weak::upgrade) {
            self.complete_animation_model(generation, Some(Ok(model)));
            return;
        }
        self.sync_animation_models();
        let model_limit = self.load_preferences.mesh_model_limit;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let result = load(path, model_limit, &avatars);
            let _ = tx.send(Worker::AnimationModel(generation, Some(result)));
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
        if self.load_preferences.animation_model_last == Some(saved) {
            self.load_preferences.animation_model_last = None;
        }
        let selection = &self.animation_model_selection;
        if selection.saved == Some(saved)
            || selection.requested == Some(saved)
            || selection.current.as_ref().is_some_and(|model| Some(&model.path) == path.as_ref())
        {
            self.reset_animation_model_selection();
        }
        jobs.discard(self.animation_model_cache.remove(&saved));
    }

    pub fn animation_model_file(&mut self, id: Uuid, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context, avatars: Arc<AvatarLibrary>) {
        if !self
            .entries
            .iter()
            .any(|e| e.id == id && matches!(e.source, Some(Source::Animation { .. })))
        {
            return;
        }
        let selection = &mut self.animation_model_selection;
        if selection.pending {
            return;
        }
        selection.generation += 1;
        let generation = selection.generation;
        selection.pending = true;
        selection.error = None;
        selection.requested = None;
        self.sync_animation_models();
        let model_limit = self.load_preferences.mesh_model_limit;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let result = rfd::FileDialog::new()
                .set_title(aurora_i18n::tr!("import-animation-model-choose"))
                .add_filter(aurora_i18n::tr!("import-mesh"), &["dae", "gltf", "glb"])
                .pick_file()
                .map(|path| load(path, model_limit, &avatars));
            let _ = tx.send(Worker::AnimationModel(generation, result));
            wake.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests;
