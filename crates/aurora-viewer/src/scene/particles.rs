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
    owner: Uuid,
    object: Uuid,
    /// The object (or avatar) whose place it follows: its own key, except for
    /// the particle editor's preview (`ParticleManager::set_preview`).
    emitter: Uuid,
}

/// Key of the particle editor's preview source of `object`, beside the
/// object's own source (ParticleEditor::updateParticles adds a separate
/// LLViewerPartSourceScript to the prim).
fn preview_key(object: Uuid) -> Uuid {
    Uuid::from_u128(object.as_u128() ^ 0x5041_5254_4943_4c45_5f50_5245_5649_4557)
}

/// Captured at the press, before asynchronous object-depth readback. Retain
/// identity even if the particle moves or its emitter leaves the object store.
#[derive(Clone, Copy, Debug)]
pub struct ParticleHit {
    pub source: Uuid,
    pub object: Uuid,
    pub owner: Uuid,
    pub point: Vec3,
    pub distance: f32,
}

impl ParticleHit {
    pub fn before_surface(&self, ray: (Vec3, Vec3), surface: Option<Vec3>) -> bool {
        surface.is_none_or(|point| self.distance <= (point - ray.0).dot(ray.1) + 0.001)
    }
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
    property_owners: HashMap<Uuid, Uuid>,
    seed: u64,
    /// The frame's instances in source order, and their back to front
    /// order (reused).
    inst_tmp: Vec<ParticleInstance>,
    order_tmp: Vec<u64>,
    /// Objects whose update carried a particle system change (reused).
    changed_tmp: Vec<usize>,
    /// The prim the particle editor previews on: its own system is hidden
    /// meanwhile, so that what shows is what the editor sets.
    preview: Option<Uuid>,
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
    pub fn on_properties(&mut self, props: &[aurora_net::build::ObjectProps]) {
        for props in props {
            if !props.owner_id.is_nil() {
                self.property_owners.insert(props.object_id, props.owner_id);
            }
        }
    }
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
                    owner: Uuid::nil(),
                    object: *id,
                    emitter: *id,
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

    pub fn emitter(&self, object: Uuid) -> Option<Uuid> {
        self.sources
            .values()
            .find(|s| s.object == object && !s.cloud && (s.ps.is_emitting() || !s.ps.particles().is_empty()))
            .map(|s| s.owner)
    }

    pub(crate) fn owner_particle_count(&self, owner: Uuid) -> usize {
        self.sources
            .values()
            .filter(|s| !s.cloud && s.owner == owner)
            .map(|s| s.ps.particles().len())
            .sum()
    }

    /// The particle a right click lands on (LLPipeline::
    /// lineSegmentIntersectParticle, LLVOPartGroup::lineSegmentIntersect):
    /// the source object of the nearest live particle the ray `origin` +
    /// t·`dir` crosses for t in 0..`max`. Use particle_vs geometry, including
    /// velocity-aligned quads and ribbons. Clouds/HUD particles are excluded.
    #[cfg(test)]
    pub fn pick(&self, ray: (Vec3, Vec3), view: (Vec3, Vec3, Vec3), max: f32) -> Option<ParticleHit> {
        self.pick_all(ray, view, max).into_iter().next()
    }

