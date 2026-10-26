//! Visual contacts derived from displayed transforms and animated joints.
//! No messages, forces, scripts or physics changes are sent to the grid.

use super::{Scene, anchors, anim, water_contacts};
use crate::world::World;
use aurora_render::{WaterEffects, WaterEvent, WaterGround};
use glam::{Mat4, Vec2, Vec3};
use rayon::prelude::*;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

#[derive(Default)]
pub struct Effects {
    pub geometry: Option<u8>,
    region: Option<u64>,
    candidates: Vec<usize>,
    scan_at: f32,
    ground_at: f32,
    ground: WaterGround,
    history: HashMap<(Uuid, u8), Contact>,
    events: Vec<WaterEvent>,
    impacts: Vec<crate::movement::water_jump::Impact>,
    bodies: water_contacts::Bodies,
    visible: HashMap<Uuid, u32>,
}

#[derive(Clone, Copy)]
struct Contact {
    position: Vec3,
    time: f32,
    ripple_at: f32,
    splash_at: f32,
}

impl Contact {
    fn new(position: Vec3, time: f32) -> Self {
        Self {
            position,
            time,
            ripple_at: time,
            splash_at: time - 0.5,
        }
    }
}

fn water_at(world: &World, p: Vec3) -> Option<f32> {
    world.regions.values().find_map(|r| {
        let off = world.region_offset(r.handle)?;
        let size = Vec2::new(r.heightmap.size_x as f32, r.heightmap.size_y as f32);
        let xy = p.truncate() - off.truncate();
        (xy.cmpge(Vec2::ZERO).all() && xy.cmplt(size).all())
            .then(|| r.info.as_ref().map(|i| i.water_height))
            .flatten()
    })
}

impl Effects {
    fn retain_events(&mut self, time: f32, quality: u8, eye: Vec2) {
        self.events.retain(|event| {
            let lifetime = match event.params[1] {
                3.0 => 0.3,       // The GPU field has already integrated the impulse.
                4.0 | 8.0 => 2.0, // Swept surface segments spread and settle in world space.
                5.0 | 6.0 => 1.6, // Sheet flight and its expanding foam rim.
                0.0 | 7.0 => 0.8,
                _ => 3.0,
            };
            (0.0..lifetime).contains(&(time - event.position[3]))
        });
        let max = [64, 96, 160, 192][quality.min(3) as usize];
        if self.events.len() > max {
            // Keep an entry's complete silhouette through its short lifetime,
            // even when many ordinary contacts fill the Low-quality history.
            self.events.sort_unstable_by(|a, b| {
                let priority = |e: &WaterEvent| {
                    if matches!(e.params[1], 5.0 | 6.0) {
                        2
                    } else if (Vec2::new(e.position[0], e.position[1]) - eye).abs().max_element() <= 18.0 {
                        1
                    } else {
                        0
                    }
                };
                priority(a).cmp(&priority(b)).then_with(|| a.position[3].total_cmp(&b.position[3]))
            });
            self.events.drain(..self.events.len() - max);
        }
    }

    fn emit_impact(&mut self, impact: crate::movement::water_jump::Impact, time: f32, settings: WaterEffects) {
        use crate::movement::water_jump::Kind;
        if !impact.position.is_finite() || !impact.speed.is_finite() {
            return;
        }
        let big = impact.kind == Kind::Cannonball;
        let strength = (impact.speed * if big { 0.28 } else { 0.18 }).clamp(if big { 1.4 } else { 0.7 }, if big { 3.5 } else { 2.2 });
        if settings.interactions {
            self.events.push(WaterEvent {
                position: impact.position.extend(time).to_array(),
                motion: [0.0, 0.0, -impact.speed, strength],
                params: [if big { 0.85 } else { 0.38 }, 6.0, impact.position.z, 0.0],
            });
        }
        if settings.bubbles {
            self.events.push(WaterEvent {
                position: (impact.position - Vec3::Z * 0.22).extend(time).to_array(),
                motion: [0.0, 0.0, 0.0, 0.18],
                params: [0.06, 1.0, impact.position.z, 0.0],
            });
        }
        if settings.splashes {
            let packets = if big { 10 } else { 6 };
            let arc = std::f32::consts::TAU / packets as f32;
            for i in 0..packets {
                let angle = i as f32 * arc;
                let direction = Vec3::new(angle.cos(), angle.sin(), 0.0);
                self.events.push(WaterEvent {
                    position: impact.position.extend(time).to_array(),
                    motion: direction.extend(strength).to_array(),
                    params: [if big { 0.46 } else { 0.22 }, 5.0, impact.position.z, arc],
                });
            }
        }
    }

