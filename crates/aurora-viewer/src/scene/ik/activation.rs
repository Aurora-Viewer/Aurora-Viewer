//! Per-contact entry gates and reversible local correction envelopes.
//! A new activation never reuses a preceding release or body solution.
use super::*;
use crate::world::rig_graph::Activation;
use glam::Mat4;

enum Phase {
    Waiting,
    Following(f32),
    Releasing(f32),
    Done,
}

struct Channel {
    bone: usize,
    rotation: Quat,
    position: Option<Vec3>,
}

pub(super) struct Held {
    pub goal: Goal,
    pub weight: f32,
    key: Option<graph::ActivationKey>,
    settings: Activation,
    requested: bool,
    phase: Phase,
    correction: Vec<Channel>,
    resolved: Option<Goal>,
    release_weight: f32,
}

fn smooth(elapsed: f32, duration: f32) -> f32 {
    let t = if duration == 0.0 {
        1.0
    } else {
        (elapsed / duration).clamp(0.0, 1.0)
    };
    t * t * (3.0 - 2.0 * t)
}

fn path(rig: &Rig, goal: &Goal) -> Vec<usize> {
    if goal.mode == Mode::Direct {
        vec![goal.bone]
    } else {
        chain(rig, goal.bone, goal.root).unwrap_or_default()
    }
}

impl Held {
    pub fn new(goal: &Goal) -> Self {
        Self {
            goal: goal.clone(),
            weight: 0.0,
            key: goal.follow.as_ref().and_then(graph::Follow::activation_key),
            settings: goal
                .follow
                .as_ref()
                .and_then(graph::Follow::activation_settings)
                .unwrap_or_default(),
            requested: false,
            phase: Phase::Waiting,
            correction: Vec::new(),
            resolved: None,
            release_weight: 0.0,
        }
    }

    #[allow(clippy::too_many_arguments)] // The same shaped source frame as the solver.
    pub fn update(&mut self, incoming: Option<&Goal>, rig: &Rig, source: &LocalPose, pelvis: Vec3, scale: &[Vec3], dt: f32) -> bool {
        let mut boundary = false;
        if let Some(goal) = incoming {
            let key = goal.follow.as_ref().and_then(graph::Follow::activation_key);
            let settings = goal
                .follow
                .as_ref()
                .and_then(graph::Follow::activation_settings)
                .unwrap_or_default();
            if key != self.key || settings != self.settings {
                // Retire the old lifetime even when the new target has not
                // streamed in yet. It cannot inherit a previous release.
                *self = Self::new(goal);
                boundary = true;
            }
        }
        let resolved = incoming.and_then(|goal| {
            if goal.bone >= rig.len() {
                return None;
            }
            goal.follow
                .as_ref()
                .map_or_else(|| Some(goal.clone()), |f| f.resolve(goal, rig, source, pelvis, scale))
        });
        if let Some((incoming, resolved)) = incoming.zip(resolved).filter(|(_, g)| g.position.is_finite()) {
            let fresh = !self.requested;
            let dt = if fresh {
                *self = Self::new(incoming);
                boundary = true;
                0.0
            } else {
                dt
            };
            self.requested = true;
            self.resolved = Some(resolved.clone());
            self.goal = incoming.clone();
            match self.phase {
                Phase::Waiting => {
                    let near = self.settings.distance == 0.0 || {
                        let matrices = rig.world_with(&source.rotations, pelvis, &source.positions, scale);
                        matrices[incoming.bone]
                            .transform_point3(incoming.contact)
                            .distance(resolved.position)
                            <= self.settings.distance
                    };
                    if near {
                        self.phase = Phase::Following(0.0);
                        self.weight = smooth(0.0, self.settings.ease_in);
                        boundary = true;
                    }
                }
                Phase::Following(ref mut elapsed) => {
                    *elapsed += dt;
                    self.weight = smooth(*elapsed, self.settings.ease_in);
                }
                Phase::Releasing(_) | Phase::Done => {}
            }
        } else {
            if !self.requested && matches!(self.phase, Phase::Waiting) {
                self.phase = Phase::Done;
            }
            let dt = if self.requested {
                self.requested = false;
                self.phase = Phase::Releasing(0.0);
                self.release_weight = self.weight;
                self.weight = 0.0;
                boundary = true;
                0.0
            } else {
                dt
            };
            if let Phase::Releasing(ref mut elapsed) = self.phase {
                *elapsed += dt;
                if self.correction.is_empty() || *elapsed >= self.settings.ease_out {
                    self.phase = Phase::Done;
                }
            }
        }
        boundary
    }