    pub fn pick_all(&self, ray: (Vec3, Vec3), view: (Vec3, Vec3, Vec3), max: f32) -> Vec<ParticleHit> {
        let (origin, dir) = ray;
        let Some(dir) = dir.try_normalize() else { return Vec::new() };
        let mut hits = Vec::new();
        for (id, s) in &self.sources {
            if s.cloud {
                continue;
            }
            let mut best: Option<ParticleHit> = None;
            for p in s.ps.particles() {
                if p.flags & ps::LL_PART_HUD != 0 {
                    continue;
                }
                let Some((pos, right, up)) = pick_geometry(p, view) else { continue };
                let Some(t) = quad_hit(origin, dir, right, up, pos) else {
                    continue;
                };
                if t <= max && best.is_none_or(|hit| t < hit.distance) {
                    best = Some(ParticleHit {
                        source: *id,
                        object: s.object,
                        owner: s.owner,
                        point: origin + dir * t,
                        distance: t,
                    });
                }
            }
            if let Some(hit) = best {
                hits.push(hit);
            }
        }
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance).then_with(|| a.source.cmp(&b.source)));
        let mut objects = std::collections::HashSet::new();
        hits.retain(|hit| objects.insert(hit.object));
        hits.truncate(128);
        hits
    }

    /// Build › Edit particles: show `system` on the prim `object` (local
    /// only, nothing is sent), or stop showing it with `None`. Called every
    /// frame with the editor's state; a change of parameters keeps the
    /// particles already out, as an object update would.
    /// ParticleEditor::updateParticles / clearParticles
    /// (indra/newview/particleeditor.cpp, originally LGPL 2.1).
    pub fn set_preview(&mut self, wanted: Option<(Uuid, ps::PartSysData)>) {
        let object = wanted.map(|(object, _)| object);
        if let Some(old) = self.preview.filter(|old| Some(*old) != object)
            && let Some(s) = self.sources.get_mut(&preview_key(old))
        {
            // closed or moved to another prim: its particles go at once
            s.ps.stop();
            s.ps.clear_particles();
        }
        self.preview = object;
        let Some((object, data)) = wanted else { return };
        let key = preview_key(object);
        if let Some(s) = self.sources.get_mut(&key) {
            if s.ps.data() != &data || !s.ps.is_emitting() {
                s.ps.set_data(data);
            }
            return;
        }
        self.seed = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        // the texture is acquired by the update's texture change
        self.sources.insert(
            key,
            Source {
                ps: ParticleSource::new(data, self.seed ^ key.as_u128() as u64),
                texture: Uuid::nil(),
                slot: 0,
                cloud: false,
                owner: Uuid::nil(),
                object,
                emitter: object,
            },
        );
    }

    /// Release every source (region change, logout).
    pub fn clear(&mut self, textures: &mut TextureStreamer) {
        for (_, s) in self.sources.drain() {
            textures.release(&s.texture);
        }
        self.live = 0;
        self.property_owners.clear();
        self.preview = None;
    }

    /// Apply object updates, simulate and build the sorted instance list.
    /// `frozen`: the photo mode holds the world still (issue #1154). The
    /// objects' particles get no time and no source emits, a script's new
    /// system included (LLFloater360Capture::freezeWorld calls
    /// LLViewerPartSim::enable(false), whose zero particle count makes
    /// shouldAddPart refuse every new particle; llviewerpartsim.cpp,
    /// originally LGPL 2.1). The avatars' loading clouds go on, like the
    /// rez effects.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        textures: &mut TextureStreamer,
        world: &mut World,
        dt: f32,
        frozen: bool,
        eye: Vec3,
        max_particles: usize,
        draw_distance: f32,
        filters: &crate::world::particle_filters::ParticleFilters,
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
            let object = world
                .objects
                .get(crate::build::root_of(world, idx))
                .map(|o| o.full_id)
                .unwrap_or_default();
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
                            owner: o.owner_id,
                            object,
                            emitter: o.full_id,
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
        let previewed = self.preview;
        for (id, s) in self.sources.iter_mut() {
            let idx = world.objects.index_of_uuid(&s.emitter);
            if let Some(object) = idx
                .and_then(|i| world.objects.get(crate::build::root_of(world, i)))
                .map(|o| o.full_id)
            {
                s.object = object;
            }
            if let Some(owner) = idx.and_then(|i| world.objects.get(i)).map(|o| o.owner_id).filter(|id| !id.is_nil()) {
                s.owner = owner;
            } else if let Some(owner) = self.property_owners.get(&s.emitter).or_else(|| self.property_owners.get(&s.object)) {
                s.owner = *owner;
            }
            let blocked = (!s.cloud && filters.hides(s.object, s.owner))
                || previewed == Some(*id)
                || world.particles_blocked(&s.owner)
                || idx.is_some_and(|idx| !world.parcel_object_visible(idx) || !crate::snapshot::isolate::object_visible(world, idx));
            if blocked {
                // Clear existing particles too, even if the emitter has gone
                // out of view. Local filters must take effect immediately.
                s.ps.clear_particles();
            }
            // (a source's clock does not run while frozen: without this,
            // a new one would burst at every frame, LL's first run)
            let held = frozen && !s.cloud;
            let dt = if held { 0.0 } else { dt };
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
            if blocked || held || pos.distance(eye) > draw_distance {
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
                let Some((pos, axis, size, shape)) = geometry(p) else {
                    continue;
                };
                flags |= shape;
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

/// Quad of a particle: centre, axis, size and `particle_flags` shape bits,
/// or `None` when it covers nothing (LLVOPartGroup::getGeometry).
///
/// A ribbon segment runs to its parent particle and is `size.x` wide: its
/// `size.y` is never used. Particles emitted in the same frame (several per
/// burst, or several bursts in one frame) share their position and motion,
/// so a segment between two of them stays empty all its life; the Legacy
/// Viewer draws it as a zero-area quad. Drawing it as a billboard instead
/// stood each such link of a leash chain up `size.y` tall (#1049).
fn geometry(p: &ps::Particle) -> Option<(Vec3, Vec3, [f32; 2], u32)> {
    if p.is_ribbon() {
        let seg = p.prev_pos - p.pos;
        if seg.length_squared() <= 1e-8 {
            return None;
        }
        return Some((p.pos + seg * 0.5, seg, [p.size.x, seg.length()], particle_flags::AXIS));
    }
    if p.flags & ps::LL_PART_FOLLOW_VELOCITY_MASK != 0 && p.vel.length_squared() > 1e-8 {
        return Some((
            p.pos,
            p.vel.normalize() * p.size.y,
            [p.size.x, p.size.y],
            particle_flags::FOLLOW_VELOCITY,
        ));
    }
    Some((p.pos, Vec3::ZERO, [p.size.x, p.size.y], 0))
}

/// Full quad axes from object.wgsl::particle_vs, not an approximation facing
/// the click ray. Picking must follow the geometry actually visible on screen.
fn pick_geometry(p: &ps::Particle, (eye, mut right, mut up): (Vec3, Vec3, Vec3)) -> Option<(Vec3, Vec3, Vec3)> {
    let (pos, axis, size, shape) = geometry(p)?;
    if shape == particle_flags::FOLLOW_VELOCITY {
        let to_cam = (eye - pos).try_normalize()?;
        let view_right = Vec3::Z.cross(to_cam);
        if view_right.length_squared() > 1e-8 {
            right = view_right.normalize();
            up = to_cam.cross(right).normalize();
        }
        let projected = glam::Vec2::new(axis.dot(right), axis.dot(up));
        if projected.length_squared() > 1e-8 {
            let direction = projected.normalize();
            (right, up) = (right * direction.y - up * direction.x, right * direction.x + up * direction.y);
        }
    } else if shape == particle_flags::AXIS {
        let to_cam = (eye - pos).try_normalize()?;
        let side = to_cam.cross(axis.normalize());
        if side.length_squared() > 1e-6 {
            right = side.normalize();
        }
        return Some((pos, right * size[0], axis));
    }
    Some((pos, right * size[0], up * size[1]))
}

/// Ray/parallelogram intersection, with full edge vectors (also handles the
/// ribbon fallback's non-orthogonal axes). Like LLVOPartGroup triangle picking.
fn quad_hit(origin: Vec3, dir: Vec3, right: Vec3, up: Vec3, pos: Vec3) -> Option<f32> {
    let normal = right.cross(up);
    let norm2 = normal.length_squared();
    let denom = dir.dot(normal);
    if norm2 <= 1e-12 || denom.abs() <= 1e-8 {
        return None;
    }
    let t = (pos - origin).dot(normal) / denom;
    if !t.is_finite() || t < 0.0 {
        return None;
    }
    let local = origin + dir * t - pos;
    let u = local.cross(up).dot(normal) / norm2;
    let v = right.cross(local).dot(normal) / norm2;
    (u.abs() <= 0.5 && v.abs() <= 0.5).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick_source(flags: u32, size: glam::Vec2, acceleration: Vec3) -> Source {
        let mut data = ps::PartSysData {
            pattern: ps::LL_PART_SRC_PATTERN_DROP,
            burst_rate: 0.1,
            burst_part_count: 1,
            part_accel: acceleration,
            ..Default::default()
        };
        data.part.flags = flags;
        data.part.start_scale = size;
        data.part.end_scale = size;
        let mut source = Source {
            ps: ParticleSource::new(data, 1),
            texture: Uuid::nil(),
            slot: 0,
            cloud: false,
            owner: Uuid::from_u128(71),
            object: Uuid::from_u128(72),
            emitter: Uuid::from_u128(72),
        };
        source.ps.update_with_wind(
            0.0,
            &SourceContext {
                pos: Vec3::X * 10.0,
                ..Default::default()
            },
            &mut 1,
            |_| Vec3::ZERO,
        );
        source
    }

    #[test]
    fn particle_pick_matches_velocity_aligned_rendering_and_captures_owner() {
        let id = Uuid::from_u128(72);
        let mut source = pick_source(ps::LL_PART_FOLLOW_VELOCITY_MASK, glam::Vec2::new(0.1, 6.0), Vec3::Y * 10.0);
        source.ps.stop();
        source.ps.update_with_wind(
            0.1,
            &SourceContext {
                pos: Vec3::X * 10.0,
                ..Default::default()
            },
            &mut 0,
            |_| Vec3::ZERO,
        );
        let center = source.ps.particles()[0].pos;
        let mut manager = ParticleManager::default();
        manager.sources.insert(id, source);
        let view = (Vec3::ZERO, Vec3::NEG_Y, Vec3::Z);
        // The long axis is horizontal on screen, not the old vertical quad.
        let ray = (Vec3::ZERO, (center + Vec3::Y * 2.5).normalize());
        let hit = manager.pick(ray, view, 30.0).expect("visible stretched particle");
        assert_eq!((hit.source, hit.owner), (id, Uuid::from_u128(71)));
        assert!(
            manager
                .pick((Vec3::ZERO, (center + Vec3::Y * 2.5 + Vec3::Z * 0.2).normalize()), view, 30.0)
                .is_none()
        );
        assert!(hit.before_surface(ray, None), "a particle in front of sky is selectable");
        assert!(hit.before_surface(ray, Some(ray.1 * 20.0)));
        assert!(
            !hit.before_surface(ray, Some(ray.1 * 5.0)),
            "opaque foreground still occludes particles"
        );
        // An asynchronous click keeps the sampled point/owner after movement
        // or emitter removal, without looking them up from a reused object ID.
        manager.sources.clear();
        assert_eq!(hit.owner, Uuid::from_u128(71));
        assert!(hit.before_surface(ray, None));
    }

    #[test]
    fn particle_pick_uses_the_visible_ribbon_segment_instead_of_its_endpoint() {
        let mut source = pick_source(ps::LL_PART_RIBBON_MASK, glam::Vec2::splat(0.2), Vec3::ZERO);
        source.ps.update_with_wind(
            0.1,
            &SourceContext {
                pos: Vec3::new(10.0, 5.0, 0.0),
                ..Default::default()
            },
            &mut 1,
            |_| Vec3::ZERO,
        );
        let mut manager = ParticleManager::default();
        manager.sources.insert(Uuid::from_u128(72), source);
        let view = (Vec3::ZERO, Vec3::NEG_Y, Vec3::Z);
        let ray = (Vec3::ZERO, Vec3::new(10.0, 2.5, 0.0).normalize());
        assert!(manager.pick(ray, view, 30.0).is_some());
        assert!(
            manager
                .pick((Vec3::ZERO, Vec3::new(10.0, 2.5, 0.3).normalize()), view, 30.0)
                .is_none()
        );
    }

    #[test]
    fn particle_pick_respects_camera_billboard_axes_and_excludes_clouds_huds() {
        let id = Uuid::from_u128(72);
        let mut manager = ParticleManager::default();
        manager.sources.insert(id, pick_source(0, glam::Vec2::new(0.1, 6.0), Vec3::ZERO));
        // A rolled camera rotates the visible long axis with the billboard.
        let view = (Vec3::ZERO, Vec3::Z, Vec3::Y);
        let ray = (Vec3::ZERO, Vec3::new(10.0, 2.5, 0.0).normalize());
        assert!(manager.pick(ray, view, 30.0).is_some());
        manager.sources.get_mut(&id).unwrap().cloud = true;
        assert!(manager.pick(ray, view, 30.0).is_none());
        manager
            .sources
            .insert(id, pick_source(ps::LL_PART_HUD, glam::Vec2::splat(6.0), Vec3::ZERO));
        assert!(manager.pick(ray, view, 30.0).is_none());
    }

    #[test]
    fn particle_pick_collects_distinct_emitters_and_deduplicates_linked_sources() {
        let mut manager = ParticleManager::default();
        for (id, object) in [(1, 11), (2, 22), (3, 11)] {
            let mut source = pick_source(0, glam::Vec2::splat(2.0), Vec3::ZERO);
            source.object = Uuid::from_u128(object);
            manager.sources.insert(Uuid::from_u128(id), source);
        }
        let hits = manager.pick_all((Vec3::ZERO, Vec3::X), (Vec3::ZERO, Vec3::NEG_Y, Vec3::Z), 20.0);
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits.iter().map(|h| h.object).collect::<Vec<_>>(),
            [Uuid::from_u128(11), Uuid::from_u128(22)]
        );
    }

    #[test]
    fn the_particle_editor_preview_follows_its_prim_and_goes_on_close() {
        let object = Uuid::from_u128(5);
        let mut manager = ParticleManager::default();
        let data = ps::PartSysData {
            burst_part_count: 3,
            ..Default::default()
        };
        manager.set_preview(Some((object, data)));
        let key = preview_key(object);
        assert_ne!(key, object);
        let s = &manager.sources[&key];
        assert_eq!((s.emitter, s.object, s.cloud), (object, object, false));
        assert_eq!(manager.preview, Some(object));
        // a change of parameters updates the same source
        let changed = ps::PartSysData {
            burst_part_count: 7,
            ..data
        };
        manager.set_preview(Some((object, changed)));
        assert_eq!(manager.sources.len(), 1);
        assert_eq!(manager.sources[&key].ps.data().burst_part_count, 7);
        // another prim: the first preview stops, the new one starts
        let other = Uuid::from_u128(6);
        manager.set_preview(Some((other, changed)));
        assert!(!manager.sources[&key].ps.is_emitting());
        assert!(manager.sources[&preview_key(other)].ps.is_emitting());
        // closed: nothing emits any more
        manager.set_preview(None);
        assert_eq!(manager.preview, None);
        assert!(manager.sources.values().all(|s| !s.ps.is_emitting() && s.ps.particles().is_empty()));
    }

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
                owner: Uuid::nil(),
                object: Uuid::from_u128(72),
                emitter: id,
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
            owner: Uuid::nil(),
            object: Uuid::from_u128(72),
            emitter: Uuid::from_u128(72),
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
    fn ribbon_links_emitted_together_draw_nothing_instead_of_standing_up() {
        // a leash: ribbon chain from the collar towards a held point, two
        // links per burst, a long y scale that ribbons ignore
        let mut data = ps::PartSysData {
            pattern: ps::LL_PART_SRC_PATTERN_DROP,
            burst_rate: 0.05,
            burst_part_count: 2,
            part_accel: Vec3::new(0.0, 0.0, -1.0),
            ..Default::default()
        };
        data.part.flags = ps::LL_PART_RIBBON_MASK | ps::LL_PART_FOLLOW_SRC_MASK | ps::LL_PART_TARGET_POS_MASK;
        data.part.max_age = 3.5;
        data.part.start_scale = glam::Vec2::new(0.07, 3.0);
        data.part.end_scale = data.part.start_scale;
        let mut source = ParticleSource::new(data, 1);
        let ctx = SourceContext {
            pos: Vec3::new(10.0, 10.0, 21.0),
            target: Some(Vec3::new(12.0, 10.0, 21.5)),
            ..Default::default()
        };
        let mut budget = usize::MAX;
        for _ in 0..40 {
            source.update_with_wind(0.06, &ctx, &mut budget, |_| Vec3::ZERO);
        }
        let quads: Vec<_> = source.particles().iter().map(geometry).collect();
        assert!(quads.len() > 20);
        assert!(quads.iter().any(Option::is_none), "links of one burst share their position");
        for (_, axis, size, shape) in quads.into_iter().flatten() {
            assert_eq!(shape, particle_flags::AXIS);
            assert_eq!(size, [0.07, axis.length()]);
            assert!(size[1] < 0.5, "a segment runs between neighbouring links, not {}", size[1]);
        }
    }

    #[test]
    fn a_ray_picks_a_particle_quad_in_front_only() {
        let (dir, right, up) = (Vec3::X, Vec3::NEG_Y, Vec3::Z);
        let (right, up) = (right, up * 0.5);
        let at = Vec3::new(10.0, 0.0, 0.0);
        assert_eq!(quad_hit(Vec3::ZERO, dir, right, up, at), Some(10.0));
        // within the half sizes, sideways and up
        assert!(quad_hit(Vec3::new(0.0, 0.45, 0.2), dir, right, up, at).is_some());
        assert!(quad_hit(Vec3::new(0.0, 0.0, 0.3), dir, right, up, at).is_none());
        // behind the eye
        assert!(quad_hit(Vec3::new(11.0, 0.0, 0.0), dir, right, up, at).is_none());
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
