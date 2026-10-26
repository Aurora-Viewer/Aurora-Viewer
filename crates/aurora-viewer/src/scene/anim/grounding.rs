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
const TRANSITION_SECONDS: f32 = 0.5;

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
    pub max_height: f32,
    /// Our confirmed native ground-sit anchor is drawn with a verified stand.
    pub standing_anchor: bool,
}

pub(super) fn unplanted(id: Uuid, standing_anchor: bool) -> bool {
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
    .any(|state| state.motion() == id && (!standing_anchor || matches!(*state, State::Sitting | State::GroundSit)))
}

pub(super) struct Pose<'a> {
    pub rotations: &'a [Quat],
    pub positions: &'a [Vec3],
    pub pelvis: Vec3,
    /// Absolute pelvis tracks replace the shaped rest translation. Apply the
    /// same origin change to neutral clearance, without changing shoe depth
    /// or removing the animation's own pelvis motion.
    pub reference_offset: Vec3,
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
    elevation: Option<Height>,
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
        // About 98% settled by the requested transition duration, without
        // overshoot and with the same response at every display rate.
        let frequency = 6.0 / TRANSITION_SECONDS;
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

fn precise_arc(from: Vec3, to: Vec3) -> Quat {
    let dot = from.dot(to);
    if dot < -1.0 + 2.0 * f32::EPSILON {
        Quat::from_rotation_arc(from, to)
    } else {
        // Keep submillimetre retargeting rotations. The usual near-parallel
        // identity shortcut leaves the actual knee short of its solved point.
        let cross = from.cross(to);
        Quat::from_xyzw(cross.x, cross.y, cross.z, 1.0 + dot).normalize()
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
    let axis = delta / distance;
    let side = side.try_normalize()?;
    let lateral = side - axis * axis.dot(side);
    let side_reach = lateral.length();
    if side_reach < 1e-5 {
        return None;
    }
    let lateral = lateral / side_reach;
    let forward = axis.cross(lateral);
    let reach = distance.clamp((a - b).abs() + 1e-6, a + b - 1e-6);
    let along = (a * a - b * b + reach * reach) / (2.0 * reach);
    let radius = (a * a - along * along).max(0.0).sqrt();
    // Intersect the knee circle with the source hinge's lateral coordinate.
    // Transporting a pole toward a near-folded leg would rotate its forward
    // flexion medially; preserve the source knee's side offset in metres.
    let bias = ((upper.dot(side) - along * axis.dot(side)) / side_reach).clamp(-radius, radius);
    let bend = hip + axis * along + lateral * bias + forward * (radius * radius - bias * bias).max(0.0).sqrt();
    let hip_delta = precise_arc(upper / a, (bend - hip).normalize());
    let knee_delta = precise_arc((hip_delta * lower).normalize(), (hip + axis * reach - bend).normalize());
    Some((hip_delta, knee_delta))
}

struct Release {
    at: Instant,
    rotations: Vec<Quat>,
    pelvis: Vec3,
}

#[derive(Clone, Copy)]
struct Acquisition {
    at: Instant,
    rotations: [Quat; 2],
    penetration: [f32; 2],
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
    acquired: Option<Acquisition>,
}

impl Placement {
    pub(super) fn resume(&mut self, duration: Duration) {
        self.sampled = self.sampled.and_then(|at| at.checked_add(duration));
        if let Some(release) = &mut self.release {
            release.at += duration;
        }
        if let Some(acquired) = &mut self.acquired {
            acquired.at += duration;
        }
    }

    fn disable(&mut self, pose: &Pose<'_>, now: Instant) -> Option<Placed> {
        self.acquired = None;
        let held = self.held.as_ref()?;
        let release = self.release.get_or_insert_with(|| Release {
            at: now,
            rotations: self.deltas.clone(),
            pelvis: held.pelvis,
        });
        let weight = 1.0 - cubic_step(now.saturating_duration_since(release.at).as_secs_f32() / TRANSITION_SECONDS);
        if weight <= 0.0 {
            // Retain the zero-weight release as the re-enable seed. A new
            // contact after a completed fade must also start at the source.
            self.feet = Default::default();
            self.height = None;
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
        let Some(frame) = frame.filter(|frame| crate::settings::foot_height_limit(frame.max_height) > 0.0) else {
            let result = self.disable(&pose, now);
            self.sampled = Some(now);
            self.held = result.clone();
            return result;
        };
        let from_source = self.setup.is_none() && self.sampled.is_some();
        let reference = frame.sole_height;
        let max_height = crate::settings::foot_height_limit(frame.max_height);
        let teleported = self
            .previous_frame
            .is_some_and(|old| old.to_world.w_axis.distance_squared(frame.to_world.w_axis) > 4.0);
        let reset = self
            .setup
            .as_ref()
            .is_none_or(|setup| setup.base != *base || setup.reference != reference)
            || (teleported && self.release.is_none());
        // Capture the visible correction before rebuilding a changed sole or
        // shape reference. Calibration must not discard an in-progress fade.
        let seed = self
            .release
            .take()
            .map(|release| {
                let weight = 1.0 - cubic_step(now.saturating_duration_since(release.at).as_secs_f32() / TRANSITION_SECONDS);
                Placed {
                    rotations: pose
                        .rotations
                        .iter()
                        .zip(&release.rotations)
                        .map(|(&raw, &delta)| (Quat::IDENTITY.slerp(delta, weight) * raw).normalize())
                        .collect(),
                    pelvis: release.pelvis * weight,
                }
            })
            .or_else(|| {
                (from_source || (reset && self.sampled.is_some() && !teleported)).then(|| Placed {
                    rotations: pose
                        .rotations
                        .iter()
                        .enumerate()
                        .map(|(joint, &raw)| (self.deltas.get(joint).copied().unwrap_or(Quat::IDENTITY) * raw).normalize())
                        .collect(),
                    pelvis: self.held.as_ref().map_or(Vec3::ZERO, |held| held.pelvis),
                })
            });
        if reset {
            *self = Self {
                setup: Some(Setup::new(rig, base, reference)?),
                ..Default::default()
            };
        }
        let dt = if seed.is_some() {
            0.0
        } else {
            self.sampled
                .map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f32())
                .min(0.1)
        };
        self.sampled = Some(now);
        let previous_frame = self.previous_frame.replace(frame);
        let setup = self.setup.as_ref()?;
        let source = rig.world_with(pose.rotations, pose.pelvis, pose.positions, &base.scale);
        // Contact thresholds are submillimetre. Measure before adding the
        // large render-space translation, avoiding loss of source clearance.
        let clearances = setup.legs.map(|leg| {
            let matrix = source[leg.joints[2]];
            footprint(matrix.w_axis.truncate(), orientation(matrix), leg.sole)
                .map(|point| {
                    frame
                        .to_world
                        .transform_vector3(point - Vec3::Z * setup.floor - pose.reference_offset)
                        .z
                })
                .into_iter()
                .fold(f32::INFINITY, f32::min)
        });
        let source: Vec<Mat4> = source.into_iter().map(|matrix| frame.to_world * matrix).collect();
        let reference_floor = frame.to_world.transform_point3(Vec3::Z * setup.floor + pose.reference_offset).z;
        let inverse_frame = frame.to_world.inverse();
        let query = |point: Vec3| {
            ground(point)
                .filter(|hit| hit.normal.z > 0.2 && hit.position.distance(point) <= crate::settings::MAX_GROUND_FEET_HEIGHT + MARGIN * 2.0)
        };
        if let Some(seed) = seed {
            let visible = rig.world_with(&seed.rotations, pose.pelvis + seed.pelvis, pose.positions, &base.scale);
            let mut rotations = [Quat::IDENTITY; 2];
            let mut penetration = [0.0; 2];
            self.feet = Default::default();
            for (side, leg) in setup.legs.iter().enumerate() {
                let ankle = leg.joints[2];
                let visible = frame.to_world * visible[ankle];
                self.feet[side].elevation = Some(Height {
                    value: reference_floor + visible.w_axis.z - source[ankle].w_axis.z,
                    velocity: 0.0,
                });
                rotations[side] = orientation(visible) * orientation(source[ankle]).inverse();
                self.feet[side].normal = rotations[side] * Vec3::Z;
                let corners = footprint(visible.w_axis.truncate(), orientation(visible), leg.sole);
                penetration[side] = correction(corners, corners.map(query)).unwrap_or(0.0).max(0.0);
            }
            self.height = Some(Height {
                value: reference_floor + frame.to_world.transform_vector3(seed.pelvis).z,
                velocity: 0.0,
            });
            self.acquired = Some(Acquisition {
                at: now,
                rotations,
                penetration,
            });
        }
        let acquisition = self.acquired;
        let acquisition_weight = acquisition.map_or(1.0, |entry| {
            cubic_step(now.saturating_duration_since(entry.at).as_secs_f32() / TRANSITION_SECONDS)
        });
        if acquisition_weight >= 1.0 {
            self.acquired = None;
        }
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
                foot.normal
                    .lerp(normal, 1.0 - (-4.0 * dt / TRANSITION_SECONDS).exp())
                    .normalize_or(Vec3::Z)
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
            if let Some(entry) = acquisition {
                // Blend only terrain tilt over the live authored rotation;
                // activating placement must not instantly flatten the shoe.
                rotations[side] =
                    (entry.rotations[side].slerp(rotations[side] * authored.inverse(), acquisition_weight) * authored).normalize();
            }
            if foot.contact {
                targets[side].z += correction(footprint(ankle, rotations[side], leg.sole), hits).unwrap_or(0.0);
            } else if let Some(floor) = flight_floor {
                targets[side].z += floor - reference_floor;
            }
            if !supported[side] && foot.departure.is_none() {
                // With no terrain information this leg follows the body.
                // Seed a future acquisition from that uncorrected pose.
                foot.elevation = Some(Height {
                    value: reference_floor,
                    velocity: 0.0,
                });
                continue;
            }
            // Smooth only the vertical terrain offset, in an absolute frame
            // that cancels collider-root steps. Authored foot clearance and
            // lateral motion are always sampled from this frame's animation.
            let goal = reference_floor + targets[side].z - ankle.z;
            let elevation = foot.elevation.get_or_insert(Height {
                value: goal,
                velocity: 0.0,
            });
            elevation.follow(goal, dt);
            targets[side].z = ankle.z + elevation.value - reference_floor;
            if supported[side] {
                // Query the actual oriented footprint before deriving body
                // reach, so a toe crossing an edge is part of that constraint.
                let corners = footprint(targets[side], rotations[side], leg.sole);
                if let Some(lift) = correction(corners, corners.map(query)) {
                    // Ease the initial penetration by elapsed time, rather
                    // than a per-frame fraction. New collision remains hard
                    // once the avatar has joined the selected set.
                    let remaining = acquisition.map_or(0.0, |entry| entry.penetration[side] * (1.0 - acquisition_weight));
                    let lift = lift - remaining;
                    if lift > 0.0 {
                        targets[side].z += lift;
                        elevation.value += lift;
                        elevation.velocity = 0.0;
                    }
                }
            }
            // The user's limit also bounds collision and retained smoothing
            // state: lowering the setting cannot leave an older large offset.
            elevation.constrain(reference_floor - max_height, reference_floor + max_height);
            targets[side].z = ankle.z + elevation.value - reference_floor;
        }
        let mut lower = reference_floor - max_height;
        let mut upper = reference_floor + max_height;
        for (side, leg) in setup.legs.iter().enumerate() {
            if !self.feet[side].contact {
                continue;
            }
            let [hip, knee, ankle] = leg.joints.map(|j| source[j].w_axis.truncate());
            let reach = hip.distance(knee) + knee.distance(ankle) - 0.0001;
            let delta = hip - targets[side];
            let vertical = (reach * reach - delta.truncate().length_squared()).max(0.0).sqrt();
            lower = lower.max(reference_floor - vertical - delta.z);
            // Large permitted offsets must not put an upright source hip
            // below its planted ankle and reverse the knee's forward hinge.
            if hip.z - ankle.z > 0.05 {
                lower = lower.max(reference_floor + 0.05 - delta.z);
            }
            upper = upper.min(reference_floor + vertical - delta.z);
        }
        let goal = goal
            .unwrap_or(reference_floor)
            .clamp(reference_floor - max_height, reference_floor + max_height);
        let height = self.height.get_or_insert(Height {
            value: goal,
            velocity: 0.0,
        });
        if pose.allowed.iter().all(|allowed| *allowed) && self.feet.iter().any(|foot| foot.departure.is_some()) {
            height.follow(goal, dt);
            height.constrain(lower, upper.max(lower));
            height.constrain(reference_floor - max_height, reference_floor + max_height);
        } else {
            // Without any known support, source/root motion is authoritative;
            // filtering it here would introduce an unrelated body correction.
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
        log::trace!(target: "aurora_viewer::foot_placement", "terrain anchor={} body={:.5} reference={reference_floor:.5} contact={:?} targets={targets:?}",
            frame.standing_anchor, height.value, self.feet.each_ref().map(|foot| foot.contact));
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
                max_height: crate::settings::DEFAULT_GROUND_FEET_HEIGHT,
                standing_anchor: false,
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
                        reference_offset: Vec3::ZERO,
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
                    let settling = placement.feet[side].elevation.is_some_and(|height| height.velocity < -0.001);
                    assert!(
                        nearest < 0.004 || (settling && nearest < 0.25),
                        "contact foot {side} floats without settling: {nearest}"
                    );
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

    #[test]
    fn grounding_absolute_pelvis_origin_keeps_authored_motion_and_shoe_depth() {
        let f = Fixture::new();
        let now = Instant::now();
        let clip = crate::demo::grounding::contact_animation(&f.rig, &f.base, false);
        for fps in [30, 60, 144] {
            for shift in [-8.0, -0.28, 0.28, 10.53] {
                let mut base = f.base.clone();
                base.local_pos[f.rig.pelvis].z += shift;
                let mut original = Placement::default();
                let mut translated = Placement::default();
                for frame in 0..=fps * 2 {
                    let time = frame as f32 / fps as f32;
                    let mut rotations = f.rig.local_rot.clone();
                    let mut pelvis = Vec3::ZERO;
                    for joint in &clip.joints {
                        if let Some(&index) = f.rig.names.get(&joint.joint_name) {
                            if let Some(rotation) = joint.rotation_at_loop(time, clip.loop_range()) {
                                rotations[index] = rotation;
                            }
                            if index == f.rig.pelvis {
                                pelvis = joint.position_at_loop(time, clip.loop_range()).unwrap_or(Vec3::ZERO);
                            }
                        }
                    }
                    let at = now + Duration::from_secs_f32(time);
                    let frame = f.frame(Vec3::ZERO, 0.4);
                    let expected = f.sample(&mut original, &rotations, pelvis, Some(frame), at, &flat);
                    let shifted_pelvis = pelvis - Vec3::Z * shift;
                    let actual = translated
                        .apply(
                            &f.rig,
                            &base,
                            Pose {
                                rotations: &rotations,
                                positions: &base.local_pos,
                                pelvis: shifted_pelvis,
                                reference_offset: -Vec3::Z * shift,
                                allowed: [true; 2],
                            },
                            Some(Frame {
                                sole_height: Some(f.floor + shift),
                                ..frame
                            }),
                            at,
                            &flat,
                        )
                        .expect("placement");
                    let expected = f
                        .rig
                        .world_with(&expected.rotations, pelvis + expected.pelvis, &f.base.local_pos, &f.base.scale);
                    let actual = f
                        .rig
                        .world_with(&actual.rotations, shifted_pelvis + actual.pelvis, &base.local_pos, &base.scale);
                    for name in [
                        "mPelvis",
                        "mHipLeft",
                        "mHipRight",
                        "mKneeLeft",
                        "mKneeRight",
                        "mAnkleLeft",
                        "mAnkleRight",
                    ] {
                        let joint = f.rig.names[name];
                        // Metre-scale origin cancellation and a nearly straight
                        // knee amplify f32 matrix noise. Compare visible joint
                        // positions and angles in their own units instead.
                        let distance = expected[joint].w_axis.distance(actual[joint].w_axis);
                        let angle = orientation(expected[joint]).angle_between(orientation(actual[joint])).abs();
                        assert!(
                            distance < 0.001 && angle < 0.005,
                            "{name} changed after a {shift} m rest-origin shift at {fps} fps, time {time}: {distance} m, {angle} radians"
                        );
                    }
                }
            }
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
        for i in 1..=45 {
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
    fn grounding_height_limit_bounds_feet_body_and_live_changes_and_reaches_one_metre() {
        let f = Fixture::new();
        let now = Instant::now();
        let raw = &f.rig.local_rot;
        for fps in [30, 60, 144] {
            let mut step = Placement::default();
            for i in 0..=fps * 2 {
                let level = if i < fps { 0.0 } else { 0.95 };
                let mut frame = f.frame(Vec3::Z * level, 0.0);
                frame.max_height = 1.0;
                let ground = |p: Vec3| {
                    Some(Surface {
                        position: p.with_z(level),
                        normal: Vec3::Z,
                    })
                };
                let placed = f.sample(
                    &mut step,
                    raw,
                    Vec3::ZERO,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / fps as f32),
                    &ground,
                );
                let world = f.check(&step, &placed, raw, Vec3::ZERO, frame, &ground);
                for side in ["Left", "Right"] {
                    let hip = world[f.rig.names[&format!("mHip{side}")]].w_axis;
                    let knee = world[f.rig.names[&format!("mKnee{side}")]].w_axis;
                    assert!(knee.x >= hip.x - 0.005, "a large root step reversed the knee hinge");
                }
            }
            for limit in [0.1, 0.4, 1.0] {
                let mut placement = Placement::default();
                let mut frame = f.frame(Vec3::Z * 0.95, 0.0);
                frame.max_height = limit;
                let source = f.rig.world_with(raw, Vec3::ZERO, &f.base.local_pos, &f.base.scale);
                for i in 0..=fps {
                    let placed = f.sample(
                        &mut placement,
                        raw,
                        Vec3::ZERO,
                        Some(frame),
                        now + Duration::from_secs_f32(i as f32 / fps as f32),
                        &flat,
                    );
                    let output = f.rig.world_with(&placed.rotations, placed.pelvis, &f.base.local_pos, &f.base.scale);
                    assert!(placed.pelvis.z.abs() <= limit + 0.0001, "body exceeded the configured height");
                    for name in ["mAnkleLeft", "mAnkleRight"] {
                        let j = f.rig.names[name];
                        let delta = output[j].w_axis.z - source[j].w_axis.z;
                        assert!(delta.abs() <= limit + 0.0002, "{name} exceeded {limit}: {delta}");
                    }
                    if i == fps && limit == 1.0 {
                        f.check(&placement, &placed, raw, Vec3::ZERO, frame, &flat);
                        assert!(
                            (placed.pelvis.z + 0.95).abs() < 0.004,
                            "one-metre range was still blocked by the old trace or pelvis limit"
                        );
                    }
                }
                frame.max_height = 0.05;
                let placed = f.sample(
                    &mut placement,
                    raw,
                    Vec3::ZERO,
                    Some(frame),
                    now + Duration::from_secs_f32(1.0 + 1.0 / fps as f32),
                    &flat,
                );
                let output = f.rig.world_with(&placed.rotations, placed.pelvis, &f.base.local_pos, &f.base.scale);
                assert!(
                    placed.pelvis.z.abs() <= 0.0501,
                    "lowering the limit retained old body smoothing state"
                );
                for name in ["mAnkleLeft", "mAnkleRight"] {
                    let j = f.rig.names[name];
                    assert!(
                        (output[j].w_axis.z - source[j].w_axis.z).abs() <= 0.0502,
                        "lowering the limit retained an old foot offset"
                    );
                }
            }
        }
    }

    #[test]
    fn grounding_smoothing_settles_a_lower_support_but_preserves_lifts_and_collision() {
        let f = Fixture::new();
        let now = Instant::now();
        let mut unloaded = Placement::default();
        for i in 0..=1 {
            let frame = f.frame(Vec3::Z * (i as f32 * 0.1), 0.0);
            let placed = f.sample(
                &mut unloaded,
                &f.rig.local_rot,
                Vec3::ZERO,
                Some(frame),
                now + Duration::from_secs_f32(i as f32 / 60.0),
                &|_| None,
            );
            for (&placed, &raw) in placed.rotations.iter().zip(&f.rig.local_rot) {
                assert!(placed.abs_diff_eq(raw, 1e-5), "unavailable terrain introduced a leg correction");
            }
            assert_eq!(placed.pelvis, Vec3::ZERO, "unavailable terrain introduced a body correction");
        }
        let frame = f.frame(Vec3::Z * 0.25, 0.0);
        let ankle = f.rig.names["mAnkleLeft"];
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            let mut previous = None::<f32>;
            for i in 0..=fps * 2 {
                let level = if i < fps { 0.22 } else { 0.10 };
                let ground = |p: Vec3| {
                    Some(Surface {
                        position: p.with_z(if p.y > 0.0 { level } else { 0.0 }),
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
                let z = world[ankle].w_axis.z;
                if i >= fps {
                    let before = previous.expect("settled support");
                    assert!(
                        z <= before + 1e-5 && before - z < 1.1 / fps as f32,
                        "support transition snapped at {fps} fps"
                    );
                    if i == fps {
                        assert!(before - z < 0.025, "the correction was applied at once");
                    }
                    if i == fps * 2 {
                        assert!((before - z).abs() < 1e-5, "the foot never settled");
                    }
                }
                previous = Some(z);
            }
            // A new source lift is visible immediately, even though terrain
            // offsets ease. Filtering solved bones would delay this movement.
            let rest = f.rig.world_with(&f.rig.local_rot, Vec3::ZERO, &f.base.local_pos, &f.base.scale);
            let [hip, knee] = ["mHipLeft", "mKneeLeft"].map(|name| f.rig.names[name]);
            let length = rest[hip].w_axis.distance(rest[knee].w_axis) + rest[knee].w_axis.distance(rest[ankle].w_axis);
            let bend = (1.0 - 0.08 / length).acos();
            let mut raw = f.rig.local_rot.clone();
            raw[hip] = Quat::from_rotation_y(-bend);
            raw[knee] = Quat::from_rotation_y(2.0 * bend);
            raw[ankle] = Quat::from_rotation_y(-bend);
            let ground = |p: Vec3| {
                Some(Surface {
                    position: p.with_z(if p.y > 0.0 { 0.1 } else { 0.0 }),
                    normal: Vec3::Z,
                })
            };
            let at = now + Duration::from_secs_f32(2.0 + 1.0 / fps as f32);
            let placed = f.sample(&mut placement, &raw, Vec3::ZERO, Some(frame), at, &ground);
            let world = f.check(&placement, &placed, &raw, Vec3::ZERO, frame, &ground);
            assert!(
                world[ankle].w_axis.z - previous.unwrap() > 0.075,
                "smoothing delayed the source lift"
            );
            let raised = |p: Vec3| {
                Some(Surface {
                    position: p.with_z(if p.y > 0.0 { 0.22 } else { 0.0 }),
                    normal: Vec3::Z,
                })
            };
            let placed = f.sample(
                &mut placement,
                &raw,
                Vec3::ZERO,
                Some(frame),
                at + Duration::from_secs_f32(1.0 / fps as f32),
                &raised,
            );
            f.check(&placement, &placed, &raw, Vec3::ZERO, frame, &raised);
        }
    }

    #[test]
    fn grounding_acquisition_from_penetrating_source_is_continuous() {
        let f = Fixture::new();
        let now = Instant::now();
        let torso = f.rig.names["mTorso"];
        let mut initial = f.rig.local_rot.clone();
        initial[f.rig.names["mAnkleRight"]] = Quat::from_rotation_y(0.15);
        for fps in [30, 60, 144] {
            for depth in [0.18, 0.35] {
                let frame = f.frame(-Vec3::Z * depth, 0.0);
                let mut placement = Placement::default();
                f.sample(&mut placement, &initial, Vec3::ZERO, None, now, &flat);
                let source = f.rig.world_with(&initial, Vec3::ZERO, &f.base.local_pos, &f.base.scale);
                let mut previous = source;
                for step in 0..=fps {
                    let time = step as f32 / fps as f32;
                    let mut raw = initial.clone();
                    raw[torso] = Quat::from_rotation_x(0.15 * (time * 5.0).sin());
                    let placed = f.sample(
                        &mut placement,
                        &raw,
                        Vec3::ZERO,
                        Some(frame),
                        now + Duration::from_secs_f32(time + 1.0 / fps as f32),
                        &flat,
                    );
                    assert!(
                        placed.rotations[torso].abs_diff_eq(raw[torso], 1e-6),
                        "activation froze the current animation"
                    );
                    if step == 0 {
                        for (placed, raw) in placed.rotations.iter().zip(&raw) {
                            assert!(placed.abs_diff_eq(*raw, 1e-5), "activation changed the source orientation");
                        }
                        assert!(
                            placed.pelvis.length() < 0.002,
                            "activation jumped the pelvis by {:.6} m at {fps} fps",
                            placed.pelvis.length()
                        );
                    }
                    let output = f.rig.world_with(&placed.rotations, placed.pelvis, &f.base.local_pos, &f.base.scale);
                    for name in ["mPelvis", "mKneeLeft", "mKneeRight", "mAnkleLeft", "mAnkleRight"] {
                        let joint = f.rig.names[name];
                        let distance = output[joint].w_axis.distance(previous[joint].w_axis);
                        assert!(distance < 2.5 / fps as f32, "{name} jumped {distance:.6} m at {fps} fps");
                    }
                    if time >= 0.6 {
                        f.check(&placement, &placed, &raw, Vec3::ZERO, frame, &flat);
                    }
                    previous = output;
                }
            }
        }
    }

    #[test]
    fn grounding_acquisition_after_recalibration_preserves_the_visible_release() {
        let f = Fixture::new();
        let now = Instant::now();
        let old_frame = f.frame(Vec3::Z * 0.2, 0.0);
        for delay in [0.25, 0.75] {
            let mut placement = Placement::default();
            f.sample(&mut placement, &f.rig.local_rot, Vec3::ZERO, Some(old_frame), now, &flat);
            f.sample(
                &mut placement,
                &f.rig.local_rot,
                Vec3::ZERO,
                None,
                now + Duration::from_secs(1),
                &flat,
            );
            let at = now + Duration::from_secs_f32(1.0 + delay);
            let before = f.sample(&mut placement, &f.rig.local_rot, Vec3::ZERO, None, at, &flat);
            let frame = Frame {
                sole_height: Some(f.floor - 0.08),
                ..old_frame
            };
            let after = f.sample(
                &mut placement,
                &f.rig.local_rot,
                Vec3::ZERO,
                Some(frame),
                at + Duration::from_micros(1),
                &flat,
            );
            let positions = |pose: &Placed| f.rig.world_with(&pose.rotations, pose.pelvis, &f.base.local_pos, &f.base.scale);
            let (before, after) = (positions(&before), positions(&after));
            for name in ["mPelvis", "mKneeLeft", "mKneeRight", "mAnkleLeft", "mAnkleRight"] {
                let joint = f.rig.names[name];
                let distance = before[joint].w_axis.distance(after[joint].w_axis);
                assert!(
                    distance < 0.002,
                    "{name} snapped {distance:.6} m on acquisition after a new sole measurement ({delay}s release)"
                );
            }
            for step in 1..=60 {
                let placed = f.sample(
                    &mut placement,
                    &f.rig.local_rot,
                    Vec3::ZERO,
                    Some(frame),
                    at + Duration::from_secs_f32(step as f32 / 60.0),
                    &flat,
                );
                if step == 60 {
                    f.check(&placement, &placed, &f.rig.local_rot, Vec3::ZERO, frame, &flat);
                }
            }
        }
    }

    #[test]
    fn grounding_smoothing_reenable_continues_the_visible_partial_or_complete_fade() {
        let f = Fixture::new();
        let now = Instant::now();
        let frame = f.frame(Vec3::Z * 0.2, 0.0);
        let mut initially_disabled = Placement::default();
        f.sample(&mut initially_disabled, &f.rig.local_rot, Vec3::ZERO, None, now, &flat);
        let enabled = f.sample(
            &mut initially_disabled,
            &f.rig.local_rot,
            Vec3::ZERO,
            Some(frame),
            now + Duration::from_secs_f32(1.0 / 60.0),
            &flat,
        );
        assert!(enabled.pelvis.length() < 0.001, "initial activation jumped to the full correction");
        for i in 1..=30 {
            let placed = f.sample(
                &mut initially_disabled,
                &f.rig.local_rot,
                Vec3::ZERO,
                Some(frame),
                now + Duration::from_secs_f32((i + 1) as f32 / 60.0),
                &flat,
            );
            if i == 15 {
                assert!(
                    (0.7..0.9).contains(&(placed.pelvis.length() / 0.199)),
                    "activation settled before the requested half-second transition"
                );
            }
            if i == 30 {
                assert!(
                    placed.pelvis.length() / 0.199 > 0.98,
                    "activation did not settle within half a second"
                );
            }
        }
        for delay in [0.25, 0.75] {
            let mut placement = Placement::default();
            let planted = f.sample(&mut placement, &f.rig.local_rot, Vec3::ZERO, Some(frame), now, &flat);
            f.sample(
                &mut placement,
                &f.rig.local_rot,
                Vec3::ZERO,
                None,
                now + Duration::from_secs(1),
                &flat,
            );
            let at = now + Duration::from_secs_f32(1.0 + delay);
            let before = f.sample(&mut placement, &f.rig.local_rot, Vec3::ZERO, None, at, &flat);
            if delay == 0.25 {
                assert!(
                    before.pelvis.distance(planted.pelvis * 0.5) < 1e-5,
                    "fade-out should be halfway after 250 ms"
                );
            }
            // Walking while disabled must not be mistaken for an active IK
            // teleport, which would discard the visible source-pose seed.
            let frame = if delay > 0.5 {
                f.frame(Vec3::new(3.0, 0.0, 0.2), 0.0)
            } else {
                frame
            };
            let after = f.sample(
                &mut placement,
                &f.rig.local_rot,
                Vec3::ZERO,
                Some(frame),
                at + Duration::from_micros(1),
                &flat,
            );
            let world = |placed: &Placed| f.rig.world_with(&placed.rotations, placed.pelvis, &f.base.local_pos, &f.base.scale);
            let (before_world, after_world) = (world(&before), world(&after));
            for name in ["mPelvis", "mKneeLeft", "mKneeRight", "mAnkleLeft", "mAnkleRight"] {
                let j = f.rig.names[name];
                assert!(
                    before_world[j].w_axis.distance(after_world[j].w_axis) < 0.002,
                    "{name} snapped on re-enable after {delay}s"
                );
            }
            let mut last_gap = f32::INFINITY;
            for i in 1..=60 {
                let placed = f.sample(
                    &mut placement,
                    &f.rig.local_rot,
                    Vec3::ZERO,
                    Some(frame),
                    at + Duration::from_secs_f32(i as f32 / 60.0),
                    &flat,
                );
                let output = f.check(&placement, &placed, &f.rig.local_rot, Vec3::ZERO, frame, &flat);
                let leg = placement.setup.as_ref().unwrap().legs[0];
                let ankle = output[leg.joints[2]];
                let gap = footprint(ankle.w_axis.truncate(), orientation(ankle), leg.sole)
                    .into_iter()
                    .map(|p| p.z)
                    .fold(f32::INFINITY, f32::min);
                assert!(gap <= last_gap + 1e-4, "re-enable reversed its approach to the support");
                if i == 60 {
                    assert!(gap < 0.004, "re-enable never reached support");
                }
                last_gap = gap;
            }
        }
    }

    #[test]
    fn grounding_retains_additive_source_motion_without_feedback_into_the_held_blend() {
        let f = Fixture::new();
        let now = Instant::now();
        let torso = f.rig.names["mTorso"];
        let ankle = f.rig.names["mAnkleLeft"];
        let additive = Quat::from_rotation_x(0.12);
        let mut source = f.rig.local_rot.clone();
        source[torso] = additive * source[torso];
        let expected = f.rig.world_with(&source, Vec3::ZERO, &f.base.local_pos, &f.base.scale);
        for enabled in [false, true] {
            // A newly evaluated paused controller uses its retained procedural
            // states, giving a deterministic additive body-noise substitute.
            let mut controller = Controller {
                aurora_feet: enabled,
                ground: enabled.then(|| f.frame(Vec3::Z * 0.2, 0.0)),
                paused_at: Some(now),
                held_procedural: vec![(torso, 2, 1.0, additive, true)],
                ..Default::default()
            };
            let mut output = vec![Mat4::IDENTITY.to_cols_array_2d(); f.rig.len()];
            controller.evaluate_on_ground(&f.rig, now, Some(&f.base), &mut output, flat);
            let torso_world = Mat4::from_cols_array_2d(&output[torso]);
            assert!(orientation(torso_world).abs_diff_eq(orientation(expected[torso]), 1e-5));
            assert!(controller.rot[torso].abs_diff_eq(f.rig.local_rot[torso], 1e-5));
            let ankle_world = Mat4::from_cols_array_2d(&output[ankle]);
            if enabled {
                assert!(ankle_world.w_axis.z < expected[ankle].w_axis.z - 0.15);
            } else {
                assert!(ankle_world.abs_diff_eq(expected[ankle], 1e-5));
            }
        }
    }

    #[test]
    fn grounding_confirmed_standing_anchor_ignores_stale_airborne_state() {
        let f = Fixture::new();
        let now = Instant::now();
        for (state, anchored, expected) in [
            (State::Falling, false, false),
            (State::Falling, true, true),
            (State::Sitting, true, false),
            (State::GroundSit, true, false),
        ] {
            let mut frame = f.frame(Vec3::Z * 0.2, 0.0);
            frame.standing_anchor = anchored;
            frame.velocity_z = -2.0;
            let mut controller = Controller {
                aurora_feet: true,
                ground: Some(frame),
                ..Default::default()
            };
            let signal = PlayingAnimation::from_signal(state.motion(), 1, now, None);
            let clip = Arc::new(BoundAnim::bind(crate::demo::idle_animation(), &f.rig));
            controller.sync(&[signal], now, |_| Some(clip.clone()));
            let queries = std::cell::Cell::new(0);
            let mut output = vec![Mat4::IDENTITY.to_cols_array_2d(); f.rig.len()];
            controller.evaluate_on_ground(&f.rig, now, Some(&f.base), &mut output, |p| {
                queries.set(queries.get() + 1);
                flat(p)
            });
            assert_eq!(
                queries.get() > 0,
                expected,
                "the standing anchor supersedes stale flight, never a sitting pose"
            );
        }
    }

    #[test]
    fn grounding_stays_out_of_the_ik_edit_snapshot_and_resumes_after_editing() {
        use crate::scene::ik::{EditRequest, Goal, Mode};

        let f = Fixture::new();
        let now = Instant::now();
        let ankle = f.rig.names["mAnkleLeft"];
        let expected = f.rig.world_with(&f.rig.local_rot, Vec3::ZERO, &f.base.local_pos, &f.base.scale);
        let mut controller = Controller {
            aurora_feet: true,
            ground: Some(f.frame(Vec3::Z * 0.2, 0.0)),
            paused_at: Some(now),
            ..Default::default()
        };
        let mut output = vec![Mat4::IDENTITY.to_cols_array_2d(); f.rig.len()];
        controller.evaluate_on_ground(&f.rig, now, Some(&f.base), &mut output, flat);
        assert!(Mat4::from_cols_array_2d(&output[ankle]).w_axis.z < expected[ankle].w_axis.z - 0.15);

        controller.ik.edit = EditRequest {
            active: true,
            ..Default::default()
        };
        let no_query = |_: Vec3| -> Option<Surface> { panic!("IK editing queried automatic terrain placement") };
        controller.evaluate_on_ground(&f.rig, now, Some(&f.base), &mut output, no_query);
        assert!(Mat4::from_cols_array_2d(&output[ankle]).abs_diff_eq(expected[ankle], 1e-5));

        let target = expected[ankle].w_axis.truncate() + Vec3::new(0.06, 0.0, 0.1);
        controller.ik.edit.goals.push(Goal {
            follow: None,
            weight: 1.0,
            bone: ankle,
            root: ankle,
            position: target,
            rotation: Some(Quat::from_rotation_x(0.3)),
            contact: Vec3::ZERO,
            pole: None,
            mode: Mode::Direct,
        });
        controller.evaluate_on_ground(&f.rig, now, Some(&f.base), &mut output, no_query);
        assert!(Mat4::from_cols_array_2d(&output[ankle]).w_axis.truncate().distance(target) < 1e-5);
        controller.ik.edit.goals.clear();
        controller.evaluate_on_ground(&f.rig, now, Some(&f.base), &mut output, no_query);
        assert!(Mat4::from_cols_array_2d(&output[ankle]).abs_diff_eq(expected[ankle], 1e-5));

        controller.ik.edit = EditRequest::default();
        controller.set_paused(false, now + Duration::from_secs(1));
        // Terrain placement resumes with its normal blend instead of jumping
        // from the edited pose on the first frame after unpausing.
        for step in 1..=30 {
            controller.evaluate_on_ground(
                &f.rig,
                now + Duration::from_millis(1000 + step * 40),
                Some(&f.base),
                &mut output,
                flat,
            );
        }
        assert!(Mat4::from_cols_array_2d(&output[ankle]).w_axis.z < expected[ankle].w_axis.z - 0.15);
    }

    #[test]
    fn explicit_ik_contacts_override_grounding_and_release_back_to_it() {
        use crate::scene::ik::{Goal, Mode};

        let f = Fixture::new();
        let now = Instant::now();
        let ankle = f.rig.names["mAnkleLeft"];
        let mut controller = Controller {
            aurora_feet: true,
            ground: Some(f.frame(Vec3::Z * 0.2, 0.0)),
            ..Default::default()
        };
        let mut output = vec![Mat4::IDENTITY.to_cols_array_2d(); f.rig.len()];
        controller.evaluate_on_ground(&f.rig, now, Some(&f.base), &mut output, flat);
        let placed = Mat4::from_cols_array_2d(&output[ankle]);
        let target = placed.w_axis.truncate() + Vec3::new(0.05, 0.0, 0.08);
        let rotation = Quat::from_rotation_y(0.35);
        controller.ik.targets.push(Goal {
            follow: None,
            weight: 1.0,
            bone: ankle,
            root: ankle,
            position: target,
            rotation: Some(rotation),
            contact: Vec3::ZERO,
            pole: None,
            mode: Mode::Direct,
        });
        for step in 1..=8 {
            controller.evaluate_on_ground(&f.rig, now + Duration::from_millis(step * 40), Some(&f.base), &mut output, flat);
        }
        let contact = Mat4::from_cols_array_2d(&output[ankle]);
        assert!(contact.w_axis.truncate().distance(target) < 1e-5);
        assert!(orientation(contact).dot(rotation).abs() > 0.99999);
        controller.ik.targets.clear();
        for step in 9..=16 {
            controller.evaluate_on_ground(&f.rig, now + Duration::from_millis(step * 40), Some(&f.base), &mut output, flat);
        }
        assert!(Mat4::from_cols_array_2d(&output[ankle]).abs_diff_eq(placed, 1e-5));
        assert!(controller.rot[ankle].abs_diff_eq(f.rig.local_rot[ankle], 1e-5));
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
                aurora_feet: true,
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
            assert!(unplanted(state.motion(), false));
        }
    }
}
