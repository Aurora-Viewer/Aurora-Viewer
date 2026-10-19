//! Terrain adaptation of the final animation pose.
//!
//! Authored clearance, terrain elevation and world-space body elevation are
//! separate quantities. No horizontal foot anchors or filtered joint angles.
//! Independently implemented concepts from ozz-animation's Foot IK sample
//! and Daniel Holden's "Inverse Kinematics and Foot Locking": rig-defined
//! knee hinges, contact phases and a separate body-height constraint.
//! https://guillaumeblanc.github.io/ozz-animation/samples/foot_ik/
//! https://www.theorangeduck.com/page/inverse-kinematics-foot-locking

use super::*;
use crate::ao::State;
use glam::Vec4;

const MARGIN: f32 = 0.001;
const ROOT_LIMIT: f32 = 0.45;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Surface {
    pub position: Vec3,
    pub normal: Vec3,
}

#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub to_world: Mat4,
    pub contact: Option<Vec4>,
    pub velocity_z: f32,
    pub sole_height: Option<f32>,
}

pub(super) fn unplanted(id: Uuid) -> bool {
    [
        State::Sitting,
        State::GroundSit,
        State::Jumping,
        State::Falling,
        State::Flying,
        State::FlyingSlow,
        State::FlyingUp,
        State::FlyingDown,
        State::Hovering,
    ]
    .iter()
    .any(|state| state.motion() == id)
}

pub(super) struct Pose<'a> {
    pub rotations: &'a [Quat],
    pub positions: &'a [Vec3],
    pub pelvis: Vec3,
    pub allowed: [bool; 2],
}

#[derive(Clone)]
pub(super) struct Placed {
    pub rotations: Vec<Quat>,
    pub pelvis: Vec3,
}

fn orientation(matrix: Mat4) -> Quat {
    matrix.to_scale_rotation_translation().1.normalize()
}

fn footprint(ankle: Vec3, rotation: Quat, sole: [Vec3; 4]) -> [Vec3; 4] {
    sole.map(|p| ankle + rotation * p)
}

fn correction(corners: [Vec3; 4], supports: [Option<Surface>; 4]) -> Option<f32> {
    corners
        .into_iter()
        .zip(supports)
        .filter_map(|(p, hit)| {
            let hit = hit?;
            (hit.normal.z > 0.2).then(|| ((hit.position - p).dot(hit.normal) + MARGIN) / hit.normal.z)
        })
        .reduce(f32::max)
}

#[derive(Clone, Copy)]
struct Leg {
    joints: [usize; 3],
    sole: [Vec3; 4],
    ankle_height: f32,
}

struct Setup {
    base: SkeletonBase,
    floor: f32,
    reference: Option<f32>,
    legs: [Leg; 2],
}

impl Setup {
    fn new(rig: &Rig, base: &SkeletonBase, reference: Option<f32>) -> Option<Self> {
        let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
        let floor = reference.unwrap_or(rest[*rig.names.get("mFootLeft")?].w_axis.z);
        let mut legs = Vec::with_capacity(2);
        for side in ["Left", "Right"] {
            let [hip, knee, ankle, ball] = ["Hip", "Knee", "Ankle", "Foot"].map(|bone| rig.names.get(&format!("m{bone}{side}")).copied());
            let (hip, knee, ankle, ball) = (hip?, knee?, ankle?, ball?);
            if rig.parent[knee] != Some(hip) || rig.parent[ankle] != Some(knee) {
                return None;
            }
            let inverse = rest[ankle].inverse();
            let toe = rig.names.get(&format!("mToe{side}")).copied().unwrap_or(ball);
            let front = inverse
                .transform_point3(rest[toe].w_axis.truncate())
                .x
                .max(inverse.transform_point3(rest[ball].w_axis.truncate()).x)
                * base.scale[ankle].x;
            let depth = (rest[ankle].w_axis.z - floor).clamp(0.01, 0.5);
            let width = 0.045 * base.scale[ankle].y;
            legs.push(Leg {
                joints: [hip, knee, ankle],
                sole: [
                    Vec3::new(-0.04, -width, -depth),
                    Vec3::new(-0.04, width, -depth),
                    Vec3::new(front, -width, -depth),
                    Vec3::new(front, width, -depth),
                ],
                ankle_height: rest[ankle].w_axis.z - floor,
            });
        }
        Some(Self {
            base: base.clone(),
            floor,
            reference,
            legs: [legs[0], legs[1]],
        })
    }
}

#[derive(Default)]
struct Foot {
    contact: bool,
    departure: Option<f32>,
    peak: f32,
    landing: f32,
    normal: Vec3,
}

impl Foot {
    fn phase(&mut self, height: f32, floor: Option<f32>) -> Option<f32> {
        let was_contact = self.contact;
        self.contact = floor.is_some() && if was_contact { height < 0.0005 } else { height < 0.00025 };
        if self.contact {
            self.departure = floor;
            self.peak = 0.0;
            self.landing = 0.0;
        } else {
            if was_contact || self.departure.is_none() {
                if self.departure.is_none() {
                    self.departure = floor;
                }
                self.peak = height.max(0.0);
                self.landing = 0.0;
            }
            self.peak = self.peak.max(height);
        }
        // Follow the authored flight from its departure floor. Descent alone
        // transitions to the next support, without flattening the flight arc.
        if self.peak > 0.01 {
            self.landing = self.landing.max(cubic_step(1.0 - height.max(0.0) / self.peak));
        }
        self.departure.map(|from| from + (floor.unwrap_or(from) - from) * self.landing)
    }
}

