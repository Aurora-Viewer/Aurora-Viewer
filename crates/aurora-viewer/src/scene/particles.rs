//! Particle sources attached to objects: simulation (aurora_prim::particles)
//! and per-frame instance lists for the renderer.

use super::Scene;
use super::textures::{TexSource, TextureStreamer};
use crate::world::World;
use aurora_net::objects::ParticleUpdate;
use aurora_prim::particles::{self as ps, ParticleSource, SourceContext};
use aurora_render::{ParticleInstance, Renderer, particle_flags};
use glam::Vec3;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

struct Source {
    ps: ParticleSource,
    texture: Uuid,
    slot: u32,
    /// Loading cloud of an avatar (not an object's particle system).
    cloud: bool,
}

/// Loading cloud: Firestorm's app_settings/cloud.xml motion, in Aurora's
/// colors (violet to turquoise), on the default soft particle.
fn cloud_data(avatar: Uuid) -> ps::PartSysData {
    let mut d = ps::PartSysData {
        pattern: ps::LL_PART_SRC_PATTERN_ANGLE_CONE,
        max_age: 0.0,
        inner_angle: std::f32::consts::PI,
        outer_angle: 0.0,
        burst_rate: 0.02,
        burst_part_count: 1,
        burst_radius: 0.3,
        burst_speed_min: 0.1,
        burst_speed_max: 1.0,
        target_id: avatar,
        ..Default::default()
    };
    d.part.max_age = 4.0;
    d.part.start_color = [0.545, 0.361, 0.965, 0.1];
    d.part.end_color = [0.369, 0.918, 0.831, 0.9];
    d.part.start_scale = glam::Vec2::splat(0.8);
    d.part.end_scale = glam::Vec2::splat(0.02);
    d.part.flags = ps::LL_PART_EMISSIVE_MASK | ps::LL_PART_INTERP_COLOR_MASK | ps::LL_PART_INTERP_SCALE_MASK | ps::LL_PART_TARGET_POS_MASK;
    d
}

#[derive(Default)]
pub struct ParticleManager {
    sources: HashMap<Uuid, Source>,
    seed: u64,
    sort_tmp: Vec<(f32, ParticleInstance)>,
    pub live: usize,
}

fn to_u8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

impl ParticleManager {
    /// Clouds around the avatars still loading; the others stop emitting.
    pub fn sync_clouds(&mut self, avatars: &[Uuid]) {
        for (id, s) in self.sources.iter_mut() {
            if s.cloud && !avatars.contains(id) {
                s.ps.stop();
            }
        }
        for id in avatars {
            if self.sources.contains_key(id) {
                continue;
            }
            self.seed = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            self.sources.insert(
                *id,
                Source {
                    ps: ParticleSource::new(cloud_data(*id), self.seed ^ id.as_u128() as u64),
                    texture: Uuid::nil(),
                    slot: 0,
                    cloud: true,
                },
            );
        }
    }

    /// Particle system of an object (render cost).
    pub fn source_data(&self, id: &Uuid) -> Option<&aurora_prim::particles::PartSysData> {
        self.sources.get(id).map(|s| s.ps.data())
    }

    /// Release every source (region change, logout).
    pub fn clear(&mut self, textures: &mut TextureStreamer) {
        for (_, s) in self.sources.drain() {
            textures.release(&s.texture);
        }
        self.live = 0;
    }

