//! Particle sources attached to objects: simulation (aurora_prim::particles)
//! and per-frame instance lists for the renderer.

use super::Scene;
use super::textures::{TexSource, TextureStreamer};
use crate::frame_profile::{Part, Parts};
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

impl Source {
    fn update(&mut self, dt: f32, ctx: &SourceContext, budget: &mut usize, world: &World) {
        self.ps.update_with_wind(dt, ctx, budget, |position| wind_at(world, position));
    }
}

/// LLViewerPartGroup::getRegion / updateParticles: use the region containing
/// the particle, even after it drifts away from its source or the source dies.
/// LLWind::getVelocity's interpolation and factor of two are in WindGrid.
fn wind_at(world: &World, position: Vec3) -> Vec3 {
    for region in world.regions.values() {
        let Some(offset) = world.region_offset(region.handle) else {
            continue;
        };
        let local = position - offset;
        if local.x >= 0.0 && local.y >= 0.0 && local.x < region.heightmap.size_x as f32 && local.y < region.heightmap.size_y as f32 {
            return region.wind.velocity(local, region.heightmap.size_x as f32);
        }
    }
    // LLViewerPartGroup falls back to the agent's region when no region
    // contains the group. WindGrid keeps its clamped/wrapped coordinates.
    world
        .main_region
        .and_then(|handle| world.regions.get(&handle))
        .map_or(Vec3::ZERO, |region| region.wind.velocity(position, region.heightmap.size_x as f32))
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
    /// The frame's instances in source order, and their back to front
    /// order (reused).
    inst_tmp: Vec<ParticleInstance>,
    order_tmp: Vec<u64>,
    /// Objects whose update carried a particle system change (reused).
    changed_tmp: Vec<usize>,
    pub live: usize,
}

fn to_u8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Sort key of an instance: ascending keys go from the farthest to the
/// nearest (`distance_sq` to the eye), then by `index` (source order) so
/// that two particles at the same distance keep their order from a frame to
/// the next. The bits of a positive float order like the float.
fn far_first_key(distance_sq: f32, index: usize) -> u64 {
    let d = if distance_sq.is_nan() { 0.0 } else { distance_sq.max(0.0) };
    ((!d.to_bits() as u64) << 32) | index as u64
}

impl ParticleManager {
    /// LLViewerObject::isParticleSource excludes expired emitters even
    /// while their last particles are fading and their PSBlock is retained.
    pub fn beacon_source(&self, id: &Uuid) -> bool {
        self.sources.get(id).is_some_and(|s| !s.cloud && s.ps.is_emitting())
    }

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

    /// The clouds of `avatars` go at once, their particles too (the rez
    /// effect draws those avatars instead).
    pub fn drop_clouds(&mut self, textures: &mut TextureStreamer, avatars: &[Uuid]) {
        for id in avatars {
            if self.sources.get(id).is_some_and(|s| s.cloud)
                && let Some(s) = self.sources.remove(id)
            {
                textures.release(&s.texture);
            }
        }
    }

    /// Particle system of an object (render cost).
    pub fn source_data(&self, id: &Uuid) -> Option<&aurora_prim::particles::PartSysData> {
        self.sources.get(id).map(|s| s.ps.data())
    }

