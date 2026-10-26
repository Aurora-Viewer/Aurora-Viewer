//! Per-part visual water contacts. Fitted collision volumes follow the displayed
//! skeleton and shape; they are read only and never change simulator physics.

use aurora_render::{WaterEffects, WaterEvent};
use glam::{Mat3, Mat4, Vec2, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

pub(super) const PARTS: [&str; 17] = [
    "L_FOOT",
    "R_FOOT",
    "L_LOWER_LEG",
    "R_LOWER_LEG",
    "L_UPPER_LEG",
    "R_UPPER_LEG",
    "L_HAND",
    "R_HAND",
    "L_LOWER_ARM",
    "R_LOWER_ARM",
    "L_UPPER_ARM",
    "R_UPPER_ARM",
    "PELVIS",
    "BELLY",
    "CHEST",
    "NECK",
    "HEAD",
];
const HEAD: usize = 16;

#[derive(Clone, Copy, Debug)]
struct Section {
    position: Vec3,
    local: Vec3,
    radius: f32,
    area: f32,
    depth: f32,
}

/// Intersect an animated ellipsoid with the horizontal surface. The projected
/// center slides on tilted limbs; `local` identifies the material point whose
/// previous transform is used for velocity instead of that sliding center.
fn section(matrix: Mat4, water: f32) -> Option<Section> {
    if !matrix.is_finite() || !water.is_finite() {
        return None;
    }
    let a = Mat3::from_mat4(matrix);
    let center = matrix.w_axis.truncate();
    let normal = a.transpose() * Vec3::Z;
    let extent = normal.length();
    if extent < 0.005 || extent > 3.0 {
        return None;
    }
    let distance = water - center.z;
    let depth = (distance - extent).max(0.0);
    if distance < -extent || depth > 0.28 {
        return None;
    }
    let d = (distance / extent).clamp(-1.0, 1.0);
    let n = normal / extent;
    let local = n * d;
    let position = matrix.transform_point3(local).with_z(water);
    let covariance = a * (Mat3::IDENTITY - Mat3::from_cols(n * n.x, n * n.y, n * n.z)) * a.transpose();
    // A shallow submerged part still disturbs the surface, attenuated by depth.
    let cut = if d > 0.0 { 1.0 } else { 1.0 - d * d };
    let xx = covariance.x_axis.x.max(0.0) * cut;
    let yy = covariance.y_axis.y.max(0.0) * cut;
    let xy = covariance.y_axis.x * cut;
    let area = std::f32::consts::PI * (xx * yy - xy * xy).max(0.0).sqrt();
    if area < 0.0002 {
        return None;
    }
    Some(Section {
        position,
        local,
        radius: (area / std::f32::consts::PI).sqrt().clamp(0.025, 0.45),
        area,
        depth,
    })
}

#[derive(Clone, Copy)]
struct Motion {
    matrix: Mat4,
    time: f32,
    wake_at: f32,
    wake_position: Option<Vec3>,
    strike_at: f32,
    splash_at: f32,
    bubble_at: f32,
    foot_armed: bool,
    carrier_z: f32,
}

#[derive(Clone, Copy)]
pub(super) struct Sample {
    pub id: Uuid,
    pub part: usize,
    pub matrix: Mat4,
    pub level: f32,
    pub surface: f32,
    pub ground: f32,
    pub carrier_z: f32,
}

#[derive(Default)]
pub(super) struct Bodies {
    history: HashMap<(Uuid, u8), Motion>,
}

impl Bodies {
    pub fn retain(&mut self, time: f32) {
        self.history.retain(|_, m| time - m.time < 0.5);
    }

    pub fn sample(&mut self, sample: Sample, time: f32, settings: WaterEffects, events: &mut Vec<WaterEvent>) {
        let Sample {
            id,
            part,
            matrix,
            level,
            surface,
            ground,
            carrier_z,
        } = sample;
        if !matrix.is_finite()
            || !level.is_finite()
            || !surface.is_finite()
            || !ground.is_finite()
            || !carrier_z.is_finite()
            || !time.is_finite()
        {
            return;
        }
        let key = (id, part as u8);
        let contact = section(matrix, surface);
        let foot = part < 2;
        let bottom = matrix.w_axis.z - (Mat3::from_mat4(matrix).transpose() * Vec3::Z).length();
        let current = Motion {
            matrix,
            time,
            wake_at: time,
            wake_position: contact.map(|c| c.position),
            strike_at: time - 0.4,
            splash_at: time - 0.4,
            bubble_at: time - 1.25,
            foot_armed: foot && bottom > surface + 0.02,
            carrier_z,
        };
        let Some(previous) = self.history.insert(key, current) else {
            return;
        };
        let dt = time - previous.time;
        let displacement = matrix.w_axis.truncate() - previous.matrix.w_axis.truncate();
        if !(0.001..0.25).contains(&dt) || displacement.length() > 2.0 {
            return;
        }
        let mut state = Motion {
            matrix,
            time,
            carrier_z,
            ..previous
        };
        let carried_speed = (carrier_z - previous.carrier_z) / dt;
        // Like the independent contact transitions of updateFootstepSounds
        // (indra/newview/llvoavatar.cpp), each foot must lift before another hit.
        // The visual extension tests the water surface, with hysteresis for waves.
        state.foot_armed |= foot && bottom > surface + 0.02;
        // A fast crossing can pass entirely through a small hand/foot between
        // frames. Sample its swept center rather than losing that impact.
        let swept = if contact.is_none_or(|c| c.depth > 0.0) && displacement.z.abs() > 0.01 {
            let t = ((surface - previous.matrix.w_axis.z) / displacement.z).clamp(0.0, 1.0);
            let between = previous.matrix * (1.0 - t) + matrix * t;
            section(between, surface).filter(|_| (previous.matrix.w_axis.z - surface) * (matrix.w_axis.z - surface) < 0.0)
        } else {
            None
        };
        if let Some(c) = swept.or(contact) {
            let velocity = (matrix.transform_point3(c.local) - previous.matrix.transform_point3(c.local)) / dt - Vec3::Z * carried_speed;
            let speed = velocity.length();
            if !velocity.is_finite() || speed > 40.0 {
                self.history.insert(key, current);
                return;
            }
            let horizontal = velocity.truncate().length();
            let top = matrix.w_axis.z + (Mat3::from_mat4(matrix).transpose() * Vec3::Z).length();
            // An ankle-deep foot can lift water without clearing its whole
            // sole. Rearm that partial lift only from actual upward material
            // motion; a wave moving past an idle shoe cannot create a step.
            state.foot_armed |= foot && velocity.z > 0.18 && top > surface + 0.025;
            let scale = (c.area / 0.08).sqrt().clamp(0.12, 1.4) * (-c.depth * 8.0).exp();
            let interval = [0.18, 0.15, 0.12, 0.10][settings.quality.min(3) as usize];
            let elapsed = time - state.wake_at;
            if settings.interactions && horizontal > 0.16 && elapsed >= interval {
                let path = state
                    .wake_position
                    .map(|p| c.position.truncate() - p.truncate())
                    .unwrap_or(Vec2::ZERO);
                let length = path.length();
                if (0.008..2.0).contains(&length) && elapsed < 0.3 {
                    // Pressure follows the interval's motion, not a brief
                    // animation peak. Area weighting prevents many small
                    // volumes from displacing a whole torso's worth of water.
                    let flow = path / elapsed;
                    let v2 = flow.length_squared();
                    let pressure = 0.08 * v2 / (1.0 + 0.35 * v2) * (c.area / 0.08).clamp(0.03, 1.4) * (-c.depth * 8.0).exp();
                    events.push(WaterEvent {
                        position: c.position.with_z(level - c.depth).extend(time).to_array(),
                        motion: flow.extend(0.0).extend(pressure).to_array(),
                        params: [c.radius, 4.0, level, length],
                    });
                }
                state.wake_at = time;
                state.wake_position = Some(c.position);
            } else if horizontal <= 0.16 {
                state.wake_at = time;
                state.wake_position = Some(c.position);
            }
            let previous_extent = (Mat3::from_mat4(previous.matrix).transpose() * Vec3::Z).length();
            let entered = previous.matrix.w_axis.z - previous_extent > surface - 0.015
                && matrix.w_axis.z - (Mat3::from_mat4(matrix).transpose() * Vec3::Z).length() <= surface;
            let emerged = previous.matrix.w_axis.z + previous_extent <= surface
                && matrix.w_axis.z + (Mat3::from_mat4(matrix).transpose() * Vec3::Z).length() > surface;
            let downstroke = velocity.z < -0.35 && (entered || time - state.strike_at >= 0.38);
            let footfall = foot && state.foot_armed && bottom < surface - 0.006 && velocity.z < -0.2 && surface - ground > 0.004;
            let strike = c.depth < 0.035
                && (if foot {
                    footfall
                } else {
                    downstroke || (emerged && velocity.z > 0.35)
                })
                && time - state.strike_at >= 0.28;
            if strike {
                let footprint = if foot { section(matrix, matrix.w_axis.z).unwrap_or(c) } else { c };
                let strength = if foot {
                    let down = (-velocity.z - 0.15).max(0.0);
                    let wet = ((surface - ground - 0.004) / 0.056).clamp(0.0, 1.0);
                    (down * 0.22 + down * down * 0.025).clamp(0.025, 0.75) * (footprint.area / 0.025).sqrt().clamp(0.4, 1.2) * wet
                } else {
                    (velocity.z.abs() * if downstroke { 0.17 } else { 0.07 } * scale).clamp(0.015, 0.65)
                };
                let flag = if foot { 1.0 } else { 0.0 };
                if settings.interactions {
                    events.push(WaterEvent {
                        position: c.position.with_z(level).extend(time).to_array(),
                        motion: velocity.extend(strength).to_array(),
                        params: [footprint.radius, 8.0, level, flag],
                    });
                }
                if settings.splashes {
                    events.push(WaterEvent {
                        position: c.position.extend(time).to_array(),
                        motion: if foot && horizontal < 0.03 {
                            let forward = matrix.x_axis.truncate().truncate().normalize_or(Vec2::X);
                            (forward * 0.03).extend(velocity.z).extend(strength).to_array()
                        } else {
                            velocity.extend(strength).to_array()
                        },
                        params: [footprint.radius, 7.0, level, flag],
                    });
                }
                state.strike_at = time;
                state.splash_at = time;
                state.foot_armed = false;
            }
            // Horizontal pushing makes a low, sparse spray only at an actual
            // surface intersection. Submerged hands cannot spray through water.
            if !foot && c.depth == 0.0 && horizontal > 1.0 && time - state.splash_at >= 0.45 {
                if settings.splashes {
                    events.push(WaterEvent {
                        position: c.position.extend(time).to_array(),
                        motion: velocity.extend((horizontal * 0.08 * scale).min(0.45)).to_array(),
                        params: [c.radius, 7.0, level, 0.0],
                    });
                }
                state.splash_at = time;
            }
        } else {
            state.wake_at = time;
            state.wake_position = None;
        }
        if foot && matrix.w_axis.z + (Mat3::from_mat4(matrix).transpose() * Vec3::Z).length() < surface - 0.02 {
            state.foot_armed = false;
        }
        // One head owns air, regardless of how many limbs touch the surface.
        let head_velocity = displacement / dt - Vec3::Z * carried_speed;
        if part == HEAD
            && settings.bubbles
            && matrix.w_axis.z < surface - 0.10
            && head_velocity.length() > 0.55
            && time - state.bubble_at >= 1.25
        {
            events.push(WaterEvent {
                position: matrix.w_axis.truncate().extend(time).to_array(),
                motion: head_velocity.extend(0.18).to_array(),
                params: [0.06, 1.0, level, 0.0],
            });
            state.bubble_at = time;
        }
        self.history.insert(key, state);
    }
}

/// Visible skin influences choose the closest anatomical volume, so a shoe
/// does not make the whole ancestor leg/pelvis visible. Collision-bone aliases
/// and Bento finger descendants resolve through the same ancestry.
pub(super) fn bone_masks(rig: &super::anim::Rig) -> Vec<u32> {
    let parents: Vec<_> = PARTS.iter().map(|name| rig.names.get(*name).and_then(|&j| rig.parent[j])).collect();
    (0..rig.len())
        .map(|joint| {
            let mut cursor = Some(joint);
            while let Some(j) = cursor {
                let mask = parents
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| **p == Some(j))
                    .fold(0, |mask, (part, _)| mask | (1 << part));
                if mask != 0 {
                    return mask;
                }
                cursor = rig.parent[j];
            }
            0
        })
        .collect()
}

