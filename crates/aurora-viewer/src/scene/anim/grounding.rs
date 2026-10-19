//! Pose-space foot placement, independent of animation IDs and playback.
//! Design references (algorithms reimplemented here, no source copied):
//! - ozz-animation Foot IK: sole geometry and pelvis reach constraints.
//!   https://guillaumeblanc.github.io/ozz-animation/samples/foot_ik/
//! - Daniel Holden, Inverse Kinematics and Foot Locking: source-motion
//!   contacts, bounded planting, minimal pose changes and ground collision.
//!   https://theorangeduck.com/page/inverse-kinematics-foot-locking
//!
//! Filter targets before solving, then enforce contact/collision constraints.
//! Filtering solved joint rotations would reintroduce floating/penetration.

use super::*;
use crate::ao::State;
use glam::Vec4;

const MAX_PELVIS: f32 = 0.45;
const CONTACT_MARGIN: f32 = 0.001;

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
    /// Neutral sole height measured by the existing hover calibration.
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

#[derive(Clone, Copy, Default)]
struct Spring {
    position: Vec3,
    velocity: Vec3,
}

impl Spring {
    /// Exact critically damped response for a constant target over this step.
    fn advance(&mut self, target: Vec3, dt: f32, frequency: f32) {
        let error = self.position - target;
        let c = self.velocity + error * frequency;
        let decay = (-frequency * dt).exp();
        self.position = target + (error + c * dt) * decay;
        self.velocity = (self.velocity - c * (frequency * dt)) * decay;
        if self.position.distance_squared(target) < 1e-10 && self.velocity.length_squared() < 1e-8 {
            self.position = target;
            self.velocity = Vec3::ZERO;
        }
    }
}

#[derive(Clone, Copy)]
struct Leg {
    joints: [usize; 3],
    /// Sole corners in ankle-rotation space, already scaled for this shape.
    sole: [Vec3; 4],
    neutral: [Vec3; 4],
}

#[derive(Default)]
struct Foot {
    previous: Option<[Vec3; 4]>,
    previous_world: Option<Vec3>,
    planted: bool,
    anchor: Option<Vec3>,
    tilt: Quat,
    offset: Spring,
    pole: Option<Vec3>,
}

#[derive(Default)]
pub(super) struct Placement {
    base: Option<SkeletonBase>,
    reference: Option<f32>,
    legs: Option<[Leg; 2]>,
    feet: [Foot; 2],
    pelvis: Spring,
    sampled: Option<Instant>,
    frame: Option<Frame>,
    held: Option<Placed>,
}

fn rotation(world: &[Mat4], joint: usize) -> Quat {
    world[joint].to_scale_rotation_translation().1.normalize()
}

fn points(ankle: Vec3, orientation: Quat, sole: [Vec3; 4]) -> [Vec3; 4] {
    sole.map(|point| ankle + orientation * point)
}

fn surface(frame: Frame, point: Vec3, ground: &dyn Fn(Vec3) -> Option<Surface>) -> Option<Surface> {
    let support = ground(frame.to_world.transform_point3(point))?;
    let inverse = frame.to_world.inverse();
    let support = Surface {
        position: inverse.transform_point3(support.position),
        normal: inverse.transform_vector3(support.normal).normalize_or(Vec3::Z),
    };
    ((support.position - point).length() < 0.6).then_some(support)
}

fn lift(corners: [Vec3; 4], supports: [Option<Surface>; 4], up: Vec3) -> Option<f32> {
    corners
        .into_iter()
        .zip(supports)
        .filter_map(|(point, support)| {
            let support = support?;
            let denominator = up.dot(support.normal);
            (denominator > 0.2).then(|| ((support.position - point).dot(support.normal) + CONTACT_MARGIN) / denominator)
        })
        .reduce(f32::max)
}

/// Preserve the authored bend plane; only a genuinely straight knee needs a
/// fallback pole. The solve changes rotations, never segment lengths.
fn solve(hip: Vec3, knee: Vec3, ankle: Vec3, target: Vec3, fallback: Vec3) -> Option<(Quat, Quat, Vec3)> {
    let upper = knee - hip;
    let lower = ankle - knee;
    let (a, b) = (upper.length(), lower.length());
    let delta = target - hip;
    let distance = delta.length();
    if !a.is_finite() || !b.is_finite() || !distance.is_finite() || a < 1e-5 || b < 1e-5 || distance < 1e-5 {
        return None;
    }
    let axis = delta / distance;
    let distance = distance.clamp((a - b).abs() + 1e-6, a + b - 1e-6);
    let source_axis = (ankle - hip).normalize_or(-Vec3::Z);
    let source_pole = upper - source_axis * upper.dot(source_axis);
    let pole = if source_pole.length_squared() > 0.000025 {
        source_pole
    } else {
        fallback
    };
    let pole = (pole - axis * pole.dot(axis))
        .try_normalize()
        .unwrap_or_else(|| axis.any_orthonormal_vector());
    let along = (a * a - b * b + distance * distance) / (2.0 * distance);
    let bend = hip + axis * along + pole * (a * a - along * along).max(0.0).sqrt();
    let hip_delta = Quat::from_rotation_arc(upper / a, (bend - hip).normalize());
    let knee_delta = Quat::from_rotation_arc((hip_delta * lower).normalize(), (hip + axis * distance - bend).normalize());
    Some((hip_delta, knee_delta, pole))
}

