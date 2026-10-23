//! Explicit named presets over one shared rig declaration. Selection is scoped
//! to the authenticated source and does not inspect or control animations.
use crate::world::rig_graph::Definition;

#[derive(Clone, Default)]
pub(super) struct Selection {
    pub name: String,
    pub cycles: Vec<u32>,
}

impl Selection {
    pub fn valid(&self, definition: &Definition, enabled: bool) -> bool {
        self.name.len() <= 64
            && ((self.cycles.is_empty() && definition.profiles.is_empty()) || self.cycles.len() == definition.bindings.len())
            && self.cycles.iter().all(|c| *c <= i32::MAX as u32)
            && if self.name.is_empty() {
                !enabled || definition.profiles.is_empty()
            } else {
                definition.profiles.iter().any(|p| p.name == self.name)
            }
    }
    pub fn allows(&self, definition: &Definition, index: usize, bone: &str) -> bool {
        if definition.profiles.is_empty() {
            self.name.is_empty()
        } else {
            definition
                .profiles
                .iter()
                .find(|p| p.name == self.name)
                .is_some_and(|p| p.contains(index, bone))
        }
    }
    pub fn smoothing(&self, definition: &Definition) -> Option<[f32; 2]> {
        definition.profiles.iter().find(|p| p.name == self.name)?.smoothing
    }
    pub fn cycle(&self, index: usize) -> u32 {
        self.cycles.get(index).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_profile_slots_distinguish_same_bone_calibrations_and_override_only_their_own_fades() {
        let definition: Definition = serde_json::from_value(serde_json::json!({
            "profiles":[{"name":"First pose","bindings":[0],"smoothing":[0.5,0.8]},
                {"name":"Second pose","bindings":[1],"smoothing":[0.1,0.2]}],
            "nodes":[],"bindings":[
                {"bone":"mWristLeft","target":{"kind":"source"}},
                {"bone":"mWristLeft","target":{"kind":"source"},"offset":{"position":[0.1,0.0,0.0],"rotation":[0.0,0.0,0.0,1.0]}}
            ]
        }))
        .unwrap();
        definition.validate().unwrap();
        let mut selected = Selection {
            name: "First pose".into(),
            cycles: vec![1, 1],
        };
        assert!(selected.valid(&definition, true));
        assert!(selected.allows(&definition, 0, "mWristLeft"));
        assert!(!selected.allows(&definition, 1, "mWristLeft"));
        assert_eq!(selected.smoothing(&definition), Some([0.5, 0.8]));
        selected.name = "Second pose".into();
        assert!(!selected.allows(&definition, 0, "mWristLeft"));
        assert!(selected.allows(&definition, 1, "mWristLeft"));
        assert_eq!(selected.smoothing(&definition), Some([0.1, 0.2]));
    }
}
