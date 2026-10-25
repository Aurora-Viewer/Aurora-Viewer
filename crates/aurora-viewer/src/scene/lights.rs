//! Local lights of a frame: the prim lights around the camera, as
//! LLPipeline::renderDeferredLighting collects them (indra/newview/
//! pipeline.cpp, originally LGPL 2.1, Linden Research / Firestorm). A light
//! with a light texture is a projector (LLVOVolume::isLightSpotlight): it
//! lights the frustum that leaves its prim, through that texture. Two
//! projectors at a time cast shadows (LLPipeline::generateSunShadow,
//! mShadowSpotLight / mTargetShadowSpotLight).

use super::Scene;
use crate::world::World;
use aurora_render::{PointLight, Projector, Renderer, SpotShadow};
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

/// The camera of a frame, for its lights.
#[derive(Debug, Clone, Copy)]
pub struct LightView {
    pub eye: Vec3,
    pub forward: Vec3,
    /// Seconds since the previous frame.
    pub dt: f32,
    /// Projectors cast shadows (the setting, with the sun's shadows on).
    pub projector_shadows: bool,
}

/// The light list's working buffers, kept between frames for their room.
#[derive(Default)]
pub struct Scratch {
    /// (distance, object index, light)
    lights: Vec<(f32, usize, PointLight)>,
    /// Objects whose projector is in the list.
    projecting: Vec<usize>,
    /// `Scene::light_objects`, copied: the loop borrows the scene.
    objects: Vec<usize>,
}

/// The two projectors that cast shadows and how far each has faded in:
/// LLPipeline::generateSunShadow's update of mShadowSpotLight. A projector
/// keeps its shadow map while it is one of the two `targets`; otherwise its
/// shadows fade out, then the map goes to the target that has none.
#[derive(Debug, Default)]
pub struct SpotShadowSlots {
    /// Object index of the projector of each shadow map.
    pub lights: [Option<usize>; 2],
    pub fade: [f32; 2],
    last_eye: Option<Vec3>,
}

impl SpotShadowSlots {
    /// `targets`: the two projectors that weigh most on screen this frame.
    /// The fade lasts a second, less as the camera moves faster.
    pub fn update(&mut self, targets: [Option<usize>; 2], eye: Vec3, dt: f32) {
        let speed = self.last_eye.map_or(0.0, |e| e.distance(eye) / dt.max(1e-4));
        self.last_eye = Some(eye);
        let amount = dt * speed.max(1.0);
        for i in 0..2 {
            if self.lights[i].is_some() && targets.contains(&self.lights[i]) {
                self.fade[i] = (self.fade[i] + amount).min(1.0);
            } else {
                self.fade[i] = (self.fade[i] - amount).max(0.0);
                if self.fade[i] == 0.0 || self.lights[i].is_none() {
                    // faded out: take the pending target the other map
                    // does not already have
                    self.lights[i] = if targets[0] != self.lights[(i + 1) % 2] {
                        targets[0]
                    } else {
                        targets[1]
                    };
                }
            }
        }
    }

    fn clear(&mut self) {
        *self = SpotShadowSlots::default();
    }
}

/// LLVOVolume::updateSpotLightPriority: apparent size of a sphere of the
/// light's radius, one radius down its beam and one radius towards the
/// camera (LLPipeline::calcPixelArea's angle; only the order matters).
fn spot_priority(position: Vec3, direction: Vec3, radius: f32, view: &LightView) -> f32 {
    let center = position + direction * radius - view.forward * radius;
    let mut dist = center.distance(view.eye);
    if dist < 16.0 {
        // calcPixelArea ramps the distance down for nearby objects
        dist = (dist / 16.0).powi(2) * 16.0;
    }
    (radius * 3f32.sqrt() / dist.max(1e-4)).atan()
}

/// Worn by an avatar: a parent up its link is one (the attachment's root,
/// or the prim itself; LLViewerObject::isAttachment).
fn worn(world: &World, o: &crate::world::objects::Object) -> bool {
    let mut current = o;
    for _ in 0..4 {
        let Some(parent) = world.objects.parent_of(current).and_then(|i| world.objects.get(i)) else {
            return false;
        };
        if parent.is_avatar() {
            return true;
        }
        current = parent;
    }
    false
}

