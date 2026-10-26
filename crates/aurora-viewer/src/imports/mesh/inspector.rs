//! Import diagnostics from actual decoded models and completed worker results.
//! Reference: LLFloaterModelPreview::updateAvatarTab / addStringToLogTab
//! (indra/newview/llfloatermodelpreview.cpp, originally LGPL 2.1).
use aurora_assets::import_mesh::Scene;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Default)]
pub struct Avatar {
    // Model names are display labels, not unique identifiers.
    pub joints: BTreeMap<String, Vec<(String, Option<[f32; 3]>)>>,
    pub has_skin: bool,
    pub unknown: bool,
    pub too_many: bool,
}
impl Avatar {
    pub fn new(scene: &Scene) -> Self {
        let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../../assets/character/avatar_skeleton.xml")).ok();
        let mut result = Self::default();
        for model in &scene.models {
            let Some(skin) = &model.skin else { continue };
            result.has_skin = true;
            result.too_many |= skin.names.len() > 110;
            for (index, name) in skin.names.iter().enumerate() {
                let joint = skeleton.as_ref().and_then(|s| s.find(name).map(|i| &s.joints[i]));
                result.unknown |= joint.is_none();
                let position = skin.alternate_bind.get(index).and_then(|matrix| {
                    let position = matrix.w_axis.truncate();
                    (position.is_finite() && joint.is_some_and(|j| j.pos.distance_squared(position) > 0.0001f32.powi(2)))
                        .then_some(position.to_array())
                });
                result.joints.entry(name.clone()).or_default().push((model.name.clone(), position));
            }
        }
        for models in result.joints.values_mut() {
            models.sort_by(|a, b| a.0.cmp(&b.0));
        }
        result
    }
    pub fn valid(&self) -> bool {
        self.has_skin && !self.unknown
    }
    pub fn conflicts(models: &[(String, Option<[f32; 3]>)]) -> bool {
        let mut positions = models.iter().filter_map(|(_, position)| position.as_ref());
        let Some(first) = positions.next() else { return false };
        positions.any(|position| glam::Vec3::from_array(*first).distance_squared(glam::Vec3::from_array(*position)) > 0.0001f32.powi(2))
    }
}

#[derive(Clone)]
pub struct Inspector {
    pub avatar: Arc<Avatar>,
    pub lod_avatars: BTreeMap<usize, Arc<Avatar>>,
    pub selected_joint: String,
    pub log: String,
    pub verbose: bool,
    pub unread: bool,
    characters: usize,
}
impl Inspector {
    pub fn new(scene: &Scene) -> Self {
        let mut result = Self {
            avatar: Arc::new(Avatar::new(scene)),
            lod_avatars: BTreeMap::new(),
            selected_joint: "mPelvis".into(),
            log: String::new(),
            verbose: false,
            unread: false,
            characters: 0,
        };
        result.push(aurora_i18n::tr!("import-log-loaded", models = scene.models.len()), false);
        for warning in &scene.warnings {
            result.push(warning, true);
        }
        result
    }
    pub fn clear(&mut self) {
        self.log.clear();
        self.characters = 0;
        self.unread = false;
    }
    pub fn push(&mut self, message: impl AsRef<str>, attention: bool) {
        let message = message.as_ref();
        let count = message.chars().count();
        if message.is_empty() || count + 1 > 65536 {
            return;
        }
        if !self.log.is_empty() {
            self.log.push('\n');
            self.characters += 1;
        }
        self.log.push_str(message);
        self.characters += count;
        while self.characters > 65536 {
            let Some(end) = self.log.find('\n') else {
                self.clear();
                return;
            };
            self.characters -= self.log[..=end].chars().count();
            self.log.drain(..=end);
        }
        self.unread |= attention;
    }
    pub fn prepared(&mut self, prepared: &super::Prepared) {
        for level in (0..4).rev() {
            let models: Vec<_> = prepared.lods.iter().filter_map(|lods| lods[level].as_ref()).collect();
            let triangles: usize = models.iter().map(|model| model.triangles()).sum();
            self.push(
                aurora_i18n::tr!("import-log-lod-ready", level = level, triangles = triangles),
                false,
            );
            if self.verbose {
                for model in models {
                    let vertices: usize = model.faces.iter().map(|face| face.vertices.len()).sum();
                    self.push(
                        aurora_i18n::tr!(
                            "import-log-model-detail",
                            model = model.name.as_str(),
                            level = level,
                            vertices = vertices,
                            faces = model.faces.len()
                        ),
                        false,
                    );
                }
            }
        }
        if prepared.degenerate_triangles > 0 {
            self.push(
                aurora_i18n::tr!("import-degenerate-physics", count = prepared.degenerate_triangles),
                true,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_keeps_whole_unicode_lines_and_rejects_oversized_messages() {
        let mut log = Inspector::new(&Scene::default());
        log.clear();
        log.push("é".repeat(65000), false);
        log.push("z".repeat(1000), true);
        assert_eq!(log.log, "z".repeat(1000));
        assert!(log.unread);
        log.push("x".repeat(65536), false);
        assert_eq!(log.log.len(), 1000);
        log.clear();
        assert!(log.log.is_empty());
        assert!(!log.unread);
    }
    #[test]
    fn conflict_threshold_ignores_missing_and_matching_overrides() {
        let mut models = vec![("a".into(), Some([0.0, 0.0, 0.02])), ("b".into(), None)];
        assert!(!Avatar::conflicts(&models));
        models[1].1 = Some([0.0, 0.0, 0.02001]);
        assert!(!Avatar::conflicts(&models));
        models[1].1 = Some([0.0, 0.0, 0.04]);
        assert!(Avatar::conflicts(&models));
    }

    #[test]
    fn repeated_model_names_keep_every_joint_position_and_conflict() {
        use aurora_assets::import_mesh::{Model, Skin};
        use glam::{Mat4, Vec3};
        let scene = Scene {
            models: [0.02, 0.04]
                .into_iter()
                .map(|z| Model {
                    name: "Repeated".into(),
                    submodel: 0,
                    faces: Vec::new(),
                    skin: Some(Skin {
                        names: vec!["mPelvis".into()],
                        inverse_bind: vec![Mat4::IDENTITY],
                        alternate_bind: vec![Mat4::from_translation(Vec3::new(0.0, 0.0, z))],
                        bind_shape: Mat4::IDENTITY,
                        pelvis_offset: 0.0,
                    }),
                    center: Vec3::ZERO,
                    size: Vec3::ONE,
                })
                .collect(),
            ..Default::default()
        };
        let avatar = Avatar::new(&scene);
        assert!(avatar.valid());
        let models = &avatar.joints["mPelvis"];
        assert_eq!(models.len(), 2);
        assert!(models.iter().all(|(name, _)| name == "Repeated"));
        assert_eq!(models[0].1, Some([0.0, 0.0, 0.02]));
        assert_eq!(models[1].1, Some([0.0, 0.0, 0.04]));
        assert!(Avatar::conflicts(models));
    }
}