#[derive(Clone, Copy)]
struct Height {
    value: f32,
    velocity: f32,
}

impl Height {
    fn follow(&mut self, goal: f32, dt: f32) {
        let frequency = 24.0;
        let error = self.value - goal;
        let c = self.velocity + frequency * error;
        let decay = (-frequency * dt).exp();
        self.value = goal + (error + c * dt) * decay;
        self.velocity = (self.velocity - frequency * c * dt) * decay;
    }

    fn constrain(&mut self, lower: f32, upper: f32) {
        if lower <= upper {
            let bounded = self.value.clamp(lower, upper);
            if bounded != self.value {
                self.value = bounded;
                self.velocity = 0.0;
            }
        }
    }
}

/// Knee local Y defines flexion, including a straight source leg. Inferring
/// a pole from nearly collinear positions amplifies the rest leg's taper.
fn hinge(hip: Vec3, knee: Vec3, ankle: Vec3, target: Vec3, side: Vec3) -> Option<(Quat, Quat)> {
    let upper = knee - hip;
    let lower = ankle - knee;
    let (a, b) = (upper.length(), lower.length());
    let delta = target - hip;
    let distance = delta.length();
    if a < 1e-5 || b < 1e-5 || distance < 1e-5 || !a.is_finite() || !b.is_finite() || !distance.is_finite() {
        return None;
    }
    let source_axis = (ankle - hip).try_normalize()?;
    let forward = source_axis.cross(side).try_normalize()?;
    let lateral = source_axis.cross(forward);
    let axis = delta / distance;
    let transport = Quat::from_rotation_arc(source_axis, axis);
    let reach = distance.clamp((a - b).abs() + 1e-6, a + b - 1e-6);
    let along = (a * a - b * b + reach * reach) / (2.0 * reach);
    let radius = (a * a - along * along).max(0.0).sqrt();
    // Keep lateral taper in metres, without amplifying its direction. Only
    // forward flexion grows; tiny contact corrections retain authored bends.
    let bias = upper.dot(lateral).clamp(-radius, radius);
    let bend = hip + axis * along + transport * (lateral * bias + forward * (radius * radius - bias * bias).max(0.0).sqrt());
    let hip_delta = Quat::from_rotation_arc(upper / a, (bend - hip).normalize());
    let knee_delta = Quat::from_rotation_arc((hip_delta * lower).normalize(), (hip + axis * reach - bend).normalize());
    Some((hip_delta, knee_delta))
}

struct Release {
    at: Instant,
    rotations: Vec<Quat>,
    pelvis: Vec3,
}

#[derive(Default)]
pub(super) struct Placement {
    setup: Option<Setup>,
    feet: [Foot; 2],
    height: Option<Height>,
    previous_frame: Option<Frame>,
    sampled: Option<Instant>,
    held: Option<Placed>,
    deltas: Vec<Quat>,
    release: Option<Release>,
}

impl Placement {
    pub(super) fn resume(&mut self, duration: Duration) {
        self.sampled = self.sampled.and_then(|at| at.checked_add(duration));
        if let Some(release) = &mut self.release {
            release.at += duration;
        }
    }

    fn disable(&mut self, pose: &Pose<'_>, now: Instant) -> Option<Placed> {
        let held = self.held.as_ref()?;
        let release = self.release.get_or_insert_with(|| Release {
            at: now,
            rotations: self.deltas.clone(),
            pelvis: held.pelvis,
        });
        let weight = 1.0 - cubic_step(now.saturating_duration_since(release.at).as_secs_f32() / 0.2);
        if weight <= 0.0 {
            *self = Self::default();
            return None;
        }
        Some(Placed {
            rotations: pose
                .rotations
                .iter()
                .zip(&release.rotations)
                .map(|(&source, &delta)| (Quat::IDENTITY.slerp(delta, weight) * source).normalize())
                .collect(),
            pelvis: release.pelvis * weight,
        })
    }