impl Scene {
    /// The lights nearest to the eye, nearest first, and those of them
    /// that cast shadows.
    pub fn local_lights(&mut self, world: &World, renderer: &mut Renderer, view: &LightView) -> (Vec<PointLight>, [Option<SpotShadow>; 2]) {
        let eye = view.eye;
        let now = Instant::now();
        let Scratch {
            mut lights,
            mut projecting,
            mut objects,
        } = std::mem::take(&mut self.light_scratch);
        objects.extend(self.light_objects.iter().copied());
        for idx in objects.drain(..) {
            if !world.parcel_object_visible(idx) {
                continue;
            }
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let Some(l) = o.extra.light else {
                continue;
            };
            let Some((center, hud)) = self.gpu.get(idx).map(|g| (g.center, g.hud)) else {
                continue;
            };
            if hud || (!self.attached_lights && worn(world, o)) {
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
                idx,
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
        // (unstable: a stable sort allocates a buffer as long as the list;
        // equal distances stay in index order, the order they came in)
        lights.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        lights.truncate(MAX_LIGHTS);
        // the two projectors of the list that weigh most on screen, among
        // those that reach into the half-space in front of the camera
        if view.projector_shadows {
            let mut ranked: Vec<(f32, usize)> = lights
                .iter()
                .filter_map(|(_, idx, l)| {
                    let p = l.projector?;
                    let ahead = view.forward.dot(l.position - eye) > -l.radius;
                    ahead.then(|| (spot_priority(l.position, p.direction, l.radius / 1.5, view), *idx))
                })
                .collect();
            ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
            let targets = [ranked.first().map(|r| r.1), ranked.get(1).map(|r| r.1)];
            self.spot_shadows.update(targets, eye, view.dt);
        } else {
            self.spot_shadows.clear();
        }
        let slots = &self.spot_shadows;
        let shadows = std::array::from_fn(|k| {
            let idx = slots.lights[k]?;
            let light = lights.iter().position(|(_, i, l)| *i == idx && l.projector.is_some())?;
            Some(SpotShadow {
                light: light as u32,
                fade: slots.fade[k],
            })
        });
        let list = lights.iter().map(|(_, _, l)| *l).collect();
        lights.clear();
        projecting.clear();
        self.light_scratch = Scratch {
            lights,
            projecting,
            objects,
        };
        (list, shadows)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_projector_keeps_its_shadow_map_while_it_is_a_target() {
        let mut slots = SpotShadowSlots::default();
        let eye = Vec3::ZERO;
        // two targets take the two maps, then fade in over a second
        slots.update([Some(7), Some(9)], eye, 0.1);
        assert_eq!(slots.lights, [Some(7), Some(9)]);
        assert_eq!(slots.fade, [0.0, 0.0]);
        for _ in 0..5 {
            slots.update([Some(9), Some(7)], eye, 0.1);
        }
        // (whatever their order among the targets)
        assert_eq!(slots.lights, [Some(7), Some(9)]);
        assert!((slots.fade[0] - 0.5).abs() < 1e-5 && (slots.fade[1] - 0.5).abs() < 1e-5);
        for _ in 0..10 {
            slots.update([Some(7), Some(9)], eye, 0.1);
        }
        assert_eq!(slots.fade, [1.0, 1.0]);
    }

    #[test]
    fn a_new_target_waits_for_the_fade_out_of_the_one_it_replaces() {
        let mut slots = SpotShadowSlots::default();
        let eye = Vec3::ZERO;
        for _ in 0..12 {
            slots.update([Some(1), Some(2)], eye, 0.1);
        }
        // 3 outranks 2: 1 keeps its map, 2 fades out before 3 takes its
        slots.update([Some(1), Some(3)], eye, 0.25);
        assert_eq!(slots.lights, [Some(1), Some(2)]);
        assert_eq!(slots.fade, [1.0, 0.75]);
        for _ in 0..3 {
            slots.update([Some(1), Some(3)], eye, 0.25);
        }
        assert_eq!(slots.lights, [Some(1), Some(3)]);
        assert_eq!(slots.fade, [1.0, 0.0]);
        slots.update([Some(1), Some(3)], eye, 0.25);
        assert_eq!(slots.fade, [1.0, 0.25]);
        // a moving camera shortens the fade (2 m in 0.25 s: 8 m/s)
        slots.update([Some(1), Some(3)], Vec3::new(2.0, 0.0, 0.0), 0.25);
        assert_eq!(slots.fade, [1.0, 1.0]);
        // a single target: the other map is given up
        for _ in 0..5 {
            slots.update([Some(1), None], Vec3::new(2.0, 0.0, 0.0), 0.25);
        }
        assert_eq!(slots.lights, [Some(1), None]);
    }

    #[test]
    fn the_projector_that_weighs_most_on_screen_comes_first() {
        let view = LightView {
            eye: Vec3::ZERO,
            forward: Vec3::X,
            dt: 0.016,
            projector_shadows: true,
        };
        let down = Vec3::NEG_Z;
        let near = spot_priority(Vec3::new(10.0, 0.0, 3.0), down, 10.0, &view);
        let far = spot_priority(Vec3::new(40.0, 0.0, 3.0), down, 10.0, &view);
        let small = spot_priority(Vec3::new(10.0, 0.0, 3.0), down, 2.0, &view);
        assert!(near > far && near > small, "{near} {far} {small}");
    }
}