    fn emit(&mut self, id: (Uuid, u8), pos: Vec3, radius: f32, water: f32, time: f32, settings: WaterEffects) {
        if !pos.is_finite() || !water.is_finite() || !time.is_finite() || !radius.is_finite() {
            return;
        }
        let previous = self.history.get(&id).copied();
        self.history.insert(
            id,
            Contact {
                position: pos,
                time,
                ..previous.unwrap_or_else(|| Contact::new(pos, time))
            },
        );
        let Some(previous) = previous else {
            return;
        };
        let dt = time - previous.time;
        let delta = pos - previous.position;
        if !(0.001..0.5).contains(&dt) || delta.length() > 8.0 {
            self.history.insert(id, Contact::new(pos, time));
            return;
        }
        let mut velocity = delta / dt;
        if velocity.length() > 40.0 {
            velocity = velocity.normalize() * 40.0;
        }
        let speed = velocity.length();
        let surface = Vec3::new(pos.x, pos.y, water);
        let state = self.history.get_mut(&id).expect("contact inserted above");
        if settings.interactions && speed > 0.2 && time - state.ripple_at >= 0.18 {
            self.events.push(WaterEvent {
                position: surface.extend(time).to_array(),
                motion: velocity.extend((speed * 0.085).clamp(0.02, 0.85)).to_array(),
                params: [radius.clamp(0.15, 3.0), 3.0, water, 0.0],
            });
            state.ripple_at = time;
        }
        if settings.splashes && (pos.z - water).abs() <= radius.max(0.5) && velocity.z < -0.6 && time - state.splash_at >= 0.38 {
            self.events.push(WaterEvent {
                position: surface.extend(time).to_array(),
                motion: velocity.extend((velocity.z.abs() * 0.17).clamp(0.12, 1.2)).to_array(),
                params: [radius.clamp(0.15, 3.0), 0.0, water, 0.0],
            });
            state.splash_at = time;
        }
    }