    /// The particle a right click lands on (LLPipeline::
    /// lineSegmentIntersectParticle, LLVOPartGroup::lineSegmentIntersect):
    /// the source object of the nearest live particle the ray `origin` +
    /// t·`dir` crosses for t in 0..`max`, each particle a quad of its size
    /// facing the ray. Loading clouds and HUD particles are not picked.
    pub fn pick(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<Uuid> {
        let dir = dir.try_normalize()?;
        let right = dir.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(dir);
        let mut best: Option<(f32, Uuid)> = None;
        for (id, s) in &self.sources {
            if s.cloud {
                continue;
            }
            for p in s.ps.particles() {
                if p.flags & ps::LL_PART_HUD != 0 {
                    continue;
                }
                let Some(t) = quad_hit(origin, dir, right, up, p.pos, p.size) else {
                    continue;
                };
                if t <= max && best.is_none_or(|(b, _)| t < b) {
                    best = Some((t, *id));
                }
            }
        }
        best.map(|(_, id)| id)
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
        parts: &mut Parts,
    ) {
        let t = Instant::now();
        out.clear();
        // ---- particle system changes carried by object updates: the store
        // lists those objects (no walk over every object each frame), taken
        // in slot order so that the sources keep the seeds a walk gave them
        let mut changed = std::mem::take(&mut self.changed_tmp);
        world.objects.take_particle_changes(&mut changed);
        changed.sort_unstable();
        for &idx in &changed {
            let Some(o) = world.objects.get_mut_untracked(idx) else {
                continue;
            };
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
        changed.clear();
        self.changed_tmp = changed;
        let t = parts.lap(Part::ParamsPartSources, t);
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
                s.update(dt, &SourceContext::default(), &mut none, world);
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
                .is_some_and(|o| world.particles_blocked(&o.owner_id))
                || idx.is_some_and(|idx| !world.parcel_object_visible(idx) || !crate::snapshot::isolate::object_visible(world, idx));
            if blocked {
                // what they already sent goes at once: the Legacy Viewer
                // lets it live out its age, which a block from the menu
                // should not wait for
                s.ps.clear_particles();
            }
            if blocked || pos.distance(eye) > draw_distance {
                let mut none = 0;
                s.update(dt, &ctx, &mut none, world);
            } else {
                s.update(dt, &ctx, &mut budget, world);
            }
            // texture changes
            let tex = world.rlva.camera_texture().unwrap_or_else(|| s.ps.texture());
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
        let t = parts.lap(Part::ParamsPartSim, t);

        // ---- instances, back to front
        self.inst_tmp.clear();
        self.order_tmp.clear();
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
                self.order_tmp.push(far_first_key(pos.distance_squared(eye), self.inst_tmp.len()));
                self.inst_tmp.push(inst);
            }
        }
        self.live = self.inst_tmp.len();
        // the order is sorted as plain integers (8 bytes each, not the
        // 48-byte instances), then the instances are gathered
        self.order_tmp.sort_unstable();
        out.extend(self.order_tmp.iter().map(|k| self.inst_tmp[(k & 0xFFFF_FFFF) as usize]));
        parts.lap(Part::ParamsPartList, t);
    }
}

