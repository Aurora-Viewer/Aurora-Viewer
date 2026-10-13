//! Figures of the "Inspect" window: what LLFloaterInspect::refresh
//! reads on each selected prim (Firestorm's "Improved Object Inspect",
//! indra/newview/llfloaterinspect.cpp, originally LGPL 2.1) — faces,
//! vertices and triangles as drawn now, its textures and their memory, and
//! the complexity of a linkset.

use super::Scene;
use super::complexity::{MAX_ATTACHMENT_COMPLEXITY, texture_cost};
use crate::world::World;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrimStats {
    /// LLViewerObject::getNumFaces, and those that are drawn.
    pub faces: u32,
    pub visible_faces: u32,
    /// At the detail level shown now (getNumVertices, getNumIndices / 3).
    pub vertices: u32,
    pub triangles: u32,
    /// Its textures, once each (getObjectTextureMemory): the faces', their
    /// materials' and the sculpt map.
    pub textures: Vec<Uuid>,
}

impl Scene {
    pub fn prim_stats(&self, world: &World, idx: usize) -> PrimStats {
        let Some(g) = self.gpu.get(idx) else {
            return PrimStats::default();
        };
        let geom = g.geom.and_then(|key| self.geom_ready(&key));
        let (vertices, indices) = geom.as_ref().map_or((0, 0), |geom| {
            geom.faces
                .iter()
                .flatten()
                .fold((0, 0), |(v, i), mesh| (v + mesh.vertex_count, i + mesh.index_count))
        });
        let mut textures = Vec::new();
        let mut add = |id: Uuid| {
            if !id.is_nil() && !textures.contains(&id) {
                textures.push(id);
            }
        };
        for face in &g.faces {
            add(face.tex_id);
            for id in face.aux_tex {
                add(id);
            }
        }
        if let Some(o) = world.objects.get(idx)
            && let Some(sculpt) = o.visual_volume().sculpt.filter(|_| !o.visual_volume().is_mesh())
        {
            add(sculpt.texture);
        }
        PrimStats {
            faces: geom.map_or(0, |geom| geom.faces.len()).max(g.built_faces) as u32,
            visible_faces: g.faces.len() as u32,
            vertices,
            triangles: indices / 3,
            textures,
        }
    }

    /// (TMem, VRAM) of a texture in bytes, as calculateTextureMemory
    /// estimates them from its full size: its own components, and RGBA on
    /// the GPU. Zero until its header is known.
    pub fn texture_memory(&self, id: &Uuid) -> (u64, u64) {
        let Some((w, h)) = self.textures.full_size(id) else {
            return (0, 0);
        };
        let pixels = u64::from(w) * u64::from(h);
        let components = if self.textures.has_alpha(id) == Some(true) { 4 } else { 3 };
        (pixels * components, pixels * 4)
    }

    /// Complexity of a linkset given by its prims, as one attachment counts
    /// in its wearer's (LLVOVolume::getRenderCost of each, then each
    /// texture once, capped).
    pub fn linkset_complexity(&self, world: &World, prims: &[usize]) -> u32 {
        let mut textures = HashSet::new();
        let mut total: f32 = prims.iter().map(|&i| self.prim_render_cost(world, i, &mut textures)).sum();
        for id in &textures {
            total += texture_cost(self.textures.full_size(id)) as f32;
        }
        total.clamp(0.0, MAX_ATTACHMENT_COMPLEXITY) as u32
    }
}