pub(super) fn system_mask(name: &str) -> u32 {
    match name {
        "head" => 1 << HEAD,
        "upper" => (0x3f << 6) | (0xf << 12) & !(1 << 12),
        "lower" => 0x3f | (1 << 12),
        _ => 0,
    }
}

pub(super) fn visibility(scene: &super::Scene, world: &crate::world::World, candidates: &[usize]) -> HashMap<Uuid, u32> {
    use super::{GeomKey, Pass};
    let mut visible = HashMap::new();
    for &idx in candidates {
        let Some(object) = world.objects.get(idx).filter(|o| o.is_avatar()) else {
            continue;
        };
        let Some(g) = scene.gpu.get(idx) else { continue };
        let mask = visible.entry(object.full_id).or_insert(0);
        for (part, source) in scene.avatar_lib.parts.iter().enumerate() {
            let group = system_mask(source.name);
            if group == 0 {
                continue;
            }
            let keys = std::iter::once(GeomKey::AvatarPart(part as u8)).chain(
                g.morph_geoms
                    .iter()
                    .copied()
                    .filter(|key| matches!(key, GeomKey::AvatarMorph { part: p, .. } if *p as usize == part)),
            );
            for key in keys {
                let Some(geom) = scene.geom_ready(&key) else { continue };
                if geom.faces.iter().flatten().any(|alloc| {
                    g.faces.iter().any(|f| {
                        f.pass != Pass::Hidden
                            && !f.fully_transparent()
                            && f.cmd.base_vertex == alloc.vertex_offset as i32
                            && f.cmd.first_index == alloc.index_offset
                    })
                }) {
                    *mask |= group;
                }
            }
        }
    }
    let bones = bone_masks(&scene.avatar_lib.rig);
    // One bounded-cadence scan for all nearby avatars, not an attachment walk
    // per body part or per frame. Hidden/HUD geometry never supplies coverage.
    for (idx, g) in scene.gpu.iter().enumerate() {
        if !g.rigged || g.hud || !g.faces.iter().any(|f| f.pass != Pass::Hidden && !f.fully_transparent()) {
            continue;
        }
        let Some(owner) = g.skeleton_owner else { continue };
        let Some(mask) = visible.get_mut(&owner) else { continue };
        let Some(object) = world.objects.get(idx) else { continue };
        if world.derender.contains(&object.full_id)
            || !world.parcel_object_visible(idx)
            || !crate::snapshot::isolate::object_visible(world, idx)
        {
            continue;
        }
        let Some(GeomKey::Mesh { id, .. }) = g.geom else { continue };
        let Some(skin) = scene.meshes.meta(&id).and_then(|m| m.skin) else {
            continue;
        };
        let Some(geom) = g.geom.as_ref().and_then(|key| scene.geom_ready(key)) else {
            continue;
        };
        for bound in &geom.joint_bounds {
            if let Some(joint) = skin
                .joint_names
                .get(bound.joint as usize)
                .and_then(|name| scene.avatar_lib.rig.names.get(name))
            {
                *mask |= bones.get(*joint).copied().unwrap_or(0);
            }
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn volume(center: Vec3, radius: Vec3) -> Mat4 {
        Mat4::from_scale_rotation_translation(radius, Quat::IDENTITY, center)
    }
    fn sample(b: &mut Bodies, out: &mut Vec<WaterEvent>, part: usize, matrix: Mat4, time: f32, settings: WaterEffects) {
        b.sample(
            Sample {
                id: Uuid::nil(),
                part,
                matrix,
                level: 0.0,
                surface: 0.0,
                ground: -2.0,
                carrier_z: 0.0,
            },
            time,
            settings,
            out,
        );
    }

    #[test]
    fn surface_float_carried_motion_cannot_create_idle_spray_wakes_or_bubble_streams() {
        let (mut bodies, mut events) = (Bodies::default(), Vec::new());
        for frame in 0..600 {
            let time = frame as f32 / 60.0;
            let z = 0.3 * (time * 3.0).sin();
            for part in [0, 6, HEAD] {
                bodies.sample(
                    Sample {
                        id: Uuid::nil(),
                        part,
                        matrix: volume(Vec3::new(0.0, part as f32, z - 0.1), Vec3::splat(0.12)),
                        level: 0.0,
                        surface: z,
                        ground: -3.0,
                        carrier_z: z,
                    },
                    time,
                    WaterEffects::default(),
                    &mut events,
                );
            }
        }
        assert!(events.is_empty(), "carried movement is not active splashing or exhalation");
    }

    #[test]
    fn gentle_wakes_follow_displaced_area_and_speed_without_large_slow_or_fast_kick_troughs() {
        let strength = |speed: f32, radius: f32| {
            let (mut bodies, mut events) = (Bodies::default(), Vec::new());
            for frame in 0..=180 {
                let time = frame as f32 / 60.0;
                sample(
                    &mut bodies,
                    &mut events,
                    14,
                    volume(Vec3::X * speed * time, Vec3::splat(radius)),
                    time,
                    WaterEffects::default(),
                );
            }
            let wakes: Vec<_> = events.iter().filter(|e| e.params[1] == 4.0).collect();
            assert!(wakes.len() > 10);
            wakes.iter().map(|e| e.motion[3]).sum::<f32>() / wakes.len() as f32
        };
        let slow = strength(0.4, 0.16);
        let walk = strength(1.2, 0.16);
        let run = strength(3.0, 0.16);
        assert!(slow < 0.015 && walk < 0.09 && run < 0.2);
        assert!(walk > slow * 3.0 && run > walk && run < walk * 3.0);
        assert!(strength(1.2, 0.06) < walk * 0.2, "a small part does not act like a torso");
        assert!(strength(8.0, 0.16) < 0.25, "short fast strokes cannot make huge troughs");
    }

    fn stepping(period: f32, ground: f32, fps: u32, settings: WaterEffects) -> Vec<WaterEvent> {
        stepping_lift(period, ground, fps, settings, 0.06, 0.14)
    }

    fn stepping_lift(period: f32, ground: f32, fps: u32, settings: WaterEffects, height: f32, lift: f32) -> Vec<WaterEvent> {
        let (mut bodies, mut events) = (Bodies::default(), Vec::new());
        for frame in 0..=fps * 5 {
            let time = frame as f32 / fps as f32;
            for part in 0..2 {
                let phase = time * std::f32::consts::TAU / period + part as f32 * std::f32::consts::PI;
                bodies.sample(
                    Sample {
                        id: Uuid::nil(),
                        part,
                        matrix: volume(
                            Vec3::new(time, part as f32 * 0.4 - 0.2, height + lift * phase.cos()),
                            Vec3::new(0.14, 0.065, 0.06),
                        ),
                        level: 0.0,
                        surface: 0.0,
                        ground,
                        carrier_z: 0.0,
                    },
                    time,
                    settings,
                    &mut events,
                );
            }
        }
        events
    }

    #[test]
    fn footfalls_are_single_left_right_impacts_per_lift_and_scale_with_landing_and_water_depth() {
        for fps in [30, 60, 144] {
            let walk = stepping(1.2, -0.15, fps, WaterEffects::default());
            let run = stepping(0.65, -0.15, fps, WaterEffects::default());
            let hits = |events: &[WaterEvent]| events.iter().filter(|e| e.params[1] == 8.0).copied().collect::<Vec<_>>();
            let walk_hits = hits(&walk);
            let run_hits = hits(&run);
            let partial = stepping_lift(1.2, -0.15, fps, WaterEffects::default(), -0.02, 0.06);
            assert!(
                (7..=9).contains(&hits(&partial).len()),
                "{fps} fps: a lifted heel can slap while the sole stays wet"
            );
            assert!((7..=9).contains(&walk_hits.len()), "{fps} fps: {} walk hits", walk_hits.len());
            assert!((14..=16).contains(&run_hits.len()), "{fps} fps: {} run hits", run_hits.len());
            assert!(
                walk_hits
                    .iter()
                    .all(|e| e.params[3] == 1.0 && e.motion[2] < -0.2 && e.params[0] < 0.12)
            );
            for side in [-0.2, 0.2] {
                let foot: Vec<_> = walk_hits.iter().filter(|e| (e.position[1] - side).abs() < 0.001).collect();
                assert!(foot.len() >= 3);
                assert!(
                    foot.windows(2).all(|p| p[1].position[3] - p[0].position[3] > 1.0),
                    "one ring per foot lift"
                );
            }
            let mean = |events: &[WaterEvent]| events.iter().map(|e| e.motion[3]).sum::<f32>() / events.len() as f32;
            assert!(mean(&run_hits) > mean(&walk_hits) * 1.5);
            assert_eq!(
                walk.iter().filter(|e| e.params[1] == 7.0).count(),
                walk_hits.len(),
                "one small spray per strike"
            );
            let shallow = stepping(1.2, -0.015, fps, WaterEffects::default());
            assert!(mean(&hits(&shallow)) < mean(&walk_hits) * 0.3);
            let dry = stepping(1.2, 0.01, fps, WaterEffects::default());
            assert!(dry.iter().all(|e| e.params[1] != 7.0 && e.params[1] != 8.0));
            let particles_off = stepping(
                1.2,
                -0.15,
                fps,
                WaterEffects {
                    splashes: false,
                    ..Default::default()
                },
            );
            let displacement = |events: &[WaterEvent]| {
                events
                    .iter()
                    .filter(|e| matches!(e.params[1], 4.0 | 8.0))
                    .map(|e| (e.position, e.motion, e.params))
                    .collect::<Vec<_>>()
            };
            assert_eq!(displacement(&walk), displacement(&particles_off));
        }
    }

    #[test]
    fn submerged_dragging_and_stationary_wave_crossings_cannot_retrigger_foot_slaps() {
        for moving in [false, true] {
            let (mut bodies, mut events) = (Bodies::default(), Vec::new());
            for frame in 0..=300 {
                let time = frame as f32 / 60.0;
                bodies.sample(
                    Sample {
                        id: Uuid::nil(),
                        part: 0,
                        matrix: volume(
                            Vec3::new(if moving { time } else { 0.0 }, 0.0, if moving { -0.1 } else { 0.04 }),
                            Vec3::splat(0.06),
                        ),
                        level: 0.0,
                        surface: if moving { 0.0 } else { (time * 4.0).sin() * 0.1 },
                        ground: -0.2,
                        carrier_z: 0.0,
                    },
                    time,
                    WaterEffects::default(),
                    &mut events,
                );
            }
            assert!(events.iter().all(|e| e.params[1] != 7.0 && e.params[1] != 8.0));
        }
    }

    #[test]
    fn anatomical_intersections_are_local_and_stationary_waterline_sliding_has_no_velocity() {
        let matrix = Mat4::from_scale_rotation_translation(Vec3::new(0.06, 0.07, 0.3), Quat::from_rotation_y(0.7), Vec3::ZERO);
        let c = section(matrix, 0.1).expect("water crosses tilted leg");
        assert!((matrix.transform_point3(c.local).z - 0.1).abs() < 1e-5);
        assert!(c.radius < 0.15 && c.area > 0.0);
        assert!(section(volume(Vec3::Z, Vec3::splat(0.1)), 0.0).is_none());
        assert!(section(volume(-Vec3::Z, Vec3::splat(0.1)), 0.0).is_none());
        assert!(section(Mat4::ZERO, 0.0).is_none());
        let (mut b, mut events) = (Bodies::default(), Vec::new());
        for frame in 0..300 {
            let t = frame as f32 / 60.0;
            b.sample(
                Sample {
                    id: Uuid::nil(),
                    part: 2,
                    matrix,
                    level: 0.0,
                    surface: (t * 3.0).sin() * 0.1,
                    ground: -2.0,
                    carrier_z: 0.0,
                },
                t,
                WaterEffects::default(),
                &mut events,
            );
        }
        assert!(events.is_empty(), "the moving intersection is not moving body material");
    }

    #[test]
    fn two_feet_leave_separate_trails_and_dry_torso_head_and_arms_emit_none() {
        let (mut b, mut events) = (Bodies::default(), Vec::new());
        for frame in 0..=180 {
            let t = frame as f32 / 60.0;
            for (part, y, z, radius) in [
                (0, -0.2, 0.0, 0.06),
                (1, 0.2, 0.0, 0.06),
                (6, -0.5, 0.8, 0.05),
                (7, 0.5, 0.8, 0.05),
                (14, 0.0, 0.6, 0.2),
                (HEAD, 0.0, 1.0, 0.12),
            ] {
                sample(
                    &mut b,
                    &mut events,
                    part,
                    volume(Vec3::new(t * 1.5, y, z), Vec3::splat(radius)),
                    t,
                    WaterEffects::default(),
                );
            }
        }
        let wakes: Vec<_> = events.iter().filter(|e| e.params[1] == 4.0).collect();
        assert!(wakes.len() >= 35);
        assert!(
            wakes
                .iter()
                .all(|e| (e.position[1].abs() - 0.2).abs() < 0.001 && e.params[0] < 0.09)
        );
        assert!(wakes.iter().any(|e| e.position[1] < 0.0) && wakes.iter().any(|e| e.position[1] > 0.0));
        assert!(events.iter().all(|e| e.params[1] != 1.0));
    }

    #[test]
    fn an_arm_stroke_stirs_water_without_avatar_translation_and_deep_limbs_cannot_spray() {
        let (mut b, mut events) = (Bodies::default(), Vec::new());
        for frame in 0..=180 {
            let t = frame as f32 / 60.0;
            sample(
                &mut b,
                &mut events,
                6,
                volume(Vec3::new((t * 3.0).sin() * 0.5, 0.6, -0.02), Vec3::new(0.07, 0.1, 0.04)),
                t,
                WaterEffects::default(),
            );
            sample(
                &mut b,
                &mut events,
                14,
                volume(Vec3::new(0.0, 0.0, 0.5), Vec3::splat(0.15)),
                t,
                WaterEffects::default(),
            );
            sample(
                &mut b,
                &mut events,
                0,
                volume(Vec3::new(t, -0.2, -1.0), Vec3::splat(0.06)),
                t,
                WaterEffects::default(),
            );
        }
        assert!(events.iter().any(|e| e.params[1] == 4.0));
        assert!(
            events.iter().all(|e| (e.position[1] - 0.6).abs() < 0.001),
            "only the moving surface hand acts"
        );
    }

    #[test]
    fn head_entry_and_emergence_have_local_strikes_and_sparse_air_belongs_only_to_the_head() {
        let (mut b, mut events) = (Bodies::default(), Vec::new());
        for frame in 0..=600 {
            let t = frame as f32 / 60.0;
            sample(
                &mut b,
                &mut events,
                HEAD,
                volume(Vec3::new(1.3, -0.7, 0.1 + 0.3 * (t * 4.0).cos()), Vec3::new(0.11, 0.09, 0.12)),
                t,
                WaterEffects::default(),
            );
        }
        let hits: Vec<_> = events.iter().filter(|e| e.params[1] == 8.0).collect();
        assert!(hits.iter().any(|e| e.motion[2] < -0.35));
        assert!(hits.iter().any(|e| e.motion[2] > 0.35));
        assert!(hits.iter().all(|e| (e.position[0] - 1.3).abs() < 0.001 && e.params[0] < 0.15));
        for quality in [0, 3] {
            let (mut b, mut events) = (Bodies::default(), Vec::new());
            let settings = WaterEffects {
                quality,
                ..Default::default()
            };
            for frame in 0..=500 {
                let t = frame as f32 * 0.02;
                for part in 0..PARTS.len() {
                    sample(
                        &mut b,
                        &mut events,
                        part,
                        volume(Vec3::new(t * 1.2, part as f32, -1.0), Vec3::splat(0.1)),
                        t,
                        settings,
                    );
                }
            }
            assert!((6..=8).contains(&events.len()), "only sparse head air, {} packets", events.len());
            assert!(events.iter().all(|e| e.params[1] == 1.0 && e.position[1] == HEAD as f32));
            assert!(events.windows(2).all(|p| p[1].position[3] - p[0].position[3] >= 1.249));
        }
    }

    #[test]
    fn anatomical_contacts_reject_teleports_idle_motion_and_preserve_particle_independence() {
        let (mut b, mut events) = (Bodies::default(), Vec::new());
        let settings = WaterEffects::default();
        for frame in 0..120 {
            sample(
                &mut b,
                &mut events,
                HEAD,
                volume(Vec3::new(0.0, 0.0, -1.0), Vec3::splat(0.1)),
                frame as f32 / 60.0,
                settings,
            );
        }
        assert!(events.is_empty());
        sample(
            &mut b,
            &mut events,
            HEAD,
            volume(Vec3::new(100.0, 0.0, -1.0), Vec3::splat(0.1)),
            2.0,
            settings,
        );
        assert!(events.is_empty());
        let (mut a, mut b, mut ea, mut eb) = (Bodies::default(), Bodies::default(), Vec::new(), Vec::new());
        for frame in 0..=300 {
            let t = frame as f32 / 60.0;
            for part in [0, 6, HEAD] {
                let m = volume(Vec3::new(t, part as f32, -0.02 + (t * 8.0).sin() * 0.15), Vec3::splat(0.08));
                sample(&mut a, &mut ea, part, m, t, settings);
                sample(
                    &mut b,
                    &mut eb,
                    part,
                    m,
                    t,
                    WaterEffects {
                        bubbles: false,
                        splashes: false,
                        ..settings
                    },
                );
            }
        }
        let displacement = |events: &[WaterEvent]| {
            events
                .iter()
                .filter(|e| matches!(e.params[1], 4.0 | 8.0))
                .map(|e| (e.position, e.motion, e.params))
                .collect::<Vec<_>>()
        };
        assert_eq!(displacement(&ea), displacement(&eb));
        assert!(ea.iter().any(|e| e.params[1] == 7.0));
        assert!(eb.iter().all(|e| matches!(e.params[1], 4.0 | 8.0)));
    }

    #[test]
    fn anatomical_swept_paths_keep_turns_at_different_frame_rates_and_stop_emitting_when_idle() {
        for fps in [30, 60, 144] {
            let (mut b, mut events) = (Bodies::default(), Vec::new());
            let settings = WaterEffects {
                splashes: false,
                bubbles: false,
                ..Default::default()
            };
            for frame in 0..=fps * 3 {
                let t = frame as f32 / fps as f32;
                let p = if t <= 1.0 {
                    Vec3::X * t * 2.0
                } else {
                    Vec3::new(2.0, (t.min(2.0) - 1.0) * 2.0, 0.0)
                };
                sample(&mut b, &mut events, 14, volume(p, Vec3::splat(0.15)), t, settings);
            }
            let length: f32 = events.iter().map(|e| e.params[3]).sum();
            assert!((3.65..4.01).contains(&length), "{fps} fps: displaced length {length}");
            assert!(events.iter().all(|e| e.params[1] == 4.0 && e.position[3] <= 2.2));
            assert!(events.iter().any(|e| e.motion[0] > 1.9) && events.iter().any(|e| e.motion[1] > 1.9));
        }
    }

    #[test]
    fn anatomical_fast_vertical_crossings_hit_and_exit_without_a_global_horizontal_wake() {
        let (mut bodies, mut events) = (Bodies::default(), Vec::new());
        let settings = WaterEffects::default();
        sample(
            &mut bodies,
            &mut events,
            6,
            volume(Vec3::new(0.4, 0.6, 0.3), Vec3::splat(0.05)),
            0.0,
            settings,
        );
        sample(
            &mut bodies,
            &mut events,
            6,
            volume(Vec3::new(0.4, 0.6, -0.3), Vec3::splat(0.05)),
            0.05,
            settings,
        );
        assert!(
            events.iter().any(|e| e.params[1] == 8.0 && e.motion[2] < 0.0),
            "the small hand crossed entirely between frames"
        );
        assert!(events.iter().all(|e| e.params[1] != 4.0));
        sample(
            &mut bodies,
            &mut events,
            6,
            volume(Vec3::new(0.4, 0.6, -0.3), Vec3::splat(0.05)),
            0.20,
            settings,
        );
        sample(
            &mut bodies,
            &mut events,
            6,
            volume(Vec3::new(0.4, 0.6, -0.3), Vec3::splat(0.05)),
            0.35,
            settings,
        );
        sample(
            &mut bodies,
            &mut events,
            6,
            volume(Vec3::new(0.4, 0.6, 0.3), Vec3::splat(0.05)),
            0.40,
            settings,
        );
        assert!(
            events.iter().any(|e| e.params[1] == 8.0 && e.motion[2] > 0.0),
            "fast emergence is swept too"
        );
        assert!(
            events
                .iter()
                .all(|e| (e.position[0] - 0.4).abs() < 0.001 && (e.position[1] - 0.6).abs() < 0.001)
        );
    }

    #[test]
    fn anatomical_visibility_resolves_fingers_feet_and_head_without_visible_ancestors() {
        let lib = super::super::avatar::AvatarLibrary::load();
        let masks = bone_masks(&lib.rig);
        assert_eq!(masks[lib.rig.names["mFootLeft"]], 1);
        assert_eq!(masks[lib.rig.names["mHandIndex3Left"]], 1 << 6);
        assert_eq!(masks[lib.rig.names["mHead"]], 1 << HEAD);
        assert!(PARTS.iter().all(|name| lib.rig.names.contains_key(*name)));
        assert_eq!(system_mask("hair"), 0);
        assert_eq!(system_mask("skirt"), 0);
    }
}
