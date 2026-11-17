//! Cursor feedback for posed surfaces, using the left-click GPU query.
//! Ordinary world surfaces keep scene/hover.rs's indexed CPU search. With
//! rigged geometry present, or near a wearer, one query at most per 100 ms
//! supplies the same attachment/body policy as a press, without waiting
//! for the GPU. HUDs retain their separate pick and cursor policy.
//!
//! An answer speaks for its pixel. While the pointer moves, the last one
//! still tells which posed object is drawn around it, and the CPU works out
//! the rest at each frame (`Scene::drawn_hover_between`): the cursor used
//! to fall back on the CPU's answer until the pointer stopped, and
//! flickered over a clicker worn under a mesh body. No query is added.

use aurora_render::{DrawnMode, Renderer};
use glam::Vec3;
use std::time::{Duration, Instant};

const REFRESH: Duration = crate::scene::hover::REFRESH;
const TIMEOUT: Duration = Duration::from_millis(500);
/// How far (m) the depth under the pointer may be from the face the last
/// answer saw and still be taken for the same object's.
const SAME_SURFACE: f32 = 0.3;
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

/// The last answer.
#[derive(Clone, Copy)]
struct Kept {
    question: Question,
    hit: Hit,
    at: Instant,
    /// The posed object of its nearest opaque face and how far along the
    /// asked ray, when no blended face lay in front of it.
    under: Option<(usize, f32)>,
}

impl Kept {
    /// The object to take for the one under the pointer, and the depth
    /// there, when `question` is asked from another pixel than the answer:
    /// the surface under the pointer is about where the answer's was.
    fn between(&self, question: &Question) -> Option<(usize, Vec3)> {
        let (under, at) = self.under?;
        let depth = question.depth?;
        let now_at = (depth - question.ray.0).dot(question.ray.1);
        (self.question.far == question.far && (now_at - at).abs() <= SAME_SURFACE).then_some((under, depth))
    }
}

/// What `Kept::under` keeps of an answer.
fn under(scene: &crate::scene::Scene, drawn: &aurora_render::Drawn, ray: (Vec3, Vec3)) -> Option<(usize, f32)> {
    if drawn.blend.is_some() {
        return None;
    }
    let face = drawn.opaque?;
    let (idx, _) = scene.record_object(face.record)?;
    // what the CPU cannot find where it is shown, and what holds it
    scene
        .gpu
        .get(idx)
        .filter(|g| !g.hud && (g.rigged || g.owner_avatar.is_some() || g.is_avatar))?;
    Some((idx, (face.point - ray.0).dot(ray.1)))
}

#[derive(Default)]
pub(super) struct HoverDrawn {
    pending: Option<(u64, Question, Instant)>,
    kept: Option<Kept>,
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
            let late = now.duration_since(started) >= TIMEOUT;
            if let Some(drawn) = renderer.take_drawn(ticket) {
                self.pending = None;
                if !late {
                    // The answer for a pixel the pointer has left is kept
                    // too: it tells what is drawn around it (`between`).
                    let here = asked.same_ray(&question);
                    let hit = if here {
                        scene.drawn_hover(world, &drawn, ray, depth, far)
                    } else {
                        scene.drawn_hover(world, &drawn, asked.ray, asked.depth, asked.far)
                    };
                    self.kept = Some(Kept {
                        question: if here { question } else { asked },
                        hit,
                        at: now,
                        under: under(scene, &drawn, asked.ray),
                    });
                }
            } else if late {
                self.cancel(renderer);
            }
        }
        if self.pending.is_none() && self.last_request.is_none_or(|at| now.duration_since(at) >= REFRESH) {
            let ticket = renderer.request_drawn(cursor.0, cursor.1, DrawnMode::Left);
            log::debug!("drawn hover: requested ticket {ticket}");
            self.pending = Some((ticket, question, now));
            self.last_request = Some(now);
        }
        let Some(kept) = self.kept.filter(|kept| now.duration_since(kept.at) < REFRESH * 2) else {
            return cpu;
        };
        // An animated opaque attachment changes the depth at every
        // frame. Keep its cursor between queries, just as a Blend
        // attachment over static depth, until the next posed answer.
        let posed = kept
            .hit
            .1
            .and_then(|i| scene.gpu.get(i))
            .is_some_and(|g| g.rigged || g.owner_avatar.is_some());
        let hit = if kept.question.same(&question, posed) {
            kept.hit
        } else if let Some((under, depth)) = kept.between(&question) {
            scene.drawn_hover_between(world, under, ray, depth, far)
        } else {
            return cpu;
        };
        if foreground(cpu, hit, ray) { cpu } else { hit }
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

    #[test]
    fn the_last_answer_speaks_for_the_pixels_around_it_on_the_same_surface() {
        let q = Question {
            ray: (Vec3::ZERO, Vec3::X),
            depth: Some(Vec3::X * 5.0),
            far: 128.0,
        };
        let kept = Kept {
            question: q,
            hit: (Some(Vec3::X * 5.1), Some(3)),
            at: Instant::now(),
            under: Some((2, 5.0)),
        };
        // the pointer moved over the same body: its surface is a little
        // nearer or farther there
        let moved = |depth: Option<Vec3>| Question {
            ray: (Vec3::ZERO, (Vec3::X + Vec3::Y * 0.01).normalize()),
            depth,
            ..q
        };
        let on_body = Vec3::new(5.1, 0.051, 0.0);
        assert!(!q.same(&moved(Some(on_body)), true), "another pixel");
        assert_eq!(kept.between(&moved(Some(on_body))), Some((2, on_body)));
        // it left the body: the wall behind, the sky, or another draw distance
        assert_eq!(kept.between(&moved(Some(Vec3::new(8.0, 0.08, 0.0)))), None);
        assert_eq!(kept.between(&moved(Some(Vec3::new(4.0, 0.04, 0.0)))), None);
        assert_eq!(kept.between(&moved(None)), None);
        assert_eq!(
            kept.between(&Question {
                far: 64.0,
                ..moved(Some(on_body))
            }),
            None
        );
        // an answer with a blended face in front, or of an object the CPU
        // finds by itself, speaks for its pixel alone
        let alone = Kept { under: None, ..kept };
        assert_eq!(alone.between(&moved(Some(on_body))), None);
    }
}