    fn update(
        &mut self,
        scene: &Scene,
        world: &World,
        eye: Vec3,
        time: f32,
        settings: WaterEffects,
    ) -> (Vec<WaterEvent>, Vec<WaterEvent>, WaterGround) {
        if self.region != world.main_region || settings.flags() == 0 {
            *self = Self {
                region: world.main_region,
                ..Default::default()
            };
        }
        if settings.flags() == 0 {
            return Default::default();
        }
        self.geometry = (settings.waves || settings.interactions).then_some(settings.quality);
        let range = settings.range();
        let extent = range * 2.0;
        // Integer multiples of the simulation cell: scrolling preserves wave phase.
        let cell = extent / settings.resolution().max(128) as f32;
        let anchor = (eye.truncate() / (cell * 16.0)).floor() * (cell * 16.0);
        let origin = anchor - Vec2::splat(range);
        if self.ground.extent != extent || self.ground.origin != origin.to_array() || time - self.ground_at > 0.5 {
            self.ground_at = time;
            let heights: Vec<[f32; 2]> = (0..4096usize)
                .into_par_iter()
                .map(|i| {
                    let xy = origin + (Vec2::new((i % 64) as f32, (i / 64) as f32) + Vec2::splat(0.5)) * extent / 64.0;
                    let p = xy.extend(0.0);
                    let w = water_at(world, p).unwrap_or(world.main_water_height());
                    [world.ground_height(p).unwrap_or(w - 100.0), w]
                })
                .collect();
            self.ground = WaterGround {
                region: world.main_region.unwrap_or(0),
                origin: origin.to_array(),
                extent,
                heights: heights.into(),
            };
        }
        if settings.flags() & (4 | 16 | 64 | 128) == 0 {
            self.events.clear();
            self.impacts.clear();
            self.history.clear();
            return (Vec::new(), Vec::new(), self.ground.clone());
        }
        if self.candidates.is_empty() || time - self.scan_at > 0.35 {
            self.scan_at = time;
            let mut near: Vec<(f32, usize)> = scene
                .gpu
                .par_iter()
                .enumerate()
                .filter_map(|(i, g)| {
                    let object = world.objects.get(i)?;
                    if g.hud || g.owner_avatar.is_some() || object.extra.reflection_probe.is_some() {
                        return None;
                    }
                    let distance = g.center.truncate().distance_squared(eye.truncate());
                    if distance >= range * range || (!g.is_avatar && g.faces.is_empty()) {
                        return None;
                    }
                    let height = object.scale.z.abs().max(if g.is_avatar { 2.0 } else { 0.0 });
                    let w = water_at(world, g.center)?;
                    let nearby_avatar = g.is_avatar && g.center.distance_squared(eye) < range * range;
                    (nearby_avatar || (!g.is_avatar && (g.center.z - w).abs() < height * 0.5 + 8.0)).then_some((distance, i))
                })
                .collect();
            near.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            self.candidates = near.into_iter().take(96).map(|(_, i)| i).collect();
            self.visible = water_contacts::visibility(scene, world, &self.candidates);
        }
        let now = Instant::now();
        let mut contacts = Vec::new();
        let body_samples = crate::frame_par::fold(
            &self.candidates,
            crate::frame_par::Grain { chunk: 4, per_helper: 1 },
            Vec::new,
            |mut samples, &idx| {
                let Some(o) = world.objects.get(idx).filter(|o| o.is_avatar()) else {
                    return samples;
                };
                if scene.too_complex.contains(&idx)
                    || world.derender.contains(&o.full_id)
                    || world.particles_blocked(&o.full_id)
                    || !world.parcel_object_visible(idx)
                    || !crate::snapshot::isolate::object_visible(world, idx)
                {
                    return samples;
                }
                let Some((p, r, _)) = Scene::object_transform(world, idx, now, 0) else {
                    return samples;
                };
                let Some(water) = water_at(world, p) else {
                    return samples;
                };
                let Some(slot) = scene.palette_slots.get(&o.full_id) else {
                    return samples;
                };
                let base = *slot as usize * anim::PALETTE_JOINTS;
                let model = anchors::skeleton_model(world, &scene.meshes, idx, p, r, 1.0);
                let visible = self.visible.get(&o.full_id).copied().unwrap_or(0);
                for (part, name) in water_contacts::PARTS.iter().enumerate() {
                    if visible & (1 << part) == 0 {
                        continue;
                    }
                    let Some(j) = scene.avatar_lib.rig.names.get(*name) else {
                        continue;
                    };
                    let Some(joint) = scene.palettes.get(base + j) else {
                        continue;
                    };
                    let matrix = model * Mat4::from_cols_array_2d(joint);
                    let center = matrix.w_axis.truncate();
                    // Distant avatars retain sparse head air; fine contact work is
                    // confined to the camera's local water patch.
                    if part != water_contacts::PARTS.len() - 1 && (center.truncate() - eye.truncate()).abs().max_element() > 18.0 {
                        continue;
                    }
                    let water = water_at(world, center).unwrap_or(water);
                    let ground = self.ground.sample(center.truncate()).map(|p| p[0]).unwrap_or(water - 20.0);
                    let surface = aurora_render::water_effects::surface_level(center.truncate(), time, settings, water, ground);
                    samples.push(water_contacts::Sample {
                        id: o.full_id,
                        part,
                        matrix,
                        level: water,
                        surface,
                        ground,
                        carrier_z: world.water_visual_dz(&o.full_id),
                    });
                }
                samples
            },
        );
        for sample in body_samples.into_iter().flatten() {
            let before = self.events.len();
            let mut part_settings = settings;
            if (sample.matrix.w_axis.truncate().truncate() - eye.truncate()).abs().max_element() > 18.0 {
                part_settings.interactions = false;
                part_settings.splashes = false;
            }
            self.bodies.sample(sample, time, part_settings, &mut self.events);
            if self.events.len() > before {
                log::trace!(target: "aurora_viewer::water_contacts", "water part {}: center {:?}, surface {:.3}, events {:?}", water_contacts::PARTS[sample.part], sample.matrix.w_axis.truncate(), sample.surface, self.events[before..].iter().map(|e| (e.params[1], e.params[3], e.motion[3])).collect::<Vec<_>>());
            }
        }

        for &idx in &self.candidates.clone() {
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let owner = if o.is_avatar() { o.full_id } else { o.owner_id };
            if scene.too_complex.contains(&idx)
                || world.derender.contains(&o.full_id)
                || world.particles_blocked(&owner)
                || !world.parcel_object_visible(idx)
                || !crate::snapshot::isolate::object_visible(world, idx)
            {
                continue;
            }
            let Some((p, r, _)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            let Some(water) = water_at(world, p) else {
                continue;
            };
            if !o.is_avatar() {
                // Root and linked pieces participate; attachments are excluded above.
                let half = o.scale.abs() * 0.5;
                let z_extent = (r * Vec3::X).z.abs() * half.x + (r * Vec3::Y).z.abs() * half.y + (r * Vec3::Z).z.abs() * half.z;
                if p.z - z_extent > water + 0.2 || p.z + z_extent < water - 0.2 {
                    // Keep the approach velocity for the first visible impact.
                    self.history.insert((o.full_id, 10), Contact::new(p, time - 0.02));
                    continue;
                }
                let emitted = self.events.len();
                self.emit((o.full_id, 10), p, half.truncate().min_element().max(0.2), water, time, settings);
                for event in &mut self.events[emitted..] {
                    event.params[3] = half.truncate().max_element().clamp(0.3, 12.0);
                }
                if settings.contact_foam && contacts.len() < 64 {
                    let axis = (r * Vec3::X).truncate().normalize_or(Vec2::X);
                    contacts.push(WaterEvent {
                        position: p.extend(time).to_array(),
                        motion: [axis.x, axis.y, 0.0, 0.0],
                        params: [half.x.clamp(0.1, 20.0), 2.0, water, half.y.clamp(0.1, 20.0)],
                    });
                }
            }
        }
        for impact in std::mem::take(&mut self.impacts) {
            self.emit_impact(impact, time, settings);
        }
        self.history.retain(|_, contact| time - contact.time < 1.0);
        self.bodies.retain(time);
        self.retain_events(time, settings.quality, eye.truncate());
        (self.events.clone(), contacts, self.ground.clone())
    }
}

impl Scene {
    pub fn avatar_water_contact(&self, world: &World) -> Option<Vec3> {
        let idx = world.objects.index_of_uuid(&world.agent_id)?;
        let (p, r, _) = Self::object_transform(world, idx, Instant::now(), 0)?;
        let base = *self.palette_slots.get(&world.agent_id)? as usize * anim::PALETTE_JOINTS;
        let model = anchors::skeleton_model(world, &self.meshes, idx, p, r, 1.0);
        ["mHead", "mFootLeft", "mFootRight", "mWristLeft", "mWristRight", "mPelvis"]
            .into_iter()
            .filter_map(|name| {
                let j = self.avatar_lib.rig.names.get(name)?;
                let matrix = self.palettes.get(base + j)?;
                let contact = model.transform_point3(Mat4::from_cols_array_2d(matrix).w_axis.truncate()) - Vec3::Z * 0.08;
                contact.is_finite().then_some(contact)
            })
            .min_by(|a, b| a.z.total_cmp(&b.z))
    }