impl Placement {
    pub(super) fn resume(&mut self, duration: Duration) {
        self.sampled = self.sampled.and_then(|at| at.checked_add(duration));
    }

    fn initialize(&mut self, rig: &Rig, base: &SkeletonBase, reference: Option<f32>) -> Option<()> {
        let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
        let mut legs = Vec::with_capacity(2);
        for side in ["Left", "Right"] {
            let mut joints = [0; 3];
            for (joint, bone) in joints.iter_mut().zip(["Hip", "Knee", "Ankle"]) {
                *joint = *rig.names.get(&format!("m{bone}{side}"))?;
            }
            let [hip, knee, ankle] = joints;
            if rig.parent[knee] != Some(hip) || rig.parent[ankle] != Some(knee) {
                return None;
            }
            let ball = *rig.names.get(&format!("mFoot{side}"))?;
            let toe = rig.names.get(&format!("mToe{side}")).copied().unwrap_or(ball);
            let ankle_position = rest[ankle].w_axis.truncate();
            let ankle_rotation = rotation(&rest, ankle);
            let ball_offset = ankle_rotation.inverse() * (rest[ball].w_axis.truncate() - ankle_position);
            let toe_offset = ankle_rotation.inverse() * (rest[toe].w_axis.truncate() - ankle_position);
            let depth = reference.map_or(-ball_offset.z, |z| (ankle_position.z - z).clamp(0.01, 0.5));
            let width = 0.045 * base.scale[ankle].y;
            let sole = [
                Vec3::new(-0.025, -width, -depth),
                Vec3::new(-0.025, width, -depth),
                Vec3::new(toe_offset.x.max(ball_offset.x), -width, -depth),
                Vec3::new(toe_offset.x.max(ball_offset.x), width, -depth),
            ];
            legs.push(Leg {
                joints,
                sole,
                neutral: points(ankle_position, ankle_rotation, sole),
            });
        }
        *self = Self {
            base: Some(base.clone()),
            reference,
            legs: Some([legs[0], legs[1]]),
            ..Default::default()
        };
        Some(())
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
        log::trace!(target: "aurora_viewer::foot_placement", "input frame={} allowed={:?} root={:?} sole={:?}",
            frame.is_some(), pose.allowed, frame.map(|frame| frame.to_world.w_axis), frame.and_then(|frame| frame.sole_height));
        let active =
            self.pelvis.position.length_squared() > 1e-10 || self.feet.iter().any(|foot| foot.offset.position.length_squared() > 1e-10);
        if frame.is_none() && !active {
            self.sampled = None;
            return None;
        }
        let teleported = frame
            .zip(self.frame)
            .is_some_and(|(a, b)| a.to_world.w_axis.distance_squared(b.to_world.w_axis) > 4.0);
        let reference = frame.and_then(|frame| frame.sole_height).or(self.reference);
        if self.base.as_ref() != Some(base) || teleported || self.reference != reference {
            self.initialize(rig, base, reference)?;
        }
        if self.sampled == Some(now)
            && let Some(held) = &self.held
        {
            return Some(held.clone());
        }
        let dt = self
            .sampled
            .replace(now)
            .map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f32())
            .min(0.1);
        self.frame = frame.or(self.frame);
        let transform = self.frame?;
        let inverse = transform.to_world.inverse();
        let up = inverse.transform_vector3(Vec3::Z).normalize_or(Vec3::Z);
        let root_rotation = transform.to_world.to_scale_rotation_translation().1;
        let legs = self.legs?;
        let source = rig.world_with(pose.rotations, pose.pelvis, pose.positions, &base.scale);
        let mut targets = [Vec3::ZERO; 2];
        let mut orientations = [Quat::IDENTITY; 2];
        let mut supported = [false; 2];
        let mut clearances = [0.0; 2];
        for side in 0..2 {
            let leg = legs[side];
            let ankle = source[leg.joints[2]].w_axis.truncate();
            let authored_rotation = rotation(&source, leg.joints[2]);
            let corners = points(ankle, authored_rotation, leg.sole);
            let center = corners.into_iter().sum::<Vec3>() * 0.25;
            let world_center = transform.to_world.transform_point3(center);
            let foot = &mut self.feet[side];
            let was_planted = foot.planted;
            let height = corners
                .into_iter()
                .zip(leg.neutral)
                .map(|(point, rest)| (point - rest).dot(up))
                .fold(f32::INFINITY, f32::min);
            clearances[side] = height.max(0.0);
            let previous_height = foot.previous.map(|previous| {
                previous
                    .into_iter()
                    .zip(leg.neutral)
                    .map(|(point, rest)| (point - rest).dot(up))
                    .fold(f32::INFINITY, f32::min)
            });
            let vertical_speed = previous_height
                .filter(|_| dt > 0.0001)
                .map_or(0.0, |previous| (height - previous) / dt);
            let velocity = foot
                .previous_world
                .filter(|_| dt > 0.0001)
                .map_or(Vec3::ZERO, |previous| (world_center - previous) / dt);
            let speed = velocity.length();
            foot.previous = Some(corners);
            foot.previous_world = Some(world_center);
            let enabled = frame.is_some() && pose.allowed[side];
            foot.planted = enabled
                && if foot.planted {
                    height < 0.055 && (height < 0.005 || vertical_speed < 0.015)
                } else {
                    height < 0.025 && (height < 0.005 || vertical_speed < 0.01)
                };
            let supports = if enabled {
                corners.map(|point| surface(transform, point, ground))
            } else {
                [None; 4]
            };
            supported[side] = supports.iter().any(Option::is_some);
            foot.planted &= supported[side];
            if was_planted != foot.planted {
                // Stance targets are absolute; swing offsets travel with the
                // pelvis. Change coordinates without applying pelvic motion
                // twice at toe-off (which would pin the lifting foot down).
                let shift = transform.to_world.transform_vector3(up * self.pelvis.position.z);
                foot.offset.position += if foot.planted { shift } else { -shift };
            }
            if !foot.planted || speed > 0.25 || foot.anchor.is_some_and(|anchor| anchor.distance(world_center) > 0.18) {
                foot.anchor = None;
            } else if foot.anchor.is_none() {
                foot.anchor = Some(world_center);
            }
            let normal = supports
                .iter()
                .flatten()
                .map(|support| root_rotation * support.normal)
                .sum::<Vec3>()
                .normalize_or(Vec3::Z);
            let tilt = if foot.planted {
                Quat::from_rotation_arc(up, root_rotation.inverse() * normal)
            } else {
                Quat::IDENTITY
            };
            // Smooth the support tilt before solving, including toe-off and
            // replanting. The authored ankle roll is never flattened away.
            foot.tilt = foot.tilt.slerp(tilt, 1.0 - (-dt * 40.0).exp()).normalize();
            orientations[side] = foot.tilt * authored_rotation;
            let horizontal = foot.anchor.map_or(Vec3::ZERO, |anchor| {
                let difference = inverse.transform_vector3(anchor - world_center);
                difference - up * difference.dot(up)
            });
            let wanted = ankle + horizontal;
            let mut required = lift(points(wanted, orientations[side], leg.sole), supports, up).unwrap_or(0.0);
            if enabled && !foot.planted && velocity.truncate().length_squared() > 0.01 {
                // A short swing look-ahead starts lifting before a stair edge,
                // while the final collision projection remains authoritative.
                let ahead = inverse.transform_vector3(velocity.with_z(0.0) * 0.08);
                let future = corners.map(|point| point + ahead);
                let future_support = future.map(|point| surface(transform, point, ground));
                required = required.max(lift(future, future_support, up).unwrap_or(0.0));
            }
            // Warp swing clearance over the terrain, rather than tracking the
            // unadapted world's ankle. A foot lifting from a high step must
            // lift FROM that step even while the other leg lowers the pelvis.
            let swing = if supported[side] {
                required + height.max(0.0) - CONTACT_MARGIN - self.pelvis.position.z
            } else {
                0.0
            };
            let correction = horizontal + up * if foot.planted { required } else { swing };
            foot.offset.advance(
                transform.to_world.transform_vector3(correction),
                dt,
                if foot.planted { 25.0 } else { 45.0 },
            );
            targets[side] = ankle + inverse.transform_vector3(foot.offset.position);
            if enabled && foot.planted {
                // Stance targets are absolute. Swing collision is projected
                // below, after applying the current pelvic displacement.
                let required = lift(points(targets[side], orientations[side], leg.sole), supports, up).unwrap_or(0.0);
                targets[side] += up * if foot.planted { required } else { required.max(0.0) };
            }
        }
        // Feasible pelvis interval from both leg reach spheres. Choose the
        // nearest valid height, rather than lowering the body to bend both
        // knees. Projection prevents a smoothing lag from leaving a foot up.
        let mut lower = -MAX_PELVIS;
        let mut upper = MAX_PELVIS;
        if pose.allowed.iter().all(|allowed| *allowed) {
            for side in 0..2 {
                if !supported[side] {
                    continue;
                }
                let [hip, knee, ankle] = legs[side].joints;
                let p = |joint: usize| source[joint].w_axis.truncate();
                let length = p(hip).distance(p(knee)) + p(knee).distance(p(ankle)) - 0.0001;
                // Anticipate a swing foot's eventual support as well. If we
                // release its pelvis bound at toe-off, the body rises and
                // must abruptly drop when this lower foot replants. Use a
                // virtual contact target only for reach planning; the actual
                // swing target keeps all its authored clearance below.
                let contact = targets[side]
                    + if self.feet[side].planted {
                        Vec3::ZERO
                    } else {
                        up * (self.pelvis.position.z - clearances[side])
                    };
                let delta = p(hip) - contact;
                let along = delta.dot(up);
                let horizontal = (delta - up * along).length_squared();
                let reach = (length * length - horizontal).max(0.0).sqrt();
                lower = lower.max(-reach - along);
                upper = upper.min(reach - along);
            }
        } else {
            lower = 0.0;
            upper = 0.0;
        }
        let feasible = lower <= upper;
        let desired = if frame.is_some() && feasible {
            0.0_f32.clamp(lower, upper)
        } else {
            0.0
        };
        self.pelvis.advance(Vec3::Z * desired, dt, 20.0);
        if frame.is_some() && feasible {
            self.pelvis.position.z = self.pelvis.position.z.clamp(lower, upper);
        }
        let pelvis = up * self.pelvis.position.z;
        let mut rotations = pose.rotations.to_vec();
        for side in 0..2 {
            if !pose.allowed[side] {
                self.feet[side] = Foot::default();
                continue;
            }
            let leg = legs[side];
            let [hip, knee, ankle] = leg.joints;
            let p = |joint: usize| source[joint].w_axis.truncate() + pelvis;
            if !self.feet[side].planted {
                targets[side] += pelvis;
            }
            if frame.is_some() && supported[side] {
                // Re-query the final footprint, including pelvic movement,
                // so heel/toe corners crossing an edge cannot use an old plane.
                let corners = points(targets[side], orientations[side], leg.sole);
                let supports = corners.map(|point| surface(transform, point, ground));
                let required = lift(corners, supports, up).unwrap_or(0.0);
                targets[side] += up * if self.feet[side].planted { required } else { required.max(0.0) };
            }
            if targets[side].distance_squared(p(ankle)) < 1e-10 && orientations[side].abs_diff_eq(rotation(&source, ankle), 1e-6) {
                continue;
            }
            let fallback = self.feet[side].pole.unwrap_or_else(|| rotation(&source, hip) * Vec3::X);
            let Some((a, b, pole)) = solve(p(hip), p(knee), p(ankle), targets[side], fallback) else {
                continue;
            };
            self.feet[side].pole = Some(pole);
            let parent = rig.parent[hip].map_or(Quat::IDENTITY, |joint| rotation(&source, joint));
            let hip_rotation = (a * rotation(&source, hip)).normalize();
            let knee_rotation = (b * a * rotation(&source, knee)).normalize();
            rotations[hip] = (parent.inverse() * hip_rotation).normalize();
            rotations[knee] = (hip_rotation.inverse() * knee_rotation).normalize();
            rotations[ankle] = (knee_rotation.inverse() * orientations[side]).normalize();
        }
        let output = Placed { rotations, pelvis };
        log::trace!(target: "aurora_viewer::foot_placement", "result support={supported:?} planted={:?} pelvis={pelvis:?} targets={targets:?}",
            [self.feet[0].planted, self.feet[1].planted]);
        self.held = Some(output.clone());
        Some(output)
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
            let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
            let floor = rest[rig.names["mFootLeft"]].w_axis.z;
            Self { rig, base, floor }
        }

        fn frame(&self, height: f32) -> Frame {
            Frame {
                to_world: Mat4::from_translation(Vec3::Z * height),
                contact: None,
                velocity_z: 0.0,
                sole_height: None,
            }
        }

        fn sample(
            &self,
            placement: &mut Placement,
            rotations: &[Quat],
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
                        pelvis: Vec3::ZERO,
                        allowed: [true; 2],
                    },
                    frame,
                    at,
                    ground,
                )
                .unwrap_or_else(|| Placed {
                    rotations: rotations.to_vec(),
                    pelvis: Vec3::ZERO,
                })
        }

        fn check_contacts(
            &self,
            placement: &Placement,
            pose: &Placed,
            frame: Frame,
            ground: &dyn Fn(Vec3) -> Option<Surface>,
            planted: [bool; 2],
        ) {
            let matrices = self
                .rig
                .world_with(&pose.rotations, pose.pelvis, &self.base.local_pos, &self.base.scale);
            for (side, leg) in placement.legs.expect("legs").into_iter().enumerate() {
                let [hip, knee, ankle] = leg.joints;
                let rest = self
                    .rig
                    .world_with(&self.rig.local_rot, Vec3::ZERO, &self.base.local_pos, &self.base.scale);
                for (a, b) in [(hip, knee), (knee, ankle)] {
                    assert!(
                        (matrices[a].w_axis.distance(matrices[b].w_axis) - rest[a].w_axis.distance(rest[b].w_axis)).abs() < 1e-4,
                        "segment length changed"
                    );
                }
                let corners = points(matrices[ankle].w_axis.truncate(), rotation(&matrices, ankle), leg.sole);
                let mut nearest = f32::INFINITY;
                for point in corners {
                    let point = frame.to_world.transform_point3(point);
                    if let Some(support) = ground(point) {
                        let distance = (point - support.position).dot(support.normal);
                        assert!(distance > -0.001, "foot {side} penetrates by {distance} m");
                        nearest = nearest.min(distance);
                    }
                }
                if planted[side] {
                    assert!(nearest < 0.003, "foot {side} floats {nearest} m above support");
                }
            }
        }
    }

    #[test]
    fn grounding_reach_constraints_lower_the_pelvis_instead_of_leaving_a_foot_floating() {
        let f = Fixture::new();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(f.floor + if p.y > 0.0 { 0.22 } else { 0.0 }),
                normal: Vec3::Z,
            })
        };
        let mut placement = Placement::default();
        let frame = f.frame(0.28);
        let pose = f.sample(&mut placement, &f.rig.local_rot, Some(frame), Instant::now(), &ground);
        assert!(
            pose.pelvis.z < -0.20,
            "a solver that only rotates the legs cannot reach this lower step"
        );
        f.check_contacts(&placement, &pose, frame, &ground, [true; 2]);
    }

    #[test]
    fn grounding_demo_shape_has_two_distinct_step_contacts_with_the_whole_sole_footprint() {
        let library = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = crate::world::World::new(library.clone());
        for event in crate::demo::events() {
            world.apply(event);
        }
        let shape = library
            .shape_params
            .shape(world.visual_params(&world.agent_id).unwrap_or(&[]), &library.rig);
        let mut f = Fixture::new();
        f.base = SkeletonBase {
            local_pos: shape.local_pos,
            scale: shape.scale,
        };
        let rest = f.rig.world_with(&f.rig.local_rot, Vec3::ZERO, &f.base.local_pos, &f.base.scale);
        f.floor = rest[f.rig.names["mFootLeft"]].w_axis.z;
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(f.floor + if p.y > 0.0 { 0.22 } else { 0.0 }),
                normal: Vec3::Z,
            })
        };
        let mut frame = f.frame(0.22);
        frame.to_world.w_axis.y = 0.005;
        let mut placement = Placement::default();
        let pose = f.sample(&mut placement, &f.rig.local_rot, Some(frame), Instant::now(), &ground);
        for (side, leg) in placement.legs.unwrap().into_iter().enumerate() {
            let ankle = leg.joints[2];
            let corners = points(rest[ankle].w_axis.truncate(), rotation(&rest, ankle), leg.sole);
            assert!(
                corners
                    .into_iter()
                    .all(|point| (frame.to_world.transform_point3(point).y > 0.0) == (side == 0)),
                "a sole corner crossed the step seam: both feet would use the high step"
            );
        }
        assert!(pose.pelvis.z < -0.15);
        f.check_contacts(&placement, &pose, frame, &ground, [true; 2]);
    }

    #[test]
    fn grounding_every_heel_and_toe_corner_stays_above_a_slope() {
        let f = Fixture::new();
        let normal = Vec3::new(-0.18, 0.28, 1.0).normalize();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(f.floor - (normal.x * p.x + normal.y * p.y) / normal.z),
                normal,
            })
        };
        let mut placement = Placement::default();
        let frame = f.frame(0.12);
        let start = Instant::now();
        for i in 0..=120 {
            let pose = f.sample(
                &mut placement,
                &f.rig.local_rot,
                Some(frame),
                start + Duration::from_secs_f32(i as f32 / 60.0),
                &ground,
            );
            f.check_contacts(&placement, &pose, frame, &ground, [true; 2]);
        }
    }

    #[test]
    fn grounding_constraint_projection_prevents_smoothing_from_causing_floor_penetration() {
        let f = Fixture::new();
        let start = Instant::now();
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            for i in 0..fps * 2 {
                let height = if i < fps { 0.0 } else { 0.18 };
                let ground = |p: Vec3| {
                    Some(Surface {
                        position: p.with_z(f.floor + height),
                        normal: Vec3::Z,
                    })
                };
                let frame = f.frame(0.20);
                let pose = f.sample(
                    &mut placement,
                    &f.rig.local_rot,
                    Some(frame),
                    start + Duration::from_secs_f32(i as f32 / fps as f32),
                    &ground,
                );
                f.check_contacts(&placement, &pose, frame, &ground, [true; 2]);
            }
        }
    }

    #[test]
    fn grounding_a_swing_foot_keeps_its_animation_but_cannot_pass_through_a_higher_step() {
        let f = Fixture::new();
        let mut raised = f.rig.local_rot.clone();
        raised[f.rig.names["mHipLeft"]] = Quat::from_rotation_y(-0.6);
        raised[f.rig.names["mKneeLeft"]] = Quat::from_rotation_y(1.2);
        raised[f.rig.names["mAnkleLeft"]] = Quat::from_rotation_y(-0.6);
        let now = Instant::now();
        for step in [0.0, 0.25] {
            let mut placement = Placement::default();
            let ground = |p: Vec3| {
                Some(Surface {
                    position: p.with_z(f.floor + if p.y > 0.0 { step } else { 0.0 }),
                    normal: Vec3::Z,
                })
            };
            let frame = f.frame(0.0);
            let mut pose = f.sample(&mut placement, &raised, Some(frame), now, &ground);
            for i in 1..120 {
                pose = f.sample(
                    &mut placement,
                    &raised,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / 60.0),
                    &ground,
                );
            }
            assert!(!placement.feet[0].planted);
            f.check_contacts(&placement, &pose, frame, &ground, [false, true]);
            if step == 0.0 {
                for name in ["mHipLeft", "mKneeLeft", "mAnkleLeft"] {
                    let j = f.rig.names[name];
                    assert!(pose.rotations[j].abs_diff_eq(raised[j], 0.001), "clear swing pose was replaced");
                }
            }
        }
    }

    #[test]
    fn grounding_preserves_the_animated_knee_plane_and_ankle_roll() {
        let (hip, knee, ankle) = (Vec3::Z, Vec3::new(-0.2, 0.0, 0.5), Vec3::ZERO);
        let (a, _, pole) = solve(hip, knee, ankle, Vec3::new(0.0, 0.0, 0.15), Vec3::X).expect("valid chain");
        assert!(
            pole.x < 0.0 && (hip + a * (knee - hip)).x < 0.0,
            "authored backwards bend was forced forwards"
        );
        let f = Fixture::new();
        let mut rot = f.rig.local_rot.clone();
        rot[f.rig.names["mAnkleLeft"]] = Quat::from_rotation_y(-0.25);
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(f.floor),
                normal: Vec3::Z,
            })
        };
        let mut placement = Placement::default();
        let frame = f.frame(0.0);
        let pose = f.sample(&mut placement, &rot, Some(frame), Instant::now(), &ground);
        let world = f.rig.world_with(&pose.rotations, pose.pelvis, &f.base.local_pos, &f.base.scale);
        assert!(rotation(&world, f.rig.names["mAnkleLeft"]).abs_diff_eq(Quat::from_rotation_y(-0.25), 0.0001));
        f.check_contacts(&placement, &pose, frame, &ground, [true; 2]);
    }

    #[test]
    fn grounding_disable_fades_to_the_authored_pose_without_support_queries() {
        let f = Fixture::new();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(f.floor),
                normal: Vec3::Z,
            })
        };
        let mut placement = Placement::default();
        let now = Instant::now();
        f.sample(&mut placement, &f.rig.local_rot, Some(f.frame(0.2)), now, &ground);
        let no_query = |_: Vec3| -> Option<Surface> { panic!("disabled placement queried geometry") };
        let mut pose = None;
        for i in 1..=240 {
            pose = Some(f.sample(
                &mut placement,
                &f.rig.local_rot,
                None,
                now + Duration::from_secs_f32(i as f32 / 60.0),
                &no_query,
            ));
        }
        let pose = pose.unwrap();
        assert!(pose.pelvis.length() < 0.0001);
        assert!(pose.rotations.iter().zip(&f.rig.local_rot).all(|(a, b)| a.abs_diff_eq(*b, 0.0001)));
    }

    #[test]
    fn grounding_pause_and_teleport_do_not_drag_old_foot_anchors() {
        let f = Fixture::new();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(f.floor),
                normal: Vec3::Z,
            })
        };
        let mut placement = Placement::default();
        let now = Instant::now();
        let before = f.sample(&mut placement, &f.rig.local_rot, Some(f.frame(0.15)), now, &ground);
        placement.resume(Duration::from_secs(10));
        let after = f.sample(
            &mut placement,
            &f.rig.local_rot,
            Some(f.frame(0.15)),
            now + Duration::from_secs(10),
            &ground,
        );
        assert_eq!(before.pelvis, after.pelvis);
        let mut frame = f.frame(0.15);
        frame.to_world.w_axis.x = 100.0;
        let pose = f.sample(
            &mut placement,
            &f.rig.local_rot,
            Some(frame),
            now + Duration::from_secs(11),
            &ground,
        );
        assert!(placement.feet.iter().all(|foot| foot.anchor.is_none_or(|anchor| anchor.x > 99.0)));
        f.check_contacts(&placement, &pose, frame, &ground, [true; 2]);
    }

    #[test]
    fn grounding_spring_response_is_identical_at_30_60_and_144_fps() {
        let mut results = Vec::new();
        for fps in [30, 60, 144] {
            let mut spring = Spring::default();
            for _ in 0..fps {
                spring.advance(Vec3::new(0.1, -0.3, 0.2), 1.0 / fps as f32, 20.0);
            }
            results.push(spring.position);
        }
        assert!(results[0].distance(results[1]) < 1e-6 && results[1].distance(results[2]) < 1e-6);
    }

    #[test]
    fn grounding_uses_the_existing_shoe_sole_calibration_instead_of_sinking_shoes() {
        let f = Fixture::new();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(f.floor),
                normal: Vec3::Z,
            })
        };
        let mut frame = f.frame(0.12);
        frame.sole_height = Some(f.floor - 0.12);
        let mut placement = Placement::default();
        let pose = f.sample(&mut placement, &f.rig.local_rot, Some(frame), Instant::now(), &ground);
        assert!(pose.pelvis.z.abs() < 0.01, "IK cancelled the shoe's root lift");
        f.check_contacts(&placement, &pose, frame, &ground, [true; 2]);
    }

    #[test]
    fn grounding_one_unsupported_foot_does_not_release_the_other() {
        let f = Fixture::new();
        let ground = |p: Vec3| {
            (p.y < 0.0).then_some(Surface {
                position: p.with_z(f.floor),
                normal: Vec3::Z,
            })
        };
        let mut placement = Placement::default();
        let frame = f.frame(0.15);
        let pose = f.sample(&mut placement, &f.rig.local_rot, Some(frame), Instant::now(), &ground);
        assert!(!placement.feet[0].planted && placement.feet[1].planted);
        f.check_contacts(&placement, &pose, frame, &ground, [false, true]);
    }

    #[test]
    fn grounding_priority_control_and_preview_protect_the_authored_pose() {
        let f = Fixture::new();
        let now = Instant::now();
        let signal = PlayingAnimation {
            id: Uuid::from_u128(42),
            sequence: 1,
            sequence_start: now,
            continuous_start: now,
        };
        for (control, preview) in [(false, false), (true, false), (false, true)] {
            let mut animation = crate::demo::grounding::lifting_animation();
            animation.base_priority = if !control && !preview { 6 } else { 2 };
            let anim = Arc::new(BoundAnim::bind(animation, &f.rig));
            let mut controller = Controller {
                control,
                ground: Some(f.frame(0.2)),
                ..Default::default()
            };
            if preview {
                controller.set_preview_clock(Some((Some(signal.id), now)));
            }
            controller.sync(&[signal], now, |_| Some(anim.clone()));
            let mut output = vec![Mat4::IDENTITY.to_cols_array_2d(); f.rig.len()];
            for i in 60..120 {
                let at = now + Duration::from_secs_f32(i as f32 / 60.0);
                controller.sync(&[signal], at, |_| Some(anim.clone()));
                controller.evaluate_on_ground(&f.rig, at, Some(&f.base), &mut output, |_| panic!("protected pose queried support"));
            }
            assert!(controller.placement.pelvis.position.length() < 1e-6);
        }
    }

    #[test]
    fn grounding_seated_and_airborne_clips_do_not_acquire_support() {
        let f = Fixture::new();
        let now = Instant::now();
        let anim = Arc::new(BoundAnim::bind(crate::demo::grounding::animation(false), &f.rig));
        for state in [State::Sitting, State::GroundSit, State::FlyingSlow, State::Falling, State::Hovering] {
            let signal = PlayingAnimation {
                id: state.motion(),
                sequence: 1,
                sequence_start: now,
                continuous_start: now,
            };
            let mut controller = Controller {
                ground: Some(f.frame(0.2)),
                ..Default::default()
            };
            let mut output = vec![Mat4::IDENTITY.to_cols_array_2d(); f.rig.len()];
            controller.sync(&[signal], now, |_| Some(anim.clone()));
            for i in 0..60 {
                let at = now + Duration::from_secs_f32(i as f32 / 60.0);
                controller.evaluate_on_ground(&f.rig, at, Some(&f.base), &mut output, |_| {
                    panic!("unplanted motion queried support")
                });
            }
            assert!(controller.placement.held.is_none());
        }
    }

    #[test]
    fn grounding_lift_and_replant_keep_clearance_when_the_pelvis_is_lowered_between_steps() {
        let f = Fixture::new();
        let animation = crate::demo::grounding::lifting_animation();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(f.floor + if p.y > 0.0 { 0.22 } else { 0.0 }),
                normal: Vec3::Z,
            })
        };
        let now = Instant::now();
        let frame = f.frame(0.28);
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            let mut previous_pelvis = Vec3::ZERO;
            for i in 0..=fps * 5 {
                let t = (i as f32 / fps as f32 - 1.0).max(0.0);
                let mut rot = f.rig.local_rot.clone();
                for joint in &animation.joints {
                    if let (Some(&j), Some(r)) = (
                        f.rig.names.get(&joint.joint_name),
                        joint.rotation_at_loop(t, animation.loop_range()),
                    ) {
                        rot[j] = r;
                    }
                }
                let pose = f.sample(
                    &mut placement,
                    &rot,
                    Some(frame),
                    now + Duration::from_secs_f32(i as f32 / fps as f32),
                    &ground,
                );
                let planted = [placement.feet[0].planted, placement.feet[1].planted];
                f.check_contacts(&placement, &pose, frame, &ground, planted);
                if i > fps {
                    assert!(
                        pose.pelvis.distance(previous_pelvis) < 0.8 / fps as f32,
                        "the pelvis snapped at toe-off/replanting: {:?} -> {:?} at {fps} fps",
                        previous_pelvis,
                        pose.pelvis
                    );
                }
                previous_pelvis = pose.pelvis;
                let side = if i == fps * 2 {
                    Some(0)
                } else if i == fps * 4 {
                    Some(1)
                } else {
                    None
                };
                if let Some(side) = side {
                    let world = f.rig.world_with(&pose.rotations, pose.pelvis, &f.base.local_pos, &f.base.scale);
                    let leg = placement.legs.unwrap()[side];
                    let ankle = leg.joints[2];
                    let gap = points(world[ankle].w_axis.truncate(), rotation(&world, ankle), leg.sole)
                        .into_iter()
                        .map(|point| {
                            let point = frame.to_world.transform_point3(point);
                            point.z - ground(point).unwrap().position.z
                        })
                        .fold(f32::INFINITY, f32::min);
                    assert!(gap > 0.06, "swing foot {side} lost its authored clearance at {fps} fps: {gap}");
                }
            }
        }
    }

    #[test]
    fn grounding_releases_a_slow_authored_lift_before_it_can_pin_the_foot() {
        let f = Fixture::new();
        let now = Instant::now();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(f.floor),
                normal: Vec3::Z,
            })
        };
        let rest = f.rig.world_with(&f.rig.local_rot, Vec3::ZERO, &f.base.local_pos, &f.base.scale);
        let [hip, knee, ankle] = ["mHipLeft", "mKneeLeft", "mAnkleLeft"].map(|name| f.rig.names[name]);
        let length = rest[hip].w_axis.distance(rest[knee].w_axis) + rest[knee].w_axis.distance(rest[ankle].w_axis);
        for fps in [30, 60, 144] {
            let mut placement = Placement::default();
            for i in 0..=fps * 2 {
                let height = i as f32 / fps as f32 * 0.02;
                let angle = (1.0 - height / length).acos();
                let mut rotations = f.rig.local_rot.clone();
                rotations[hip] = Quat::from_rotation_y(-angle);
                rotations[knee] = Quat::from_rotation_y(2.0 * angle);
                rotations[ankle] = Quat::from_rotation_y(-angle);
                let pose = f.sample(
                    &mut placement,
                    &rotations,
                    Some(f.frame(0.0)),
                    now + Duration::from_secs_f32(i as f32 / fps as f32),
                    &ground,
                );
                if height > 0.01 {
                    assert!(!placement.feet[0].planted, "a slow lift was pinned at {fps} fps");
                    let world = f.rig.world_with(&pose.rotations, pose.pelvis, &f.base.local_pos, &f.base.scale);
                    let leg = placement.legs.unwrap()[0];
                    let gap = points(world[ankle].w_axis.truncate(), rotation(&world, ankle), leg.sole)
                        .into_iter()
                        .map(|point| point.z - f.floor)
                        .fold(f32::INFINITY, f32::min);
                    assert!(gap > height * 0.7, "the slow lift lost its clearance: {gap} at {fps} fps");
                }
            }
        }
    }

    #[test]
    fn grounding_runs_after_blending_without_feeding_corrections_into_the_source_pose() {
        let f = Fixture::new();
        let mut first = crate::demo::grounding::animation(false);
        first.ease_in = 2.0;
        let mut second = first.clone();
        for joint in &mut second.joints {
            let angle = if joint.joint_name.starts_with("mHip") {
                -0.3
            } else if joint.joint_name.starts_with("mKnee") {
                0.6
            } else if joint.joint_name.starts_with("mAnkle") {
                -0.3
            } else {
                continue;
            };
            for key in &mut joint.rot_keys {
                key.rotation = Quat::from_rotation_y(angle);
            }
        }
        let clips = [Arc::new(BoundAnim::bind(first, &f.rig)), Arc::new(BoundAnim::bind(second, &f.rig))];
        let now = Instant::now();
        let signals = [42u128, 43].map(|id| PlayingAnimation {
            id: Uuid::from_u128(id),
            sequence: 1,
            sequence_start: now,
            continuous_start: now,
        });
        let mut controller = Controller {
            ground: Some(f.frame(0.2)),
            ..Default::default()
        };
        for at in [now, now + Duration::from_secs(1)] {
            controller.sync(&signals, at, |id| Some(clips[usize::from(*id == signals[1].id)].clone()));
        }
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(f.floor),
                normal: Vec3::Z,
            })
        };
        let mut output = vec![Mat4::IDENTITY.to_cols_array_2d(); f.rig.len()];
        controller.evaluate_on_ground(&f.rig, now + Duration::from_secs(1), Some(&f.base), &mut output, ground);
        let source = controller.rot.clone();
        f.check_contacts(
            &controller.placement,
            controller.placement.held.as_ref().unwrap(),
            f.frame(0.2),
            &ground,
            [true; 2],
        );
        controller.evaluate_on_ground(&f.rig, now + Duration::from_secs(1), Some(&f.base), &mut output, ground);
        assert!(
            source.iter().zip(&controller.rot).all(|(a, b)| a.abs_diff_eq(*b, 1e-6)),
            "the authored pose accumulated IK"
        );
    }
}
