//! Eager retirement for frequently replaced live-preview assets. Ordinary
//! scene geometry keeps its normal reuse interval; a stream must not retain
//! ten seconds of obsolete meshes or release buffers still drawn by an object.
use super::*;

impl Scene {
    pub fn retire_preview_mesh(&mut self, id: Uuid) {
        self.retired_preview_meshes.insert(id);
    }

    pub(super) fn collect_preview_meshes(&mut self, renderer: &mut Renderer) {
        let ready: Vec<_> = self
            .retired_preview_meshes
            .iter()
            .copied()
            .filter(|id| {
                (0..4).all(|lod| {
                    self.geoms
                        .get(&GeomKey::Mesh { id: *id, lod })
                        .is_none_or(|entry| entry.refs == 0 && !matches!(entry.state, GeomState::Pending))
                })
            })
            .take(64)
            .collect();
        for id in ready {
            for lod in 0..4 {
                let key = GeomKey::Mesh { id, lod };
                if let Some(GeomEntry {
                    state: GeomState::Ready(geometry),
                    ..
                }) = self.geoms.remove(&key)
                {
                    for face in geometry.faces.iter().flatten() {
                        renderer.free_mesh(*face);
                    }
                    self.joint_boxes.release(&key);
                    self.geom_trash.push(geometry);
                }
            }
            self.skin_binds.release(&id);
            self.meshes.retire_local(&id, &self.jobs);
            self.retired_preview_meshes.remove(&id);
        }
    }
}
