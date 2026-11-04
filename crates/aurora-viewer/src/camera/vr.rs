//! VR body projection and translation-only Avatar framing. The body/camera
//! separation follows XR body-transformer practice; comfort thresholds are Aurora's.
use glam::Vec3;

pub const FOLLOW_START: f32 = 0.18;
pub const FOLLOW_STOP: f32 = 0.05;
const MAX_FOLLOW_ERROR: f32 = 3.0;
const MAX_ACCEPTED_STEP: f32 = 1.0;
const MANUAL_SETTLE_TIME: f32 = 0.25;

/// The rendered head target and independent raw physical displacement sample.
#[derive(Debug, Clone, Copy)]
pub struct FollowGoal {
    pub world: Vec3,
    pub physical: Vec3,
}
#[derive(Debug, Clone, Copy)]
pub struct FollowControl {
    pub yaw: f32,
    pub forward: bool,
    pub braking: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BodyFollow {
    active: bool,
    owns_motion: bool,
    offset: Vec3,
    last_root: Option<Vec3>,
    reference: Option<(u64, Option<u64>)>,
    manual_pause: f32,
    settled_since: Option<std::time::Instant>,
    goal: Option<Vec3>,
    physical: Option<Vec3>,
    rearm_physical: Option<Vec3>,
    budget: f32,
    braking: f32,
    stopped: f32,
    arrival_direction: Vec3,
    missing_age: f32,
    terminating: bool,
    last_yaw: f32,
}
impl BodyFollow {
    pub fn offset(&self) -> Vec3 {
        self.offset
    }
    pub fn rebase(&mut self) {
        *self = Self::default();
    }
    /// Pause synthetic movement without changing the tracked origin offset.
    pub fn pause(&mut self) {
        self.active = false;
        self.owns_motion = false;
        self.last_root = None;
        self.manual_pause = 0.0;
        self.settled_since = None;
        self.goal = None;
        self.physical = None;
        self.rearm_physical = None;
        self.budget = 0.0;
        self.braking = 0.0;
        self.stopped = 0.0;
        self.arrival_direction = Vec3::ZERO;
        self.missing_age = 0.0;
        self.terminating = false;
    }
    pub fn reference(&mut self, generation: u64, origin: Option<u64>) {
        if let Some(previous) = self.reference {
            if previous.0 != generation {
                self.rebase();
            } else if previous.1 != origin {
                self.pause();
            }
        }
        self.reference = Some((generation, origin));
    }
    /// Arrival control for delayed simulator authority. A command-time budget
    /// stops walking before delayed position feedback can turn arrival into pursuit.
    pub fn step(
        &mut self,
        body: Vec3,
        velocity: Vec3,
        yaw: f32,
        goal: Option<FollowGoal>,
        allowed: bool,
        manual: bool,
        dt: f32,
    ) -> Option<FollowControl> {
        if manual {
            self.pause();
            self.manual_pause = MANUAL_SETTLE_TIME;
            return None;
        }
        let valid = goal.filter(|g| g.world.is_finite() && g.physical.is_finite());
        self.missing_age = if valid.is_some() {
            0.0
        } else if dt.is_finite() {
            self.missing_age + dt.max(0.0)
        } else {
            1.0
        };
        let held = if goal.is_none() && self.missing_age <= 0.5 {
            self.goal.zip(self.physical).map(|(world, physical)| FollowGoal { world, physical })
        } else {
            None
        };
        let valid = valid.or(held);
        if allowed && !self.terminating && goal.is_none() && valid.is_none() && self.missing_age <= 0.5 {
            return None;
        }
        if !allowed || valid.is_none() || !body.is_finite() || !velocity.is_finite() || !yaw.is_finite() || !dt.is_finite() || dt <= 0.0 {
            self.active = false;
            self.goal = None;
            self.budget = 0.0;
            if !self.terminating {
                self.rearm_physical = self.physical;
            }
            self.terminating |= self.owns_motion || self.braking > 0.0;
            return self.termination(velocity, yaw, dt);
        }
        if self.terminating {
            return self.termination(velocity, yaw, dt);
        }
        self.last_yaw = yaw;
        let sample = valid?;
        let dt = dt.min(0.1);
        self.manual_pause = (self.manual_pause - dt).max(0.0);
        if self.manual_pause > 0.0 {
            return None;
        }
        let physical = sample.physical.with_z(0.0);
        let error = (sample.world - body).with_z(0.0).length();
        if !error.is_finite() || error > MAX_FOLLOW_ERROR {
            self.active = false;
            self.goal = None;
            self.rearm_physical = self.physical;
            self.terminating |= self.owns_motion || self.braking > 0.0;
            return self.termination(velocity, yaw, dt);
        }
        if self.active || self.braking > 0.0 {
            if let Some(previous) = self.physical {
                let change = physical - previous;
                let updated = self.goal.unwrap_or(sample.world) + change;
                if !updated.is_finite() {
                    self.pause();
                    return None;
                }
                self.goal = Some(updated);
                // Only raw physical advancement buys additional walking time.
                let direction = (updated - body).with_z(0.0).normalize_or_zero();
                self.budget = (self.budget + change.dot(direction) / 3.2).max(0.0);
            }
            self.physical = Some(physical);
        }
        if self.braking > 0.0 {
            self.braking += dt;
            self.stopped = if velocity.truncate().length() < 0.05 {
                self.stopped + dt
            } else {
                0.0
            };
            if (self.braking >= 0.55 && self.stopped >= 0.25) || self.braking >= 2.0 {
                self.braking = 0.0;
                let residual = (self.goal.unwrap_or(body) - body).with_z(0.0);
                let distance = residual.length();
                if self.stopped >= 0.25
                    && distance.is_finite()
                    && distance > FOLLOW_STOP + 0.01
                    && residual.normalize_or_zero().dot(self.arrival_direction) > 0.15
                {
                    // A short pulse may undershoot under authority delay. Only
                    // after STOP has fully settled may another bounded pulse
                    // close a same-direction shortfall; never reverse on overshoot.
                    self.active = true;
                    self.owns_motion = true;
                    self.settled_since = None;
                    self.budget = ((distance - FOLLOW_STOP) * 0.60 / 3.2).max(dt);
                } else {
                    self.active = false;
                    self.goal = None;
                    self.rearm_physical = Some(physical);
                    self.owns_motion = false;
                }
            }
            return Some(FollowControl {
                yaw,
                forward: false,
                braking: true,
            });
        }
        if !self.active {
            if error <= FOLLOW_START || self.rearm_physical.is_some_and(|p| p.distance(physical) < 0.08) {
                return None;
            }
            self.active = true;
            self.owns_motion = true;
            self.settled_since = None;
            self.goal = Some(sample.world);
            self.physical = Some(physical);
            self.budget = ((error - FOLLOW_STOP) * 0.65 / 3.2).max(0.0);
            self.arrival_direction = (sample.world - body).with_z(0.0).normalize_or_zero();
        }
        let delta = (self.goal? - body).with_z(0.0);
        let distance = delta.length();
        let direction = delta.normalize_or_zero();
        // Delayed stop plus inertia: reserve coast distance rather than chasing
        // the head after crossing it. Braking never issues reverse movement.
        let speed = velocity.with_z(0.0).dot(direction).max(0.0);
        if self.budget <= 0.0 || delta.dot(self.arrival_direction) <= 0.0 || distance <= FOLLOW_STOP + speed * 0.30 {
            self.active = false;
            self.braking = dt;
            self.stopped = 0.0;
            return Some(FollowControl {
                yaw,
                forward: false,
                braking: true,
            });
        }
        let angle = (delta.y.atan2(delta.x) - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let next_yaw = yaw + angle * (4.0 * dt).min(1.0);
        let forward = Vec3::new(yaw.cos(), yaw.sin(), 0.0).dot(direction) > 0.98;
        if forward {
            self.budget = (self.budget - dt).max(0.0);
        }
        Some(FollowControl {
            yaw: next_yaw,
            forward,
            braking: false,
        })
    }
    /// Solve arrival in world space independently of the avatar's facing.
    pub fn step_facing(
        &mut self,
        body: Vec3,
        velocity: Vec3,
        facing: f32,
        goal: Option<FollowGoal>,
        allowed: bool,
        manual: bool,
        dt: f32,
    ) -> Option<FollowControl> {
        let delta = self
            .goal
            .zip(self.physical)
            .and_then(|(target, previous)| goal.map(|sample| target + (sample.physical - previous).with_z(0.0)))
            .or(self.goal)
            .or(goal.map(|sample| sample.world))
            .map_or(Vec3::ZERO, |target| target - body);
        let heading = if delta.truncate().length_squared() > 1e-8 {
            delta.y.atan2(delta.x)
        } else {
            facing
        };
        self.step(body, velocity, heading, goal, allowed, manual, dt)
    }

    fn termination(&mut self, velocity: Vec3, yaw: f32, dt: f32) -> Option<FollowControl> {
        if !self.terminating {
            return None;
        }
        let dt = if dt.is_finite() && dt > 0.0 { dt.min(0.1) } else { 0.016 };
        self.braking += dt;
        self.stopped = if velocity.is_finite() && velocity.truncate().length() < 0.05 {
            self.stopped + dt
        } else {
            0.0
        };
        let yaw = if yaw.is_finite() { yaw } else { self.last_yaw };
        self.last_yaw = yaw;
        // Hold an explicit STOP across runtime/focus loss, including delayed
        // accepted responses. An unrelated external push cannot own us forever.
        if (self.braking >= 0.55 && self.stopped >= 0.25) || self.braking >= 2.0 {
            self.terminating = false;
            self.braking = 0.0;
            self.stopped = 0.0;
            self.owns_motion = false;
        }
        Some(FollowControl {
            yaw,
            forward: false,
            braking: true,
        })
    }
    #[cfg(test)]
    pub fn request(&mut self, body: Vec3, head: Option<Vec3>, allowed: bool, manual: bool, dt: f32) -> Option<f32> {
        if manual {
            // Manual motion owns its full displacement, including the first step.
            // Retain the accumulated origin offset so takeover cannot jump the view.
            self.active = false;
            self.owns_motion = false;
            self.manual_pause = MANUAL_SETTLE_TIME;
            return None;
        }
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.2) } else { 0.0 };
        self.manual_pause = (self.manual_pause - dt).max(0.0);
        let valid_head = head.filter(|head| head.is_finite());
        if !allowed || !body.is_finite() || dt == 0.0 || self.manual_pause > 0.0 || valid_head.is_none() {
            self.active = false;
            return None;
        }
        let delta = valid_head? - body;
        let horizontal = delta.truncate();
        let distance = horizontal.length();
        if !distance.is_finite() || distance > MAX_FOLLOW_ERROR {
            self.active = false;
            return None;
        }
        self.active = if self.active {
            distance > FOLLOW_STOP
        } else {
            distance > FOLLOW_START
        };
        if self.active {
            self.settled_since = None;
            self.owns_motion = true;
            Some(horizontal.y.atan2(horizontal.x))
        } else {
            // Accepted stopping/glide responses still belong to the previous
            // automatic walk. Only manual takeover or a rebase releases them.
            None
        }
    }
    #[cfg(test)]
    pub fn accepted_root(&mut self, root: Vec3) -> Vec3 {
        self.accepted_root_at(root, Vec3::ZERO, std::time::Instant::now())
    }
    pub fn accepted_root_at(&mut self, root: Vec3, velocity: Vec3, now: std::time::Instant) -> Vec3 {
        if !root.is_finite() {
            self.active = false;
            self.owns_motion = false;
            self.last_root = None;
            return self.offset;
        }
        if let Some(previous) = self.last_root {
            let delta = root - previous;
            let distance = delta.length();
            if !distance.is_finite() || distance > MAX_ACCEPTED_STEP {
                // A teleport/correction is not an accepted walking step. A region
                // coordinate change is handled separately by `reference`.
                self.rebase();
            } else if self.owns_motion {
                let stopped =
                    !self.active && delta.truncate().length() < 0.001 && velocity.is_finite() && velocity.truncate().length() < 0.05;
                if stopped {
                    let since = *self.settled_since.get_or_insert(now);
                    if now.saturating_duration_since(since) >= std::time::Duration::from_millis(250) {
                        self.owns_motion = false;
                    }
                } else {
                    self.settled_since = None;
                }
                let next = if self.owns_motion {
                    self.offset - delta.with_z(0.0)
                } else {
                    self.offset
                };
                if next.is_finite() {
                    self.offset = next;
                } else {
                    self.rebase();
                }
            }
        }
        self.last_root = Some(root);
        self.offset
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FramingInput {
    pub recenter: bool,
    pub zoom: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WalkOverride {
    active: bool,
}
impl WalkOverride {
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn apply_frame(&self, frame: &mut crate::movement::Frame) {
        if self.active {
            frame.running = false;
            frame.suspended = true;
        }
    }
    pub fn update(&mut self, following: bool, requested_run: bool) -> Option<bool> {
        if following == self.active {
            return None;
        }
        self.active = following;
        Some(!following && requested_run)
    }
}

/// Apply the existing camera distance limits without turning a horizontal dolly
/// into a vertical pan. Infeasible newly changed limits stop that dolly.
pub fn constrained_distance(position: Vec3, previous: Vec3, origin: Vec3, min: f32, max: f32) -> Vec3 {
    if !position.is_finite() || !origin.is_finite() || !min.is_finite() || min < 0.0 || max.is_nan() || max < min {
        return previous;
    }
    let offset = position - origin;
    let distance = offset.length();
    if distance.is_finite() && distance >= min && distance <= max {
        return position;
    }
    let height = offset.z.abs();
    if height > max {
        return previous;
    }
    let lower = (min * min - height * height).max(0.0).sqrt();
    let upper = (max * max - height * height).max(0.0).sqrt();
    let flat = offset.with_z(0.0);
    let distance = flat.length();
    let direction = flat.normalize_or(Vec3::NEG_X);
    let candidate = origin + (direction * distance.clamp(lower, upper)).with_z(offset.z);
    if candidate.is_finite() { candidate } else { previous }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vr_arrival_budget_brakes_delayed_authority_without_reversal_or_camera_motion() {
        use std::collections::VecDeque;
        use std::time::{Duration, Instant};
        for delay in [0.10_f32, 0.25] {
            for distance in [0.31_f32, 0.4, 0.6] {
                for direction in [Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0).normalize()] {
                    let mut follow = BodyFollow::default();
                    let mut body = Vec3::ZERO;
                    let mut velocity = Vec3::ZERO;
                    let facing = 0.0_f32;
                    let dt = 0.01;
                    let now = Instant::now();
                    let idle = FollowControl {
                        yaw: facing,
                        forward: false,
                        braking: false,
                    };
                    let mut queue = VecDeque::from(vec![idle; (delay / dt).round() as usize]);
                    let mut saw_walk = false;
                    let mut saw_brake = false;
                    let mut last_forward_frame = 0;
                    let mut travel = 0.0;
                    follow.accepted_root_at(body, velocity, now);
                    for frame in 0..1200 {
                        let physical = direction * (distance + if frame % 2 == 0 { 0.01 } else { -0.01 });
                        let rendered = body + follow.offset() + physical;
                        let observation = if frame % 47 >= 35 && frame % 47 < 45 {
                            None
                        } else {
                            Some(FollowGoal { world: rendered, physical })
                        };
                        let control = follow
                            .step_facing(body, velocity, facing, observation, true, false, dt)
                            .unwrap_or(FollowControl {
                                yaw: facing,
                                forward: false,
                                braking: false,
                            });
                        saw_brake |= control.braking;
                        if control.forward {
                            last_forward_frame = frame;
                        }
                        saw_walk |= control.forward;
                        // Orientation, FORWARD and STOP all traverse the same
                        // delayed authority path, unlike immediate local prediction.
                        queue.push_back(control);
                        let accepted = queue.pop_front().expect("fixed delay queue");
                        let desired = if accepted.forward {
                            let heading = (accepted.yaw / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
                            Vec3::new(heading.cos(), heading.sin(), 0.0) * 3.2
                        } else {
                            Vec3::ZERO
                        };
                        velocity += (desired - velocity) * if accepted.forward { 0.30 } else { 0.10 };
                        let step = velocity * dt;
                        assert!(step.dot(direction) >= -1e-5, "arrival cannot reverse toward an overshot goal");
                        body += step;
                        travel += step.length();
                        follow.accepted_root_at(body, velocity, now + Duration::from_secs_f32((frame + 1) as f32 * dt));
                        assert!(
                            (body + follow.offset()).length() < 0.002,
                            "accepted catch-up must not move the tracked head"
                        );
                    }
                    assert!(saw_walk && saw_brake);
                    assert!(body.dot(direction) > 0.0);
                    assert!(last_forward_frame < 1000, "arrival must settle within ten seconds");
                    assert!(
                        (direction * distance - body).length() <= FOLLOW_STOP + 0.05,
                        "body must arrive inside padding, distance={distance}, delay={delay}, direction={direction:?}, body={body:?}"
                    );
                    assert!(
                        travel < distance + 0.25,
                        "arrival travel must remain bounded without circles: {travel}"
                    );
                    assert!(
                        follow
                            .step_facing(
                                body,
                                velocity,
                                facing,
                                Some(FollowGoal {
                                    world: body + follow.offset() + direction * (distance + 0.4),
                                    physical: direction * (distance + 0.4),
                                }),
                                true,
                                false,
                                dt
                            )
                            .is_some(),
                        "a genuine new physical step re-arms arrival"
                    );
                }
            }
        }
    }

    #[test]
    fn vr_second_physical_step_does_not_rearm_after_text_or_runtime_focus_loss() {
        let mut follow = BodyFollow {
            rearm_physical: Some(Vec3::X * 0.4),
            ..BodyFollow::default()
        };
        let physical = Vec3::X * 0.8;
        let goal = FollowGoal { world: physical, physical };
        assert!(
            follow
                .step(Vec3::ZERO, Vec3::ZERO, 0.0, Some(goal), true, false, 0.016)
                .is_some_and(|c| c.forward)
        );
        // UI/text ineligibility and tracking loss both use allowed=false,
        // never the manual takeover path. They must retain the new-step latch.
        assert!(
            follow
                .step(Vec3::ZERO, Vec3::ZERO, 0.0, None, false, false, 0.016)
                .is_some_and(|c| c.braking)
        );
        for _ in 0..160 {
            assert!(
                !follow
                    .step(Vec3::ZERO, Vec3::ZERO, 0.0, Some(goal), true, false, 0.016)
                    .is_some_and(|c| c.forward)
            );
        }
        let new = FollowGoal {
            world: physical + Vec3::X * 0.4,
            physical: physical + Vec3::X * 0.4,
        };
        assert!(
            follow
                .step(Vec3::ZERO, Vec3::ZERO, 0.0, Some(new), true, false, 0.016)
                .is_some_and(|c| c.forward)
        );
    }

    #[test]
    fn vr_focus_loss_holds_stop_through_delayed_coast_without_rearming_on_regain() {
        use std::collections::VecDeque;
        use std::time::{Duration, Instant};
        for delay in [0.10_f32, 0.25] {
            let dt = 0.01;
            let mut follow = BodyFollow::default();
            let mut body = Vec3::ZERO;
            let mut velocity = Vec3::ZERO;
            let physical = Vec3::X * 0.6;
            let mut queue = VecDeque::from(vec![false; (delay / dt).round() as usize]);
            let now = Instant::now();
            let mut saw_stop = false;
            follow.accepted_root_at(body, velocity, now);
            for frame in 0..500 {
                let allowed = !(3..45).contains(&frame);
                let goal = allowed.then_some(FollowGoal {
                    world: body + follow.offset() + physical,
                    physical,
                });
                let control = follow.step(body, velocity, 0.0, goal, allowed, false, dt);
                if frame >= 3 {
                    assert!(
                        !control.is_some_and(|c| c.forward),
                        "focus regain must not restart an interrupted physical step"
                    );
                    saw_stop |= control.is_some_and(|c| c.braking);
                }
                queue.push_back(control.is_some_and(|c| c.forward));
                let forward = queue.pop_front().expect("delay queue");
                let desired = if forward { 3.2 } else { 0.0 };
                velocity.x += (desired - velocity.x) * 0.1;
                body += velocity * dt;
                follow.accepted_root_at(body, velocity, now + Duration::from_secs_f32((frame + 1) as f32 * dt));
                assert!((body + follow.offset()).length() < 0.002);
            }
            assert!(saw_stop);
            assert!(
                follow
                    .step(
                        body,
                        velocity,
                        0.0,
                        Some(FollowGoal {
                            world: body + follow.offset() + physical + Vec3::X * 0.4,
                            physical: physical + Vec3::X * 0.4,
                        }),
                        true,
                        false,
                        dt
                    )
                    .is_some_and(|c| c.forward)
            );
        }
    }

    #[test]
    fn vr_in_bounds_distance_constraints_preserve_position_bits() {
        let position = Vec3::new(128.137, -81.763, 2.123);
        assert_eq!(
            constrained_distance(position, Vec3::ZERO, Vec3::new(128.0, -80.0, 1.0), 0.1, 100.0),
            position
        );
    }

    #[test]
    fn vr_arrival_aligns_before_walking_and_ignores_rendered_feedback_drift() {
        let mut follow = BodyFollow::default();
        let physical = Vec3::Y * 0.6;
        let mut yaw = 0.0;
        for frame in 0..60 {
            let c = follow
                .step(
                    Vec3::ZERO,
                    Vec3::ZERO,
                    yaw,
                    Some(FollowGoal {
                        // Render feedback shifts must never buy a new goal or walk budget.
                        world: physical + Vec3::X * frame as f32 * 0.001,
                        physical,
                    }),
                    true,
                    false,
                    0.016,
                )
                .expect("align or brake");
            if frame == 0 {
                assert!(!c.forward);
            }
            assert!(c.yaw >= yaw && c.yaw <= std::f32::consts::FRAC_PI_2);
            yaw = c.yaw;
            if c.braking {
                break;
            }
        }
        assert!((follow.goal.expect("latched goal") - physical).length() < 1e-6);
    }

    #[test]
    fn vr_crossing_pause_retains_origin_offset_without_countering_future_motion() {
        let mut follow = BodyFollow::default();
        follow.reference(1, Some(3));
        follow.accepted_root(Vec3::ZERO);
        follow.request(Vec3::ZERO, Some(Vec3::X * 0.4), true, false, 0.016);
        let offset = follow.accepted_root(Vec3::X * 0.1);
        follow.pause();
        assert_eq!(follow.offset(), offset);
        follow.reference(1, Some(4));
        assert_eq!(follow.accepted_root(Vec3::new(-255.9, 0.0, 0.0)), offset);
        assert_eq!(follow.accepted_root(Vec3::new(-255.8, 0.0, 0.0)), offset);
    }

    #[test]
    fn vr_horizontal_zoom_obeys_camera_distance_limits_without_down_pan() {
        let previous = Vec3::new(-4.0, 0.0, 2.0);
        let p = constrained_distance(Vec3::new(-8.0, 0.0, 2.0), previous, Vec3::ZERO, 1.0, 5.0);
        assert_eq!(p.z, 2.0);
        assert!((p.length() - 5.0).abs() < 1e-5);
        assert_eq!(
            constrained_distance(Vec3::new(-8.0, 0.0, 2.0), previous, Vec3::ZERO, 0.0, 1.0),
            previous
        );
    }

    #[test]
    fn vr_body_follow_hysteresis_counters_only_accepted_motion_and_manual_takes_over_without_a_jump() {
        let mut follow = BodyFollow::default();
        follow.reference(1, Some(3));
        follow.accepted_root(Vec3::ZERO);
        let head = Vec3::new(0.4, 0.0, 1.6);
        assert!(follow.request(Vec3::ZERO, Some(head), true, false, 0.016).is_some());
        assert!(follow.accepted_root(Vec3::X * 0.1).abs_diff_eq(Vec3::X * -0.1, 1e-6));
        assert!((Vec3::X * 0.1 + follow.offset() + head).abs_diff_eq(head, 1e-6));
        assert!(follow.request(Vec3::X * 0.15, Some(head), true, false, 0.016).is_some());
        assert!(follow.request(Vec3::X * 0.36, Some(head), true, false, 0.016).is_none());
        // The stopping response remains body-owned even after releasing controls.
        follow.accepted_root(Vec3::X * 0.37);
        assert!(follow.offset().abs_diff_eq(Vec3::X * -0.37, 1e-6));
        let before = Vec3::X * 0.37 + follow.offset() + head;
        assert!(follow.request(Vec3::X * 0.37, Some(head), true, true, 0.016).is_none());
        assert_eq!(before, Vec3::X * 0.37 + follow.offset() + head);
        follow.accepted_root(Vec3::X * 0.47);
        assert!((Vec3::X * 0.47 + follow.offset() + head - before).abs_diff_eq(Vec3::X * 0.1, 1e-6));
    }
    #[test]
    fn vr_follow_releases_stopping_ownership_before_a_later_external_push_or_vehicle_takeover() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let mut follow = BodyFollow::default();
        follow.accepted_root_at(Vec3::ZERO, Vec3::ZERO, now);
        follow.request(Vec3::ZERO, Some(Vec3::X * 0.4), true, false, 0.016);
        follow.accepted_root_at(Vec3::X * 0.34, Vec3::X, now + Duration::from_millis(20));
        follow.request(Vec3::X * 0.36, Some(Vec3::X * 0.4), true, false, 0.016);
        follow.accepted_root_at(Vec3::X * 0.37, Vec3::X * 0.1, now + Duration::from_millis(40));
        follow.accepted_root_at(Vec3::X * 0.37, Vec3::ZERO, now + Duration::from_millis(100));
        follow.accepted_root_at(Vec3::X * 0.37, Vec3::ZERO, now + Duration::from_millis(400));
        let offset = follow.offset();
        follow.accepted_root_at(Vec3::X * 0.47, Vec3::X, now + Duration::from_secs(1));
        assert_eq!(follow.offset(), offset, "an unrelated later push must move the head with the body");
        follow.rebase();
        follow.accepted_root_at(Vec3::X * 0.4, Vec3::X, now + Duration::from_secs(2));
        assert_eq!(follow.offset(), Vec3::ZERO, "seat/fly/new-view takeover releases follow motion");
    }

    #[test]
    fn vr_body_follow_rejects_stale_invalid_or_ineligible_goals_and_rebases_references() {
        let mut follow = BodyFollow::default();
        follow.reference(1, Some(3));
        follow.accepted_root(Vec3::ZERO);
        assert!(
            follow
                .request(Vec3::ZERO, Some(Vec3::X * (FOLLOW_START - 0.02)), true, false, 0.016)
                .is_none()
        );
        for (head, allowed) in [
            (None, true),
            (Some(Vec3::splat(f32::NAN)), true),
            (Some(Vec3::X * 4.0), true),
            (Some(Vec3::X), false),
        ] {
            assert!(follow.request(Vec3::ZERO, head, allowed, false, 0.016).is_none());
        }
        follow.request(Vec3::ZERO, Some(Vec3::X), true, false, 0.016);
        follow.accepted_root(Vec3::X * 0.1);
        let offset = follow.offset();
        follow.reference(1, Some(4));
        follow.accepted_root(Vec3::new(-255.9, 0.0, 0.0));
        assert_eq!(follow.offset(), offset, "region-coordinate changes are not body movement");
        follow.reference(2, Some(4));
        assert_eq!(follow.offset(), Vec3::ZERO, "runtime recenter explicitly rebases the body origin");
    }
    #[test]
    fn vr_body_follow_catches_up_at_walk_speed_without_chasing_its_own_camera_translation() {
        let mut follow = BodyFollow::default();
        let mut body = Vec3::ZERO;
        let physical_head = Vec3::new(0.8, 0.0, 1.6);
        follow.accepted_root(body);
        for _ in 0..120 {
            let world_head = body + follow.offset() + physical_head;
            if let Some(yaw) = follow.request(body, Some(world_head), true, false, 0.016) {
                body += Vec3::new(yaw.cos(), yaw.sin(), 0.0) * 3.2 * 0.016;
            }
            follow.accepted_root(body);
            assert!((body + follow.offset() + physical_head).abs_diff_eq(physical_head, 1e-5));
        }
        assert!(
            body.x > 0.8 - FOLLOW_STOP && body.x < 0.8 - FOLLOW_STOP + 0.052,
            "body stops inside the padded release radius"
        );
        assert!(follow.request(body, Some(physical_head), true, false, 0.016).is_none());
    }
    #[test]
    fn vr_body_follow_keeps_a_blocked_body_and_its_camera_stationary() {
        let mut follow = BodyFollow::default();
        follow.accepted_root(Vec3::ZERO);
        for _ in 0..60 {
            assert!(follow.request(Vec3::ZERO, Some(Vec3::X * 0.8), true, false, 0.016).is_some());
            assert_eq!(follow.accepted_root(Vec3::ZERO), Vec3::ZERO);
        }
        assert!(follow.request(Vec3::ZERO, Some(Vec3::X * f32::MAX), true, false, 0.016).is_none());
        assert_eq!(follow.accepted_root(Vec3::splat(f32::NAN)), Vec3::ZERO);
    }

    #[test]
    fn vr_follow_walk_override_restores_always_run_temporary_run_and_trigger_preferences() {
        for (always, temp, trigger) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let mut state = WalkOverride::default();
            let requested = always || temp || trigger;
            assert_eq!(state.update(true, requested), Some(false));
            assert_eq!(state.update(true, requested), None);
            assert_eq!(state.update(false, requested), Some(requested));
            assert!(!state.active());
        }
    }
}
