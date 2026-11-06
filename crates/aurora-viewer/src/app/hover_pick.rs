//! Cursor feedback for posed surfaces, using the left-click GPU query.
//! Ordinary world surfaces keep scene/hover.rs's indexed CPU search. With
//! rigged geometry present, or near a wearer, one query at most per 100 ms
//! supplies the same attachment/body policy as a press, without waiting
//! for the GPU. HUDs retain their separate pick and cursor policy.

use aurora_render::{DrawnMode, Renderer};
use glam::Vec3;
use std::time::{Duration, Instant};

const REFRESH: Duration = crate::scene::hover::REFRESH;
const TIMEOUT: Duration = Duration::from_millis(500);
type Hit = (Option<Vec3>, Option<usize>);

#[derive(Clone, Copy)]
struct Question {
    ray: (Vec3, Vec3),
    depth: Option<Vec3>,
    far: f32,
}

impl Question {
    fn same_ray(&self, other: &Self) -> bool {
        self.far == other.far && self.ray.0.distance(other.ray.0) <= 5e-4 && self.ray.1.distance(other.ray.1) <= 2e-5
    }

    fn same(&self, other: &Self, posed: bool) -> bool {
        self.same_ray(other)
            && (posed
                || match (self.depth, other.depth) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.distance(b) <= 0.002 + 1e-4 * a.distance(self.ray.0),
                    _ => false,
                })
    }
}

/// The CPU still preempts a kept GPU target when it finds a different
/// foreground surface. The 5 cm tolerance is `Scene::designated`'s.
fn foreground(cpu: Hit, kept: Hit, ray: (Vec3, Vec3)) -> bool {
    match (cpu, kept) {
        ((Some(a), Some(i)), (Some(b), Some(j))) if i != j => (a - ray.0).dot(ray.1) + 0.05 < (b - ray.0).dot(ray.1),
        _ => false,
    }
}

#[derive(Default)]
pub(super) struct HoverDrawn {
    pending: Option<(u64, Question, Instant)>,
    kept: Option<(Question, Hit, Instant)>,
    last_request: Option<Instant>,
}

impl HoverDrawn {
    /// A click, HUD or UI owns the pointer. Forget only our own ticket.
    pub(super) fn cancel(&mut self, renderer: &mut Renderer) {
        if let Some((ticket, _, _)) = self.pending.take() {
            renderer.forget_drawn(ticket);
        }
        self.kept = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn pick(
        &mut self,
        renderer: &mut Renderer,
        scene: &mut crate::scene::Scene,
        world: &crate::world::World,
        cursor: (f32, f32),
        ray: Option<(Vec3, Vec3)>,
        depth: Option<Vec3>,
        far: f32,
        click_waiting: bool,
    ) -> Hit {
        let cpu = scene.hover_pick(world, ray, depth, far);
        let Some(ray) = ray.filter(|_| !click_waiting && scene.hover_needs_drawn()) else {
            self.cancel(renderer);
            return cpu;
        };
        // request_drawn truncates to a physical pixel. Fractional pointer
        // motion inside that pixel must not invalidate its answer. Keep
        // the original press ray for the shared CPU body-clearance rules.
        let pixel_ray = renderer.cursor_ray(cursor.0.floor() + 0.5, cursor.1.floor() + 0.5).unwrap_or(ray);
        let question = Question {
            ray: pixel_ray,
            depth,
            far,
        };
        let now = Instant::now();
        if let Some((ticket, asked, started)) = self.pending {
            if let Some(drawn) = renderer.take_drawn(ticket) {
                self.pending = None;
                if asked.same_ray(&question) && now.duration_since(started) < TIMEOUT {
                    let hit = scene.drawn_hover(world, &drawn, ray, depth, far);
                    self.kept = Some((question, hit, now));
                }
            } else if !asked.same_ray(&question) || now.duration_since(started) >= TIMEOUT {
                self.cancel(renderer);
            }
        }
        if self.pending.is_none() && self.last_request.is_none_or(|at| now.duration_since(at) >= REFRESH) {
            let ticket = renderer.request_drawn(cursor.0, cursor.1, DrawnMode::Left);
            log::debug!("drawn hover: requested ticket {ticket}");
            self.pending = Some((ticket, question, now));
            self.last_request = Some(now);
        }
        self.kept
            .filter(|(asked, hit, at)| {
                // An animated opaque attachment changes the depth at every
                // frame. Keep its cursor between queries, just as a Blend
                // attachment over static depth, until the next posed answer.
                let posed = hit
                    .1
                    .and_then(|i| scene.gpu.get(i))
                    .is_some_and(|g| g.rigged || g.owner_avatar.is_some());
                asked.same(&question, posed) && now.duration_since(*at) < REFRESH * 2 && !foreground(cpu, *hit, ray)
            })
            .map_or(cpu, |(_, hit, _)| hit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drawn_hover_answer_cannot_follow_the_cursor_to_another_surface() {
        let q = Question {
            ray: (Vec3::ZERO, Vec3::X),
            depth: Some(Vec3::X * 5.0),
            far: 128.0,
        };
        assert!(q.same(&q, false));
        assert!(!q.same(
            &Question {
                ray: (Vec3::Y, Vec3::X),
                ..q
            },
            false
        ));
        assert!(!q.same(
            &Question {
                ray: (Vec3::ZERO, Vec3::Y),
                ..q
            },
            false
        ));
        assert!(!q.same(
            &Question {
                depth: Some(Vec3::X * 5.01),
                ..q
            },
            false
        ));
        assert!(!q.same(&Question { depth: None, ..q }, false));
        assert!(q.same(
            &Question {
                depth: Some(Vec3::X * 4.8),
                ..q
            },
            true
        ));
        assert!(!q.same(
            &Question {
                ray: (Vec3::Y, Vec3::X),
                ..q
            },
            true
        ));
        assert!(!q.same(&Question { far: 64.0, ..q }, false));
        assert!(q.same(
            &Question {
                depth: Some(Vec3::X * 5.001),
                ..q
            },
            false
        ));
        let kept = (Some(Vec3::X * 5.0), Some(2));
        assert!(foreground((Some(Vec3::X * 4.0), Some(1)), kept, q.ray));
        assert!(!foreground((Some(Vec3::X * 6.0), Some(1)), kept, q.ray));
        assert!(!foreground((Some(Vec3::X * 4.0), Some(2)), kept, q.ray));
        assert!(!foreground((Some(Vec3::X * 4.0), None), kept, q.ray));
    }
}