    /// Apply object updates, simulate and build the sorted instance list.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        textures: &mut TextureStreamer,
        world: &mut World,
        dt: f32,
        eye: Vec3,
        max_particles: usize,
        draw_distance: f32,
        out: &mut Vec<ParticleInstance>,
    ) {
        out.clear();
        // ---- particle system changes carried by object updates
        for o in world.objects.slots.iter_mut().flatten() {
            let Some(update) = o.particle_update.take() else {
                continue;
            };
            let data = match update {
                ParticleUpdate::Set(bytes) => ps::parse(&bytes),
                _ => None,
            };
            match (data, self.sources.get_mut(&o.full_id)) {
                (Some(d), Some(s)) => s.ps.set_data(d),
                (Some(d), None) => {
                    self.seed = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
                    let texture = d.part_image_id;
                    let slot = textures.acquire(renderer, texture, TexSource::Asset);
                    self.sources.insert(
                        o.full_id,
                        Source {
                            ps: ParticleSource::new(d, self.seed ^ o.full_id.as_u128() as u64),
                            texture,
                            slot,
                            cloud: false,
                        },
                    );
                }
                (None, Some(s)) => s.ps.stop(),
                (None, None) => {}
            }
        }
        if max_particles == 0 {
            self.clear(textures);
            return;
        }

        // ---- simulate
        let now = Instant::now();
        let dt = dt.clamp(0.0, ps::MAX_PARTICLE_DT);
        let live: usize = self.sources.values().map(|s| s.ps.particles().len()).sum();
        let mut budget = max_particles.saturating_sub(live);
        let mut dead = Vec::new();
        for (id, s) in self.sources.iter_mut() {
            let idx = world.objects.index_of_uuid(id);
            let transform = idx.and_then(|i| Scene::object_transform(world, i, now, 0));
            let Some((pos, rot, hud)) = transform else {
                // object gone: stop emitting, let the particles live out
                s.ps.stop();
                let mut none = 0;
                s.ps.update(dt, &SourceContext::default(), &mut none);
                if s.ps.is_dead() {
                    dead.push(*id);
                }
                continue;
            };
            if hud {
                continue;
            }
            let target = {
                let t = s.ps.data().target_id;
                if t.is_nil() {
                    None
                } else {
                    world
                        .objects
                        .index_of_uuid(&t)
                        .and_then(|ti| Scene::object_transform(world, ti, now, 0))
                        .map(|(p, _, _)| p)
                }
            };
            let ctx = SourceContext {
                pos,
                rot,
                vel: Vec3::ZERO,
                target,
                wind: Vec3::ZERO,
            };
            // sources beyond the draw distance, or of a blocked owner
            // (LLViewerPartSourceScript, flagParticles), keep simulating but do not emit
            let blocked = idx
                .and_then(|i| world.objects.get(i))
                .is_some_and(|o| world.particles_blocked(&o.owner_id));
            if blocked || pos.distance(eye) > draw_distance {
                let mut none = 0;
                s.ps.update(dt, &ctx, &mut none);
            } else {
                s.ps.update(dt, &ctx, &mut budget);
            }
            // texture changes
            let tex = s.ps.texture();
            if tex != s.texture {
                textures.release(&s.texture);
                s.slot = textures.acquire(renderer, tex, TexSource::Asset);
                s.texture = tex;
            }
            if s.ps.is_dead() {
                dead.push(*id);
            }
        }
        for id in dead {
            if let Some(s) = self.sources.remove(&id) {
                textures.release(&s.texture);
            }
        }

        // ---- instances, back to front
        self.sort_tmp.clear();
        for s in self.sources.values() {
            let slot = if s.texture.is_nil() || !textures.is_loaded(&s.texture) {
                0
            } else {
                s.slot
            };
            textures.note_usage(&s.texture, 128.0);
            for p in s.ps.particles() {
                if p.flags & ps::LL_PART_HUD != 0 {
                    continue;
                }
                let mut flags = 0;
                if p.is_emissive() {
                    flags |= particle_flags::EMISSIVE;
                }
                if p.blend_dst == ps::LL_PART_BF_ONE {
                    flags |= particle_flags::ADDITIVE;
                }
                let mut pos = p.pos;
                let mut axis = Vec3::ZERO;
                let mut size = [p.size.x, p.size.y];
                if p.is_ribbon() {
                    // segment towards the parent particle
                    let seg = p.prev_pos - p.pos;
                    if seg.length_squared() > 1e-8 {
                        pos = p.pos + seg * 0.5;
                        axis = seg;
                        size = [p.size.x, seg.length()];
                        flags |= particle_flags::AXIS;
                    }
                } else if p.flags & ps::LL_PART_FOLLOW_VELOCITY_MASK != 0 && p.vel.length_squared() > 1e-8 {
                    axis = p.vel.normalize() * p.size.y;
                    flags |= particle_flags::FOLLOW_VELOCITY;
                }
                let inst = ParticleInstance {
                    pos: pos.to_array(),
                    texture: slot,
                    size,
                    flags,
                    color: [to_u8(p.color[0]), to_u8(p.color[1]), to_u8(p.color[2]), to_u8(p.color[3])],
                    axis: axis.to_array(),
                    glow: p.glow,
                };
                self.sort_tmp.push((pos.distance_squared(eye), inst));
            }
        }
        self.live = self.sort_tmp.len();
        self.sort_tmp.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        out.extend(self.sort_tmp.iter().map(|(_, i)| *i));
    }
}
