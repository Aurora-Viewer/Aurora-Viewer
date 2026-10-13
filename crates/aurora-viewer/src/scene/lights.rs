//! Local lights of a frame: the prim lights around the camera, as
//! LLPipeline::renderDeferredLighting collects them (indra/newview/
//! pipeline.cpp, originally LGPL 2.1, Linden Research / Firestorm). A light
//! with a light texture is a projector (LLVOVolume::isLightSpotlight): it
//! lights the frustum that leaves its prim, through that texture.

use super::Scene;
use crate::world::World;
use aurora_render::{PointLight, Projector, Renderer};
use glam::Vec3;
use std::time::Instant;
use uuid::Uuid;

/// Lights further than this from the camera (plus their reach) are left out.
const LIGHT_RANGE: f32 = 96.0;
/// Size of the frame's light list (`MAX_LIGHTS` of the shaders).
const MAX_LIGHTS: usize = 64;
/// Pixels asked of a projected texture: LL keeps it at full size while its
/// light is drawn; the beam of a light spreads it over whole walls.
const PROJECTED_PIXELS: f32 = 512.0;

impl Scene {
    /// The lights nearest to `eye`, nearest first.
    pub fn local_lights(&mut self, world: &World, renderer: &mut Renderer, eye: Vec3) -> Vec<PointLight> {
        let now = Instant::now();
        let mut lights: Vec<(f32, PointLight)> = Vec::new();
        let mut projecting: Vec<usize> = Vec::new();
        let objects: Vec<usize> = self.light_objects.iter().copied().collect();
        for idx in objects {
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let Some(l) = o.extra.light else {
                continue;
            };
            let Some((center, hud)) = self.gpu.get(idx).map(|g| (g.center, g.hud)) else {
                continue;
            };
            if hud {
                continue;
            }
            let d = center.distance(eye);
            if d > LIGHT_RANGE + l.radius * 1.5 {
                continue;
            }
            // deferred SL lights reach 1.5x their radius with half the
            // falloff (pipeline.cpp: getLightRadius() * 1.5,
            // getLightFalloff(DEFERRED_LIGHT_FALLOFF = 0.5))
            let radius = (l.radius * 1.5).clamp(0.1, 30.0);
            let mut position = center;
            let mut projector = None;
            if let Some(image) = o.extra.light_image.filter(|i| !i.texture.is_nil())
                && let Some((pos, rot, _)) = Self::object_transform(world, idx, now, 0)
            {
                // the frustum leaves the prim itself, not its drawn bounds
                position = pos;
                let slot = self.projected_texture(renderer, idx, image.texture);
                projecting.push(idx);
                projector = Some(Projector::new(pos, rot, o.visual_scale(), radius, image.params, slot));
            }
            // the packed light color is already linear (LLLightParams::unpack ->
            // setLinearColor; pipeline.cpp sends getLightLinearColor() as is),
            // times 3.25 for the PBR punctual intensity (multiPointLightF)
            let color = Vec3::new(l.color[0], l.color[1], l.color[2]) * l.color[3] * 3.25;
            lights.push((
                d,
                PointLight {
                    position,
                    radius,
                    color,
                    falloff: l.falloff * 0.5,
                    projector,
                },
            ));
        }
        // textures of the projectors that left the list
        let textures = &mut self.textures;
        self.light_textures.retain(|idx, (id, _)| {
            let kept = projecting.contains(idx);
            if !kept {
                textures.release(id);
            }
            kept
        });
        lights.sort_by(|a, b| a.0.total_cmp(&b.0));
        lights.into_iter().take(MAX_LIGHTS).map(|(_, l)| l).collect()
    }

    /// Texture table slot of the texture object `idx` projects, held while
    /// the object stays in the light list.
    fn projected_texture(&mut self, renderer: &mut Renderer, idx: usize, id: Uuid) -> u32 {
        let slot = match self.light_textures.get(&idx) {
            Some((held, slot)) if *held == id => *slot,
            _ => {
                let slot = self.textures.acquire(renderer, id, super::textures::TexSource::Asset);
                if let Some((old, _)) = self.light_textures.insert(idx, (id, slot)) {
                    self.textures.release(&old);
                }
                slot
            }
        };
        self.textures.note_usage(&id, PROJECTED_PIXELS);
        slot
    }
}
