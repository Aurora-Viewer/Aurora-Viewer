//! Whether a mesh has a physics shape of its own, for the "Prim" entry of
//! the Features tab's physics shape list (LLPanelVolume::getState,
//! indra/newview/llpanelvolume.cpp: `gMeshRepo.hasPhysicsShape`, originally
//! LGPL 2.1). The rule itself is `aurora_assets::mesh_physics::has_physics_shape`;
//! this keeps the answers and the collision blocks asked for to find one.
//!
//! The tab asks while it shows a mesh whose answer is unknown; once per frame,
//! the app turns those questions into requests to the mesh streamer and
//! releases what nobody asked for that frame (the tab left, another prim or
//! nothing selected, an answer that arrived), the way the physics shape
//! display drops the blocks it no longer draws (`scene/physics_shapes.rs`).

use std::collections::HashMap;
use uuid::Uuid;

#[derive(Default)]
pub struct MeshPhysics {
    /// Answers already known: a mesh asset never changes.
    known: HashMap<Uuid, bool>,
    /// The mesh whose collision blocks are requested.
    fetching: Option<Uuid>,
    /// The mesh the tab asked about this frame, answer still unknown.
    asked: Option<Uuid>,
}

impl MeshPhysics {
    /// The answer for `id`, `lookup` reading what the streamer holds of it;
    /// `None` while unknown, which asks for its blocks this frame. A nil id
    /// has no shape and fetches nothing (LLMeshRepository::hasPhysicsShape).
    pub fn ask(&mut self, id: Uuid, lookup: impl FnOnce() -> Option<bool>) -> Option<bool> {
        if id.is_nil() {
            return Some(false);
        }
        if let Some(&known) = self.known.get(&id) {
            return Some(known);
        }
        let known = lookup();
        match known {
            Some(known) => {
                self.known.insert(id, known);
            }
            None => self.asked = Some(id),
        }
        known
    }

    /// Once per frame: the mesh whose blocks stay requested, and the one to
    /// release. The request is renewed every frame, since the physics shape
    /// display may release the same mesh when it stops drawing it.
    pub fn end_frame(&mut self) -> (Option<Uuid>, Option<Uuid>) {
        let asked = self.asked.take();
        let release = self.fetching.filter(|f| Some(*f) != asked);
        self.fetching = asked;
        (asked, release)
    }

    /// Asks the streamer for this frame's blocks and releases the others.
    pub fn apply(&mut self, meshes: &mut crate::scene::meshes::MeshStreamer) {
        let (want, release) = self.end_frame();
        if let Some(id) = release {
            meshes.want_physics(id, false);
        }
        if let Some(id) = want {
            meshes.want_physics(id, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(0xa);
    const B: Uuid = Uuid::from_u128(0xb);

    #[test]
    fn an_unknown_mesh_is_requested_while_asked_then_released() {
        let mut m = MeshPhysics::default();
        assert_eq!(m.ask(A, || None), None);
        assert_eq!(m.end_frame(), (Some(A), None), "requested");
        assert_eq!(m.ask(A, || None), None);
        assert_eq!(m.end_frame(), (Some(A), None), "renewed while still unknown");
        // the tab left, or a plain prim or nothing selected: nobody asks
        assert_eq!(m.end_frame(), (None, Some(A)), "released");
        assert_eq!(m.end_frame(), (None, None));
    }

    #[test]
    fn an_answer_is_kept_and_releases_the_request() {
        let mut m = MeshPhysics::default();
        assert_eq!(m.ask(A, || None), None);
        m.end_frame();
        assert_eq!(m.ask(A, || Some(true)), Some(true));
        assert_eq!(m.end_frame(), (None, Some(A)), "known: released");
        assert_eq!(m.ask(A, || unreachable!("kept")), Some(true));
        assert_eq!(m.end_frame(), (None, None));
    }

    #[test]
    fn switching_to_a_known_mesh_releases_the_pending_one() {
        let mut m = MeshPhysics::default();
        assert_eq!(m.ask(B, || Some(false)), Some(false));
        assert_eq!(m.ask(A, || None), None);
        m.end_frame();
        assert_eq!(m.ask(B, || unreachable!("kept")), Some(false));
        assert_eq!(m.end_frame(), (None, Some(A)));
    }

    #[test]
    fn switching_to_another_unknown_mesh_moves_the_request() {
        let mut m = MeshPhysics::default();
        m.ask(A, || None);
        m.end_frame();
        m.ask(B, || None);
        assert_eq!(m.end_frame(), (Some(B), Some(A)));
    }

    #[test]
    fn a_nil_mesh_id_has_no_shape_and_fetches_nothing() {
        let mut m = MeshPhysics::default();
        assert_eq!(m.ask(Uuid::nil(), || unreachable!("not looked up")), Some(false));
        assert_eq!(m.end_frame(), (None, None));
    }
}