    pub fn retained(&self) -> bool {
        !matches!(self.phase, Phase::Done)
    }

    pub fn waiting(&self) -> bool {
        self.requested && matches!(self.phase, Phase::Waiting)
    }

    /// The source animation alone may be outside a finger's radius even
    /// though another binding has placed its wrist beside the target.
    pub fn arrive_after_contacts(&mut self, matrices: &[Mat4]) -> bool {
        if !self.waiting() {
            return false;
        }
        let resolved = self
            .goal
            .follow
            .as_ref()
            .map_or_else(|| Some(self.goal.clone()), |follow| follow.context.preview(&self.goal, matrices));
        let Some(resolved) = resolved else { return false };
        let Some(bone) = matrices.get(self.goal.bone) else { return false };
        if bone.transform_point3(self.goal.contact).distance(resolved.position) > self.settings.distance {
            return false;
        }
        self.resolved = Some(resolved);
        self.phase = Phase::Following(0.0);
        self.weight = smooth(0.0, self.settings.ease_in);
        true
    }

    /// While a peer still controls this joint, fade the released target out
    /// of their blend instead of dropping it or freezing the peer's channels.
    pub fn peer_release(&self) -> Option<(&Goal, f32)> {
        let Phase::Releasing(elapsed) = self.phase else { return None };
        let weight = self.release_weight * (1.0 - smooth(elapsed, self.settings.ease_out));
        (weight > 0.0).then(|| self.resolved.as_ref().map(|g| (g, weight))).flatten()
    }

    pub fn capture(&mut self, rig: &Rig, source: &LocalPose, solved: &LocalPose) {
        if !self.requested || self.weight == 0.0 || self.goal.weight == 0.0 {
            return;
        }
        self.correction.clear();
        self.correction.extend(
            path(rig, &self.goal)
                .into_iter()
                .filter(|&j| j < source.rotations.len())
                .map(|bone| Channel {
                    bone,
                    rotation: solved.rotations[bone],
                    // IK never freezes shape/animation segment positions.
                    position: (self.goal.mode == Mode::Direct).then_some(solved.positions[bone]),
                }),
        );
    }
}

/// Crossfade the last controlled pose toward the live animation, including an
/// abrupt Idle clip change. Shared joints use one sample, never a sum of contacts.
pub(super) fn release(rig: &Rig, held: &[&Held], tasks: &[(&Goal, f32)], source: &LocalPose, pose: &mut LocalPose) {
    if !held.iter().any(|held| matches!(held.phase, Phase::Releasing(_))) {
        return;
    }
    let mut active = vec![false; rig.len()];
    let mut active_positions = vec![false; rig.len()];
    for (goal, _) in tasks.iter().filter(|(g, w)| g.weight > 0.0 && *w > 0.0) {
        for bone in path(rig, goal).into_iter().filter(|&j| j < rig.len()) {
            active[bone] = true;
        }
        if goal.mode == Mode::Direct && goal.bone < rig.len() {
            active_positions[goal.bone] = true;
        }
    }
    let mut weights = vec![0.0; rig.len()];
    let mut position_weights = vec![0.0; rig.len()];
    for held in held {
        let Phase::Releasing(elapsed) = held.phase else { continue };
        let weight = 1.0 - smooth(elapsed, held.settings.ease_out);
        for channel in &held.correction {
            let bone = channel.bone;
            if !active[bone] && weight > weights[bone] {
                weights[bone] = weight;
                pose.rotations[bone] = source.rotations[bone].slerp(channel.rotation, weight).normalize();
            }
            if let Some(position) = channel.position
                && !active_positions[bone]
                && weight > position_weights[bone]
            {
                position_weights[bone] = weight;
                pose.positions[bone] = source.positions[bone].lerp(position, weight);
            }
        }
    }
}

#[cfg(test)]
mod tests;