/// Distance along the ray (`dir` unit, perpendicular to the quad) to a
/// particle quad centred on `pos`, `size` wide along `right` and `up`.
fn quad_hit(origin: Vec3, dir: Vec3, right: Vec3, up: Vec3, pos: Vec3, size: glam::Vec2) -> Option<f32> {
    let t = (pos - origin).dot(dir);
    if t < 0.0 {
        return None;
    }
    let local = origin + dir * t - pos;
    (local.dot(right).abs() <= size.x * 0.5 && local.dot(up).abs() <= size.y * 0.5).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_beacons_exclude_clouds_and_expired_sources_with_live_particles() {
        let id = Uuid::from_u128(682);
        let mut manager = ParticleManager::default();
        manager.sources.insert(
            id,
            Source {
                ps: ParticleSource::new(
                    ps::PartSysData {
                        pattern: ps::LL_PART_SRC_PATTERN_DROP,
                        burst_part_count: 1,
                        burst_rate: 100.0,
                        ..Default::default()
                    },
                    1,
                ),
                texture: Uuid::nil(),
                slot: 0,
                cloud: false,
            },
        );
        let mut budget = 1;
        manager
            .sources
            .get_mut(&id)
            .unwrap()
            .ps
            .update_with_wind(0.0, &SourceContext::default(), &mut budget, |_| Vec3::ZERO);
        assert!(manager.beacon_source(&id));
        manager.sources.get_mut(&id).unwrap().cloud = true;
        assert!(!manager.beacon_source(&id));
        let source = manager.sources.get_mut(&id).unwrap();
        source.cloud = false;
        source.ps.stop();
        assert!(!source.ps.particles().is_empty(), "the last particles still fade");
        assert!(!manager.beacon_source(&id));
    }

    #[test]
    fn particle_wind_uses_live_region_grids_after_the_emitter_stops() {
        use aurora_net::{NetEvent, terrain::WindGrid};
        use std::sync::Arc;
        let mut world = World::new(Arc::new(super::super::avatar::AvatarLibrary::load()));
        let main = (1024u64 << 32) | 2048;
        let east = (1280u64 << 32) | 2048;
        world.main_region = Some(main);
        for handle in [main, east] {
            world.apply(NetEvent::Wind {
                handle,
                grid: Box::new(WindGrid {
                    velocities: std::array::from_fn(|i| {
                        if handle == main {
                            Vec3::X * (i % 16) as f32
                        } else {
                            Vec3::Y * -4.0
                        }
                    }),
                }),
            });
        }
        assert_eq!(wind_at(&world, Vec3::new(24.0, 8.0, 1000.0)), Vec3::X * 3.0);
        assert_eq!(wind_at(&world, Vec3::new(256.0, 8.0, 20.0)), Vec3::Y * -8.0);
        assert_eq!(wind_at(&world, Vec3::new(-1.0, 8.0, 20.0)), Vec3::ZERO);
        assert_eq!(wind_at(&world, Vec3::new(512.0, 8.0, 20.0)), Vec3::ZERO);
        assert_eq!(wind_at(&world, Vec3::new(520.0, 8.0, 20.0)), Vec3::X);

        let mut data = ps::PartSysData {
            pattern: ps::LL_PART_SRC_PATTERN_DROP,
            burst_part_count: 1,
            burst_rate: 100.0,
            ..Default::default()
        };
        data.part.flags = ps::LL_PART_WIND_MASK;
        let mut source = Source {
            ps: ParticleSource::new(data, 1),
            texture: Uuid::nil(),
            slot: 0,
            cloud: false,
        };
        let ctx = SourceContext {
            pos: Vec3::new(24.0, 8.0, 20.0),
            ..Default::default()
        };
        let mut budget = 1;
        source.update(0.0, &ctx, &mut budget, &world);
        source.ps.stop();
        source.update(0.1, &SourceContext::default(), &mut budget, &world);
        assert!((source.ps.particles()[0].vel.x - 0.03).abs() < 1e-6);
        world.apply(NetEvent::Wind {
            handle: main,
            grid: Box::new(WindGrid {
                velocities: [Vec3::Y * 5.0; 256],
            }),
        });
        source.update(0.1, &ctx, &mut budget, &world);
        let p = source.ps.particles()[0];
        assert!((p.vel.x - 0.0297).abs() < 1e-6);
        assert!((p.vel.y - 0.1).abs() < 1e-6);
        world.regions.clear();
        source.update(0.1, &ctx, &mut budget, &world);
        assert!((source.ps.particles()[0].vel - p.vel * 0.99).length() < 1e-6);
    }

    #[test]
    fn a_ray_picks_a_particle_quad_in_front_only() {
        let (dir, right, up) = (Vec3::X, Vec3::NEG_Y, Vec3::Z);
        let size = glam::Vec2::new(1.0, 0.5);
        let at = Vec3::new(10.0, 0.0, 0.0);
        assert_eq!(quad_hit(Vec3::ZERO, dir, right, up, at, size), Some(10.0));
        // within the half sizes, sideways and up
        assert!(quad_hit(Vec3::new(0.0, 0.45, 0.2), dir, right, up, at, size).is_some());
        assert!(quad_hit(Vec3::new(0.0, 0.0, 0.3), dir, right, up, at, size).is_none());
        // behind the eye
        assert!(quad_hit(Vec3::new(11.0, 0.0, 0.0), dir, right, up, at, size).is_none());
    }

    #[test]
    fn instances_are_ordered_far_first_then_by_source_order() {
        // distances squared and the index each instance was built at
        let mut keys: Vec<u64> = [(4.0, 0), (100.0, 1), (0.0, 2), (4.0, 3), (f32::NAN, 4), (2.5e9, 5), (1e-30, 6)]
            .into_iter()
            .map(|(d, i)| far_first_key(d, i))
            .collect();
        keys.sort_unstable();
        let order: Vec<u64> = keys.iter().map(|k| k & 0xFFFF_FFFF).collect();
        // equal distances keep the order they were built in; a distance that
        // is not a number counts as none
        assert_eq!(order, [5, 1, 0, 3, 6, 2, 4]);
    }
}