    pub(super) fn apply(
        &mut self,
        rig: &Rig,
        base: &SkeletonBase,
        pose: Pose<'_>,
        frame: Option<Frame>,
        now: Instant,
        ground: &dyn Fn(Vec3) -> Option<Surface>,
    ) -> Option<Placed> {
        if !pose.allowed.iter().any(|allowed| *allowed) {
            *self = Self::default();
            return None;
        }
        if self.sampled == Some(now) {
            return self.held.clone();
        }
        let Some(frame) = frame else {
            let result = self.disable(&pose, now);
            self.sampled = Some(now);
            self.held = result.clone();
            return result;
        };
        let reference = frame.sole_height;
        let teleported = self
            .previous_frame
            .is_some_and(|old| old.to_world.w_axis.distance_squared(frame.to_world.w_axis) > 4.0);
        if self
            .setup
            .as_ref()
            .is_none_or(|setup| setup.base != *base || setup.reference != reference)
            || teleported
            || self.release.is_some()
        {
            *self = Self {
                setup: Some(Setup::new(rig, base, reference)?),
                ..Default::default()
            };
        }
        let dt = self
            .sampled
            .map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f32())
            .min(0.1);
        self.sampled = Some(now);
        let previous_frame = self.previous_frame.replace(frame);
        let setup = self.setup.as_ref()?;
        let source = rig.world_with(pose.rotations, pose.pelvis, pose.positions, &base.scale);
        // Contact thresholds are submillimetre. Measure before adding the
        // large render-space translation, avoiding loss of source clearance.
        let clearances = setup.legs.map(|leg| {
            let matrix = source[leg.joints[2]];
            footprint(matrix.w_axis.truncate(), orientation(matrix), leg.sole)
                .map(|point| frame.to_world.transform_vector3(point - Vec3::Z * setup.floor).z)
                .into_iter()
                .fold(f32::INFINITY, f32::min)
        });
        let source: Vec<Mat4> = source.into_iter().map(|matrix| frame.to_world * matrix).collect();
        let reference_floor = frame.to_world.transform_point3(Vec3::Z * setup.floor).z;
        let inverse_frame = frame.to_world.inverse();
        let query = |point: Vec3| ground(point).filter(|hit| hit.normal.z > 0.2 && hit.position.distance(point) < 0.75);
        let mut targets = [Vec3::ZERO; 2];
        let mut rotations = [Quat::IDENTITY; 2];
        let mut supported = [false; 2];
        let mut goal = None::<f32>;
        for (side, leg) in setup.legs.iter().enumerate() {
            let ankle = source[leg.joints[2]].w_axis.truncate();
            let authored = orientation(source[leg.joints[2]]);
            targets[side] = ankle;
            rotations[side] = authored;
            if !pose.allowed[side] {
                self.feet[side] = Foot::default();
                continue;
            }
            let corners = footprint(ankle, authored, leg.sole);
            let height = clearances[side];
            let hits = corners.map(query);
            supported[side] = hits.iter().any(Option::is_some);
            let normal = hits
                .iter()
                .flatten()
                .max_by(|a, b| a.position.z.total_cmp(&b.position.z))
                .map_or(Vec3::Z, |hit| hit.normal);
            let foot = &mut self.feet[side];
            foot.normal = if foot.normal.length_squared() < 0.5 {
                normal
            } else {
                foot.normal.lerp(normal, 1.0 - (-20.0 * dt).exp()).normalize_or(Vec3::Z)
            };
            let flat = Quat::from_rotation_arc(authored * Vec3::Z, Vec3::Z) * authored;
            let slope = Quat::from_rotation_arc(Vec3::Z, foot.normal);
            let neutral_ankle = ankle.with_z(reference_floor + leg.ankle_height);
            let neutral_corners = footprint(neutral_ankle, slope * flat, leg.sole);
            let terrain_floor = correction(neutral_corners, hits).map(|lift| reference_floor + lift);
            let flight_floor = foot.phase(height, terrain_floor);
            if let Some(floor) = terrain_floor {
                goal = Some(goal.map_or(floor, |other| other.min(floor)));
                // Anticipate a lower support for the body, not a flight target.
                // Absolute height cancels collider-root jumps before filtering.
                if let Some(previous) = previous_frame.filter(|previous| dt > 1e-5 && previous.to_world != frame.to_world) {
                    let ahead = neutral_corners.map(|point| {
                        let previous = previous.to_world.transform_point3(inverse_frame.transform_point3(point));
                        point + (point - previous).with_z(0.0) * (0.22 / dt)
                    });
                    if let Some(lift) = correction(ahead, ahead.map(query)) {
                        let next = reference_floor + lift;
                        // A ledge is not a stair tread. Preparing a large
                        // drop while the heels still have support would
                        // crouch the body even if the avatar stops at its edge.
                        if floor - next <= 0.25 {
                            goal = goal.map(|value| value.min(next));
                        }
                    }
                }
            }
            let tilt_weight = 1.0 - cubic_step(height.max(0.0) / 0.06);
            rotations[side] = (Quat::IDENTITY.slerp(slope, tilt_weight) * authored).normalize();
            if foot.contact {
                targets[side].z += correction(footprint(ankle, rotations[side], leg.sole), hits).unwrap_or(0.0);
            } else if let Some(floor) = flight_floor {
                targets[side].z += floor - reference_floor;
            }
            if supported[side] {
                // Query the actual oriented footprint before deriving body
                // reach, so a toe crossing an edge is part of that constraint.
                let corners = footprint(targets[side], rotations[side], leg.sole);
                if let Some(lift) = correction(corners, corners.map(query)) {
                    targets[side].z += if foot.contact { lift } else { lift.max(0.0) };
                }
            }
        }
        let mut lower = reference_floor - ROOT_LIMIT;
        let mut upper = reference_floor + ROOT_LIMIT;
        for (side, leg) in setup.legs.iter().enumerate() {
            if !self.feet[side].contact {
                continue;
            }
            let [hip, knee, ankle] = leg.joints.map(|j| source[j].w_axis.truncate());
            let reach = hip.distance(knee) + knee.distance(ankle) - 0.0001;
            let delta = hip - targets[side];
            let vertical = (reach * reach - delta.truncate().length_squared()).max(0.0).sqrt();
            lower = lower.max(reference_floor - vertical - delta.z);
            upper = upper.min(reference_floor + vertical - delta.z);
        }
        let goal = goal
            .unwrap_or(reference_floor)
            .clamp(reference_floor - ROOT_LIMIT, reference_floor + ROOT_LIMIT);
        let height = self.height.get_or_insert(Height {
            value: goal,
            velocity: 0.0,
        });
        if pose.allowed.iter().all(|allowed| *allowed) {
            height.follow(goal, dt);
            height.constrain(lower, upper);
        } else {
            *height = Height {
                value: reference_floor,
                velocity: 0.0,
            };
        }
        let offset = Vec3::Z * (height.value - reference_floor);
        let mut result = Placed {
            rotations: pose.rotations.to_vec(),
            pelvis: inverse_frame.transform_vector3(offset),
        };
        for (side, leg) in setup.legs.iter().enumerate() {
            if !pose.allowed[side] {
                continue;
            }
            let [hip, knee, ankle] = leg.joints;
            if !supported[side] && self.feet[side].departure.is_none() {
                targets[side] += offset;
            }
            let p = |joint: usize| source[joint].w_axis.truncate() + offset;
            if p(ankle).distance_squared(targets[side]) < 1e-9 && orientation(source[ankle]).abs_diff_eq(rotations[side], 1e-5) {
                continue;
            }
            let Some((a, b)) = hinge(p(hip), p(knee), p(ankle), targets[side], orientation(source[knee]) * Vec3::Y) else {
                continue;
            };
            let hip_rotation = a * orientation(source[hip]);
            let knee_rotation = b * a * orientation(source[knee]);
            let parent = rig.parent[hip].map_or(orientation(frame.to_world), |j| orientation(source[j]));
            result.rotations[hip] = (parent.inverse() * hip_rotation).normalize();
            result.rotations[knee] = (hip_rotation.inverse() * knee_rotation).normalize();
            result.rotations[ankle] = (knee_rotation.inverse() * rotations[side]).normalize();
        }
        self.deltas = result
            .rotations
            .iter()
            .zip(pose.rotations)
            .map(|(&placed, &raw)| placed * raw.inverse())
            .collect();
        log::trace!(target: "aurora_viewer::foot_placement", "terrain body={:.5} reference={reference_floor:.5} contact={:?} targets={targets:?}",
            height.value, self.feet.each_ref().map(|foot| foot.contact));
        self.held = Some(result.clone());
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        rig: Arc<Rig>,
        base: SkeletonBase,
        floor: f32,
    }

    impl Fixture {
        fn new() -> Self {
            let library = crate::scene::avatar::AvatarLibrary::load();
            let rig = library.rig.clone();
            let base = SkeletonBase {
                local_pos: rig.local_pos.clone(),
                scale: rig.scale.clone(),
            };
            let floor = Setup::new(&rig, &base, None).expect("humanoid").floor;
            Self { rig, base, floor }
        }

        fn frame(&self, position: Vec3, yaw: f32) -> Frame {
            Frame {
                to_world: Mat4::from_rotation_translation(Quat::from_rotation_z(yaw), position - Vec3::Z * self.floor),
                contact: None,
                velocity_z: 0.0,
                sole_height: None,
            }
        }

        fn sample(
            &self,
            placement: &mut Placement,
            rotations: &[Quat],
            pelvis: Vec3,
            frame: Option<Frame>,
            at: Instant,
            ground: &dyn Fn(Vec3) -> Option<Surface>,
        ) -> Placed {
            placement
                .apply(
                    &self.rig,
                    &self.base,
                    Pose {
                        rotations,
                        positions: &self.base.local_pos,
                        pelvis,
                        allowed: [true; 2],
                    },
                    frame,
                    at,
                    ground,
                )
                .unwrap_or(Placed {
                    rotations: rotations.to_vec(),
                    pelvis: Vec3::ZERO,
                })
        }

        fn check(
            &self,
            placement: &Placement,
            placed: &Placed,
            raw: &[Quat],
            pelvis: Vec3,
            frame: Frame,
            ground: &dyn Fn(Vec3) -> Option<Surface>,
        ) -> Vec<Mat4> {
            let source = self.rig.world_with(raw, pelvis, &self.base.local_pos, &self.base.scale);
            let output = self
                .rig
                .world_with(&placed.rotations, pelvis + placed.pelvis, &self.base.local_pos, &self.base.scale);
            for (side, leg) in placement.setup.as_ref().expect("setup").legs.iter().enumerate() {
                let [hip, knee, ankle] = leg.joints;
                for (a, b) in [(hip, knee), (knee, ankle)] {
                    assert!(
                        (output[a].w_axis.distance(output[b].w_axis) - source[a].w_axis.distance(source[b].w_axis)).abs() < 1e-4,
                        "a leg segment stretched"
                    );
                }
                assert!(
                    (output[ankle].w_axis - source[ankle].w_axis).truncate().truncate().length() < 0.0002,
                    "IK changed the authored lateral trajectory"
                );
                let matrix = frame.to_world * output[ankle];
                let mut nearest = f32::INFINITY;
                for point in footprint(matrix.w_axis.truncate(), orientation(matrix), leg.sole) {
                    if let Some(hit) = ground(point) {
                        let distance = (point - hit.position).dot(hit.normal);
                        assert!(distance > -0.0015, "foot {side} penetrates by {distance}");
                        nearest = nearest.min(distance);
                    }
                }
                if placement.feet[side].contact {
                    assert!(nearest < 0.004, "contact foot {side} floats by {nearest}");
                }
                assert!(
                    output[knee].w_axis.y * if side == 0 { 1.0 } else { -1.0 } > 0.01,
                    "knee {side} crossed the body's centre: {:?}",
                    output[knee].w_axis
                );
            }
            output.into_iter().map(|matrix| frame.to_world * matrix).collect()
        }
    }

    fn flat(p: Vec3) -> Option<Surface> {
        Some(Surface {
            position: p.with_z(0.0),
            normal: Vec3::Z,
        })
    }

    #[test]
    fn grounding_rewrite_contacts_and_knee_hinges_survive_unequal_supports_and_turning() {
        let f = Fixture::new();
        let now = Instant::now();
        for fps in [30, 60, 144] {
            for heights in [[0.0_f32, 0.22], [0.22, 0.0], [0.18, 0.18], [0.18, 0.30]] {
                let mut placement = Placement::default();
                let root = if heights[0].min(heights[1]) > 0.0 { 0.0 } else { 0.25 };
                for i in 0..=fps * 2 {
                    let frame = f.frame(Vec3::Z * root, i as f32 / fps as f32);
                    let inverse = frame.to_world.inverse();
                    let ground = |p: Vec3| {
                        Some(Surface {
                            position: p.with_z(heights[usize::from(inverse.transform_point3(p).y < 0.0)]),
                            normal: Vec3::Z,
                        })
                    };
                    let placed = f.sample(
                        &mut placement,
                        &f.rig.local_rot,
                        Vec3::ZERO,
                        Some(frame),
                        now + Duration::from_secs_f32(i as f32 / fps as f32),
                        &ground,
                    );
                    f.check(&placement, &placed, &f.rig.local_rot, Vec3::ZERO, frame, &ground);
                    if i == fps * 2 {
                        let expected = heights[0].min(heights[1]) + MARGIN;
                        assert!(
                            (root + placed.pelvis.z - expected).abs() < 0.005,
                            "body failed to follow common support"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn grounding_rewrite_filters_world_height_across_a_physical_root_step() {
        let f = Fixture::new();
        let now = Instant::now();
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            let mut previous = None::<f32>;
            for i in 0..=fps * 3 {
                let level = if i < fps { 0.0 } else { 0.22 };
                let frame = f.frame(Vec3::Z * level, 0.0);
                let ground = |p: Vec3| {
                    Some(Surface {
                        position: p.with_z(level),
                        normal: Vec3::Z,
                    })
                };
                let placed = f.sample(
                    &mut placement,
                    &f.rig.local_rot,
                    Vec3::ZERO,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / fps as f32),
                    &ground,
                );
                let world = f.check(&placement, &placed, &f.rig.local_rot, Vec3::ZERO, frame, &ground);
                let body = world[f.rig.pelvis].w_axis.z;
                if let Some(previous) = previous {
                    assert!(
                        (body - previous).abs() < 2.5 / fps as f32,
                        "collider root popped the rendered body at {fps} fps"
                    );
                }
                previous = Some(body);
            }
        }
    }

    #[test]
    fn grounding_rewrite_movement_down_a_step_keeps_contact_and_limits_body_speed() {
        let f = Fixture::new();
        let now = Instant::now();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(if p.x < 0.0 { 0.22 } else { 0.0 }),
                normal: Vec3::Z,
            })
        };
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            let mut previous = None::<f32>;
            for i in 0..=fps * 4 {
                let x = -0.45 + (i as f32 / fps as f32 - 1.0).max(0.0) * 0.5;
                let frame = f.frame(Vec3::new(x, 0.0, if x < 0.0 { 0.22 } else { 0.0 }), 0.0);
                let placed = f.sample(
                    &mut placement,
                    &f.rig.local_rot,
                    Vec3::ZERO,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / fps as f32),
                    &ground,
                );
                let world = f.check(&placement, &placed, &f.rig.local_rot, Vec3::ZERO, frame, &ground);
                let body = world[f.rig.pelvis].w_axis.z;
                if i > fps
                    && let Some(previous) = previous
                {
                    assert!(
                        (body - previous).abs() < 2.5 / fps as f32,
                        "body snapped down at the step edge at {fps} fps: {previous} -> {body}"
                    );
                }
                previous = Some(body);
            }
        }
    }

    #[test]
    fn grounding_rewrite_alternating_flight_keeps_source_clearance_without_body_pumping() {
        let f = Fixture::new();
        let clip = crate::demo::grounding::lifting_animation();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(if p.y > 0.0 { 0.22 } else { 0.0 }),
                normal: Vec3::Z,
            })
        };
        let now = Instant::now();
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            let frame = f.frame(Vec3::Z * 0.28, 0.0);
            let mut previous = None::<f32>;
            for i in 0..=fps * 5 {
                let t = (i as f32 / fps as f32 - 1.0).max(0.0);
                let mut raw = f.rig.local_rot.clone();
                for joint in &clip.joints {
                    if let (Some(&j), Some(r)) = (f.rig.names.get(&joint.joint_name), joint.rotation_at_loop(t, clip.loop_range())) {
                        raw[j] = r;
                    }
                }
                let placed = f.sample(
                    &mut placement,
                    &raw,
                    Vec3::ZERO,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / fps as f32),
                    &ground,
                );
                let world = f.check(&placement, &placed, &raw, Vec3::ZERO, frame, &ground);
                if i > fps
                    && let Some(previous) = previous
                {
                    assert!(
                        (placed.pelvis.z - previous).abs() < 0.3 / fps as f32,
                        "body pumped with an authored foot lift"
                    );
                }
                previous = Some(placed.pelvis.z);
                if i == fps * 2 || i == fps * 4 {
                    let side = usize::from(i == fps * 4);
                    assert!(!placement.feet[side].contact);
                    let leg = placement.setup.as_ref().unwrap().legs[side];
                    let ankle = leg.joints[2];
                    let gap = footprint(world[ankle].w_axis.truncate(), orientation(world[ankle]), leg.sole)
                        .map(|p| p.z - ground(p).unwrap().position.z)
                        .into_iter()
                        .fold(f32::INFINITY, f32::min);
                    assert!(gap > 0.09, "flight foot {side} was pinned to the ground: {gap}");
                }
            }
        }
    }

    #[test]
    fn grounding_rewrite_preserves_a_contact_crouch_and_authored_ankle_roll() {
        let f = Fixture::new();
        let now = Instant::now();
        let frame = f.frame(Vec3::ZERO, 0.0);
        for angle in [0.2, 0.45, 0.7] {
            let mut raw = f.rig.local_rot.clone();
            for side in ["Left", "Right"] {
                for (bone, angle) in [("Hip", -angle), ("Knee", angle * 2.0), ("Ankle", -angle)] {
                    raw[f.rig.names[&format!("m{bone}{side}")]] = Quat::from_rotation_y(angle);
                }
            }
            let source = f.rig.world_with(&raw, Vec3::ZERO, &f.base.local_pos, &f.base.scale);
            let pelvis = Vec3::Z * (f.floor - source[f.rig.names["mFootLeft"]].w_axis.z);
            let mut placement = Placement::default();
            for i in 0..=120 {
                let placed = f.sample(
                    &mut placement,
                    &raw,
                    pelvis,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / 60.0),
                    &flat,
                );
                f.check(&placement, &placed, &raw, pelvis, frame, &flat);
                if i == 120 {
                    assert!(placed.pelvis.z.abs() < 0.005, "body correction erased an authored crouch");
                    for name in ["mHipLeft", "mHipRight", "mKneeLeft", "mKneeRight"] {
                        assert!(
                            placed.rotations[f.rig.names[name]].angle_between(raw[f.rig.names[name]]).abs() < 0.015,
                            "authored {angle} radian flexion was replaced"
                        );
                    }
                }
            }
        }
        let mut raw = f.rig.local_rot.clone();
        raw[f.rig.names["mAnkleLeft"]] = Quat::from_rotation_y(-0.2);
        let mut placement = Placement::default();
        let placed = f.sample(&mut placement, &raw, Vec3::ZERO, Some(frame), now, &flat);
        let world = f.check(&placement, &placed, &raw, Vec3::ZERO, frame, &flat);
        assert!(
            orientation(world[f.rig.names["mAnkleLeft"]])
                .angle_between(Quat::from_rotation_y(-0.2))
                .abs()
                < 0.001
        );
    }

    #[test]
    fn grounding_rewrite_footprint_and_shoe_calibration_follow_slopes() {
        let f = Fixture::new();
        let normal = Vec3::new(-0.15, 0.25, 1.0).normalize();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(-(p.x * normal.x + p.y * normal.y) / normal.z),
                normal,
            })
        };
        let now = Instant::now();
        let mut placement = Placement::default();
        for i in 0..=120 {
            let frame = f.frame(Vec3::Z * 0.12, 0.0);
            let placed = f.sample(
                &mut placement,
                &f.rig.local_rot,
                Vec3::ZERO,
                Some(frame),
                now + Duration::from_secs_f32(i as f32 / 60.0),
                &ground,
            );
            f.check(&placement, &placed, &f.rig.local_rot, Vec3::ZERO, frame, &ground);
        }
        let mut frame = f.frame(Vec3::Z * 0.12, 0.0);
        frame.sole_height = Some(f.floor - 0.12);
        let mut placement = Placement::default();
        let placed = f.sample(&mut placement, &f.rig.local_rot, Vec3::ZERO, Some(frame), now, &flat);
        assert!(placed.pelvis.z.abs() < 0.005, "IK cancelled shoe hover calibration");
        f.check(&placement, &placed, &f.rig.local_rot, Vec3::ZERO, frame, &flat);
    }

    #[test]
    fn grounding_rewrite_disable_pause_and_teleport_release_only_our_corrections() {
        let f = Fixture::new();
        let now = Instant::now();
        let frame = f.frame(Vec3::Z * 0.2, 0.0);
        let mut placement = Placement::default();
        let placed = f.sample(&mut placement, &f.rig.local_rot, Vec3::ZERO, Some(frame), now, &flat);
        placement.resume(Duration::from_secs(10));
        let held = f.sample(
            &mut placement,
            &f.rig.local_rot,
            Vec3::ZERO,
            Some(frame),
            now + Duration::from_secs(10),
            &flat,
        );
        assert_eq!(placed.pelvis, held.pelvis);
        let no_query = |_: Vec3| -> Option<Surface> { panic!("disabled placement queried support") };
        let mut last = 1.0;
        for i in 1..=30 {
            let placed = f.sample(
                &mut placement,
                &f.rig.local_rot,
                Vec3::ZERO,
                None,
                now + Duration::from_secs(10) + Duration::from_secs_f32(i as f32 / 60.0),
                &no_query,
            );
            assert!(placed.pelvis.length() <= last + 1e-5);
            last = placed.pelvis.length();
        }
        assert!(last < 1e-5);
        let frame = f.frame(Vec3::new(100.0, 0.0, 0.0), 0.0);
        let placed = f.sample(
            &mut placement,
            &f.rig.local_rot,
            Vec3::ZERO,
            Some(frame),
            now + Duration::from_secs(11),
            &flat,
        );
        f.check(&placement, &placed, &f.rig.local_rot, Vec3::ZERO, frame, &flat);
        assert!(placed.pelvis.z.abs() < 0.005, "teleport retained old world elevation");
    }

    #[test]
    fn grounding_rewrite_does_not_replace_valid_walk_or_recovery_on_flat_ground() {
        let f = Fixture::new();
        let now = Instant::now();
        for recovery in [false, true] {
            let clip = crate::demo::grounding::contact_animation(&f.rig, &f.base, recovery);
            let mut placement = Placement::default();
            let frame = f.frame(Vec3::ZERO, 0.0);
            for i in 0..=432 {
                let t = if i < 144 { 0.0 } else { (i - 144) as f32 / 144.0 };
                let mut raw = f.rig.local_rot.clone();
                let mut pelvis = Vec3::ZERO;
                for joint in &clip.joints {
                    if let Some(&j) = f.rig.names.get(&joint.joint_name) {
                        if let Some(rotation) = joint.rotation_at_loop(t, clip.loop_range()) {
                            raw[j] = rotation;
                        }
                        if j == f.rig.pelvis
                            && let Some(position) = joint.position_at_loop(t, clip.loop_range())
                        {
                            pelvis = position;
                        }
                    }
                }
                let placed = f.sample(
                    &mut placement,
                    &raw,
                    pelvis,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / 144.0),
                    &flat,
                );
                f.check(&placement, &placed, &raw, pelvis, frame, &flat);
                if i > 144 {
                    assert!(
                        placement.feet.iter().any(|foot| foot.contact),
                        "the source walk has no true stance foot"
                    );
                    assert!(placed.pelvis.z.abs() < 0.003, "valid source body height was replaced");
                    for side in ["Left", "Right"] {
                        for bone in ["Hip", "Knee", "Ankle"] {
                            let j = f.rig.names[&format!("m{bone}{side}")];
                            assert!(
                                placed.rotations[j].angle_between(raw[j]).abs() < 0.02,
                                "valid source {bone}{side} was changed at {t}: {:?} -> {:?}",
                                raw[j],
                                placed.rotations[j]
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn grounding_rewrite_neutral_noise_does_not_reverse_a_compressed_knee() {
        let f = Fixture::new();
        let now = Instant::now();
        let frame = f.frame(Vec3::Z * 0.25, 0.0);
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(if p.y > 0.0 { 0.22 } else { 0.0 }),
                normal: Vec3::Z,
            })
        };
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            let mut previous = None::<Vec3>;
            for i in 0..=fps * 2 {
                let mut raw = f.rig.local_rot.clone();
                raw[f.rig.names["mKneeLeft"]] = Quat::from_rotation_y(0.008 * (i as f32 / fps as f32 * 12.0).sin());
                let placed = f.sample(
                    &mut placement,
                    &raw,
                    Vec3::ZERO,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / fps as f32),
                    &ground,
                );
                let world = f.check(&placement, &placed, &raw, Vec3::ZERO, frame, &ground);
                let knee = world[f.rig.names["mKneeLeft"]].w_axis.truncate();
                let hip = world[f.rig.names["mHipLeft"]].w_axis.truncate();
                assert!(knee.x - hip.x > 0.05, "a near-neutral source reversed the compressed knee");
                if let Some(previous) = previous {
                    assert!(knee.distance(previous) < 0.015, "knee flipped at {fps} fps");
                }
                previous = Some(knee);
            }
        }
    }

    #[test]
    fn grounding_rewrite_a_slow_lift_releases_in_millimetres() {
        let f = Fixture::new();
        let rest = f.rig.world_with(&f.rig.local_rot, Vec3::ZERO, &f.base.local_pos, &f.base.scale);
        let [hip, knee, ankle] = ["mHipLeft", "mKneeLeft", "mAnkleLeft"].map(|name| f.rig.names[name]);
        let length = rest[hip].w_axis.distance(rest[knee].w_axis) + rest[knee].w_axis.distance(rest[ankle].w_axis);
        let frame = f.frame(Vec3::ZERO, 0.0);
        let now = Instant::now();
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            for i in 0..=fps * 5 {
                let height = i as f32 / fps as f32 * 0.002;
                let angle = (1.0 - height / length).acos();
                let mut raw = f.rig.local_rot.clone();
                raw[hip] = Quat::from_rotation_y(-angle);
                raw[knee] = Quat::from_rotation_y(2.0 * angle);
                raw[ankle] = Quat::from_rotation_y(-angle);
                let placed = f.sample(
                    &mut placement,
                    &raw,
                    Vec3::ZERO,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / fps as f32),
                    &flat,
                );
                let world = f.check(&placement, &placed, &raw, Vec3::ZERO, frame, &flat);
                if height > 0.005 {
                    assert!(!placement.feet[0].contact, "a slow authored lift stayed planted");
                    let leg = placement.setup.as_ref().unwrap().legs[0];
                    let clearance = footprint(world[ankle].w_axis.truncate(), orientation(world[ankle]), leg.sole)
                        .map(|p| p.z)
                        .into_iter()
                        .fold(f32::INFINITY, f32::min);
                    assert!(clearance > height * 0.7, "the slow lift was pinned: {clearance}");
                }
            }
        }
    }

    #[test]
    fn grounding_rewrite_landing_does_not_jump_back_when_the_descent_slows_or_pauses() {
        let mut foot = Foot::default();
        foot.phase(0.0, Some(0.22));
        let departure = foot.phase(0.12, Some(0.0)).expect("flight from higher step");
        assert!((departure - 0.22).abs() < 1e-6);
        let mut previous = departure;
        for height in [0.10, 0.08, 0.06, 0.04, 0.04, 0.0399, 0.0399, 0.02, 0.01, 0.003, 0.0] {
            let floor = foot.phase(height, Some(0.0)).expect("flight floor");
            assert!(floor <= previous + 1e-6, "a slowing descent jumped back to departure height");
            previous = floor;
        }
        assert!(previous.abs() < 1e-5);
    }

    #[test]
    fn grounding_rewrite_slow_turning_on_fixed_steps_keeps_world_height_continuous() {
        let f = Fixture::new();
        let now = Instant::now();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(if p.y > 0.0 { 0.22 } else { 0.0 }),
                normal: Vec3::Z,
            })
        };
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            let mut previous = None::<f32>;
            for i in 0..=fps * 16 {
                let frame = f.frame(Vec3::Z * 0.25, i as f32 / fps as f32 * std::f32::consts::TAU / 16.0);
                let placed = f.sample(
                    &mut placement,
                    &f.rig.local_rot,
                    Vec3::ZERO,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / fps as f32),
                    &ground,
                );
                let world = f.check(&placement, &placed, &f.rig.local_rot, Vec3::ZERO, frame, &ground);
                let body = world[f.rig.pelvis].w_axis.z;
                if let Some(previous) = previous {
                    assert!(
                        (body - previous).abs() < 2.5 / fps as f32,
                        "body snapped while turning at {fps} fps"
                    );
                }
                previous = Some(body);
            }
        }
    }

    #[test]
    fn grounding_rewrite_approaching_a_ledge_does_not_crouch_before_leaving_support() {
        let f = Fixture::new();
        let now = Instant::now();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(if p.x < 0.0 { 0.4 } else { 0.0 }),
                normal: Vec3::Z,
            })
        };
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            for i in 0..=fps * 3 {
                let x = (-0.6 + i as f32 / fps as f32 * 0.5).min(-0.03);
                let frame = f.frame(Vec3::new(x, 0.0, 0.4), 0.0);
                let placed = f.sample(
                    &mut placement,
                    &f.rig.local_rot,
                    Vec3::ZERO,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / fps as f32),
                    &ground,
                );
                f.check(&placement, &placed, &f.rig.local_rot, Vec3::ZERO, frame, &ground);
                assert!(
                    placed.pelvis.z.abs() < 0.005,
                    "the body crouched in anticipation of a fall: {:?}",
                    placed.pelvis
                );
            }
        }
    }

    #[test]
    fn grounding_rewrite_source_gait_has_continuous_takeoff_and_landing_velocity() {
        let f = Fixture::new();
        let clip = crate::demo::grounding::contact_animation(&f.rig, &f.base, false);
        let ankle = f.rig.names["mAnkleLeft"];
        let sample = |time: f32| {
            let time = clip.local_time(f64::from(time)) as f32;
            let mut raw = f.rig.local_rot.clone();
            let mut pelvis = Vec3::ZERO;
            for joint in &clip.joints {
                if let Some(&j) = f.rig.names.get(&joint.joint_name) {
                    if let Some(rotation) = joint.rotation_at_loop(time, clip.loop_range()) {
                        raw[j] = rotation;
                    }
                    if j == f.rig.pelvis
                        && let Some(position) = joint.position_at_loop(time, clip.loop_range())
                    {
                        pelvis = position;
                    }
                }
            }
            f.rig.world_with(&raw, pelvis, &f.base.local_pos, &f.base.scale)[ankle]
                .w_axis
                .truncate()
        };
        let dt = 1.0 / 60.0;
        for at in [0.6, 1.0] {
            let before = (sample(at) - sample(at - dt)) / dt;
            let after = (sample(at + dt) - sample(at)) / dt;
            assert!(
                before.distance(after) < 0.4,
                "the source gait jerked at {at}: {before:?} -> {after:?}"
            );
        }
    }

    #[test]
    fn grounding_rewrite_honours_protected_poses_and_airborne_states() {
        let f = Fixture::new();
        let now = Instant::now();
        let no_query = |_: Vec3| -> Option<Surface> { panic!("protected pose queried terrain") };
        for (control, preview, priority) in [(false, false, 6), (true, false, 2), (false, true, 2)] {
            let mut clip = crate::demo::grounding::lifting_animation();
            clip.base_priority = priority;
            let anim = Arc::new(BoundAnim::bind(clip, &f.rig));
            let signal = PlayingAnimation {
                id: Uuid::from_u128(42),
                sequence: 1,
                sequence_start: now,
                continuous_start: now,
            };
            let mut controller = Controller {
                control,
                ground: Some(f.frame(Vec3::ZERO, 0.0)),
                ..Default::default()
            };
            if preview {
                controller.set_preview_clock(Some((Some(signal.id), now)));
            }
            controller.sync(&[signal], now, |_| Some(anim.clone()));
            controller.sync(&[signal], now + Duration::from_secs(1), |_| Some(anim.clone()));
            let mut output = vec![Mat4::IDENTITY.to_cols_array_2d(); f.rig.len()];
            controller.evaluate_on_ground(&f.rig, now + Duration::from_secs(1), Some(&f.base), &mut output, no_query);
        }
        for state in [
            State::Sitting,
            State::GroundSit,
            State::Jumping,
            State::Falling,
            State::FlyingSlow,
            State::Hovering,
        ] {
            assert!(unplanted(state.motion()));
        }
    }
}
