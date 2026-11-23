//! Private local-preview bindings follow inventory items across attachment re-rez.
//! LLVOAvatar::getWornAttachment / LLViewerObject::getAttachmentItemID supply
//! the identity; nothing is stored on the object or sent to the simulator.

use super::*;
use crate::world::{World, objects::Object};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct Identity {
    item: Uuid,
    parts: usize,
    part: Part,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum Part {
    Root,
    Child {
        mesh: Uuid,
        position: [u32; 3],
        rotation: [u32; 4],
    },
}

impl Part {
    fn of(object: &Object, root: bool) -> Self {
        if root {
            return Self::Root;
        }
        // Child arrival order is not a stable link number. Use source geometry
        // and the saved local transform; ambiguous matches are never applied.
        Self::Child {
            mesh: object.volume.sculpt.map_or(Uuid::nil(), |s| s.texture),
            position: object.position.to_array().map(f32::to_bits),
            rotation: object.rotation.to_array().map(f32::to_bits),
        }
    }
}

impl Identity {
    fn capture(world: &World, index: usize) -> Option<Self> {
        let root = crate::build::root_of(world, index);
        let object = world.objects.get(root)?;
        let parent = world.objects.parent_of(object)?;
        if world.objects.get(parent)?.full_id != world.agent_id {
            return None;
        }
        let item = object.attachment_item_id().filter(|id| !id.is_nil())?;
        Some(Self {
            item,
            parts: crate::build::family(world, root).len(),
            part: Part::of(world.objects.get(index)?, root == index),
        })
    }

    fn resolve(&self, world: &World) -> Option<Uuid> {
        let root = world.worn_attachment(&self.item)?;
        let family = crate::build::family(world, root);
        if family.len() != self.parts {
            return None;
        }
        let mut matches = family.into_iter().filter_map(|index| {
            let object = world.objects.get(index)?;
            (Part::of(object, index == root) == self.part).then_some(object.full_id)
        });
        let id = matches.next()?;
        matches.next().is_none().then_some(id)
    }
}

impl Imports {
    pub fn sync_attachment_targets(&mut self, world: &World) {
        let now = Instant::now();
        if self.persistence.attachment_next.is_some_and(|next| next > now) {
            return;
        }
        self.persistence.attachment_next = Some(now + Duration::from_millis(250));
        let ids: HashSet<_> = self.local_appearance_targets.keys().map(|(id, _)| *id).collect();
        self.persistence.attachments.retain(|id, _| ids.contains(id));
        for id in ids {
            if let Some(index) = world.objects.index_of_uuid(&id)
                && let Some(identity) = Identity::capture(world, index)
            {
                self.persistence.attachments.insert(id, identity);
            }
        }
        let moves: Vec<_> = self
            .persistence
            .attachments
            .iter()
            .filter_map(|(old, identity)| {
                let new = identity.resolve(world)?;
                (new != *old).then_some((*old, new, identity.clone()))
            })
            .collect();
        for (old, new, identity) in moves {
            let faces: Vec<_> = self.local_appearance_targets.keys().filter(|(id, _)| *id == old).copied().collect();
            for key in faces {
                let Some(appearance) = self.local_appearance_targets.remove(&key) else {
                    continue;
                };
                let deadline = self.persistence.appearance_expirations.remove(&key);
                if let std::collections::hash_map::Entry::Vacant(entry) = self.local_appearance_targets.entry((new, key.1)) {
                    entry.insert(appearance);
                    if let Some(deadline) = deadline {
                        self.persistence.appearance_expirations.insert((new, key.1), deadline);
                    }
                }
            }
            self.persistence.attachments.remove(&old);
            self.persistence.attachments.insert(new, identity);
            self.persistence.next_expiration = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creator_attachment_identity_refuses_incomplete_ambiguous_and_foreign_matches() {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        for event in crate::demo::events().into_iter().chain(crate::demo::worn::events()) {
            world.apply(event);
        }
        let root = world.worn_attachment(&crate::demo::worn::item_id()).unwrap();
        let child = crate::build::family(&world, root).into_iter().find(|i| *i != root).unwrap();
        let identity = Identity::capture(&world, child).unwrap();
        let child_id = world.objects.get(child).unwrap().full_id;
        assert_eq!(identity.resolve(&world), Some(child_id));

        let aurora_net::NetEvent::ObjectUpdates { handle, objects } = crate::demo::worn::events().remove(0) else {
            panic!("objects")
        };
        world.objects.remove_idx(child);
        assert_eq!(identity.resolve(&world), None);
        let mut duplicate = objects[1].clone();
        duplicate.local_id += 10;
        duplicate.full_id = Uuid::from_u128(0xabcdef10);
        world.apply(aurora_net::NetEvent::ObjectUpdates {
            handle,
            objects: vec![objects[1].clone(), duplicate],
        });
        let mut ambiguous = identity.clone();
        ambiguous.parts = 3;
        assert_eq!(ambiguous.resolve(&world), None);

        world.agent_id = Uuid::from_u128(0xabcdef11);
        assert_eq!(Identity::capture(&world, root), None);
        assert_eq!(identity.resolve(&world), None);
    }
}