    pub fn water_impact(&mut self, impact: crate::movement::water_jump::Impact) {
        if self.water_effects.impacts.len() < 8 {
            self.water_effects.impacts.push(impact);
        }
    }

    pub fn update_water_effects(&mut self, world: &World, eye: Vec3, time: f32, settings: WaterEffects) {
        let mut effects = std::mem::take(&mut self.water_effects);
        let (events, contacts, ground) = effects.update(self, world, eye, time, settings);
        self.water_effects = effects;
        self.lists.water_events = events;
        self.lists.water_contacts = contacts;
        self.lists.water_ground = ground;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_quality_keeps_entry_sheets_and_expires_spent_contact_impulses() {
        use crate::movement::water_jump::{Impact, Kind};
        let mut effects = Effects::default();
        effects.emit_impact(
            Impact {
                position: Vec3::ZERO,
                speed: 8.0,
                kind: Kind::Cannonball,
            },
            1.0,
            WaterEffects::default(),
        );
        for i in 0..150 {
            effects.events.push(WaterEvent {
                position: [0.0, 0.0, 0.0, 1.02 + i as f32 * 0.0001],
                params: [0.3, 7.0, 0.0, 0.0],
                ..Default::default()
            });
        }
        effects.retain_events(1.1, 0, Vec2::ZERO);
        assert_eq!(effects.events.len(), 64);
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 5.0).count(), 10);
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 6.0).count(), 1);
        effects.events.push(WaterEvent {
            position: [0.0, 0.0, 0.0, 1.0],
            params: [0.3, 4.0, 0.0, 0.0],
            ..Default::default()
        });
        effects.retain_events(1.4, 0, Vec2::ZERO);
        assert!(!effects.events.iter().any(|e| e.params[1] == 4.0));
        effects.retain_events(2.7, 0, Vec2::ZERO);
        assert!(effects.events.iter().all(|e| e.params[1] == 1.0));
    }

    #[test]
    fn intentional_water_entries_have_one_bounded_burst_and_independent_effect_switches() {
        use crate::movement::water_jump::{Impact, Kind};
        let impact = Impact {
            position: Vec3::new(2.0, 3.0, 20.0),
            speed: 8.0,
            kind: Kind::Cannonball,
        };
        let mut effects = Effects::default();
        effects.emit_impact(impact, 1.0, WaterEffects::default());
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 5.0).count(), 10);
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 6.0).count(), 1);
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 1.0).count(), 1);
        let mut settings = WaterEffects {
            bubbles: false,
            splashes: false,
            ..Default::default()
        };
        effects.events.clear();
        effects.emit_impact(impact, 2.0, settings);
        assert_eq!(effects.events.len(), 1);
        assert_eq!(effects.events[0].params[1], 6.0);
        settings.interactions = false;
        effects.events.clear();
        effects.emit_impact(impact, 3.0, settings);
        assert!(effects.events.is_empty());
        effects.emit_impact(Impact { speed: f32::NAN, ..impact }, 3.0, WaterEffects::default());
        assert!(effects.events.is_empty());
        effects.emit_impact(
            Impact {
                kind: Kind::Dive,
                ..impact
            },
            4.0,
            WaterEffects::default(),
        );
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 5.0).count(), 6);
        assert!(effects.events.iter().all(|e| e.motion[3] <= 3.5));
        let dive: Vec<_> = effects.events.iter().filter(|e| e.params[1] == 5.0).collect();
        let span: f32 = dive.iter().map(|e| e.params[3]).sum();
        assert!(
            (span - std::f32::consts::TAU).abs() < 0.001,
            "one complete, non-overlapping entry sheet"
        );
        let mut cannonball = Effects::default();
        cannonball.emit_impact(impact, 4.0, WaterEffects::default());
        let big = cannonball.events.iter().find(|e| e.params[1] == 5.0).unwrap();
        assert!(big.motion[3] > dive[0].motion[3] && big.params[0] > dive[0].params[0]);
    }

    #[test]
    fn object_contacts_reject_teleports_and_preserve_independent_displacement() {
        let id = (Uuid::nil(), 10);
        let mut e = Effects::default();
        let s = WaterEffects::default();
        e.emit(id, Vec3::ZERO, 0.3, 0.0, 0.0, s);
        e.emit(id, Vec3::X * 100.0, 0.3, 0.0, 0.1, s);
        assert!(e.events.is_empty());
        e.emit(id, Vec3::new(100.2, 0.0, -0.2), 0.3, 0.0, 0.3, s);
        assert!(e.events.iter().any(|e| e.params[1] == 3.0));
        assert!(e.events.iter().any(|e| e.params[1] == 0.0));
        assert!(e.events.iter().all(|e| e.params[1] != 1.0));
    }
}
