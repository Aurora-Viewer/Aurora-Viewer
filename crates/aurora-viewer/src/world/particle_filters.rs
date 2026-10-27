//! Local particle-only filters. Keep these separate from resident/object mutes:
//! filtering an emitter or owner must not affect voice, chat or avatar rendering.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParticleFilters {
    objects: HashSet<Uuid>,
    owners: HashSet<Uuid>,
}

impl ParticleFilters {
    pub fn blocked(&self, id: Uuid, owner: bool) -> bool {
        if id.is_nil() {
            return false;
        }
        if owner {
            self.owners.contains(&id)
        } else {
            self.objects.contains(&id)
        }
    }
    pub fn set(&mut self, id: Uuid, owner: bool, blocked: bool) -> bool {
        if id.is_nil() {
            return false;
        }
        let set = if owner { &mut self.owners } else { &mut self.objects };
        if blocked { set.insert(id) } else { set.remove(&id) }
    }
    pub fn hides(&self, object: Uuid, owner: Uuid) -> bool {
        self.blocked(object, false) || self.blocked(owner, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn particle_filters_are_independent_and_persist_without_mutes() {
        let (a, b, owner, other) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        let mut filters = ParticleFilters::default();
        filters.set(a, false, true);
        assert!(filters.hides(a, owner));
        assert!(!filters.hides(b, owner));
        filters.set(owner, true, true);
        assert!(filters.hides(b, owner));
        assert!(!filters.hides(b, other));
        filters.set(a, false, false);
        assert!(filters.hides(a, owner));
        let encoded = serde_json::to_string(&filters).unwrap();
        let mut loaded: ParticleFilters = serde_json::from_str(&encoded).unwrap();
        assert_eq!(loaded, filters);
        loaded.set(owner, true, false);
        assert!(!loaded.hides(a, owner));
        assert!(!loaded.hides(b, owner));
        assert!(!loaded.set(Uuid::nil(), true, true));
    }
}
