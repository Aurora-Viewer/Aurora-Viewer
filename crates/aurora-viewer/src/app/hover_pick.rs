//! Cursor feedback for posed surfaces, using the left-click GPU query.
//! Ordinary world surfaces keep scene/hover.rs's indexed CPU search. With
//! rigged geometry present, or near a wearer, one query at most per 100 ms
//! supplies the same attachment/body policy as a press, without waiting
//! for the GPU. HUDs retain their separate pick and cursor policy.
//!
//! An answer speaks for its pixel. While the pointer moves, the cursor
//! used to fall back on the CPU's answer until the pointer stopped, and
//! flickered wherever the two differ: over a scripted rigged mesh the
//! click acts on, over a clicker worn under a mesh body, over a scripted
//! mesh worn unrigged. So the last answer still tells, at each frame,
//! which worn mesh the cursor was on (the CPU tests it alone under the
//! pointer, `Scene::drawn_hover_on`) and which posed object is drawn
//! around (`Scene::drawn_hover_between`). No query is added.
//!
//! `AURORA_HOVER_TRACE=1` writes a `hover trace` line each time the
//! cursor's target or where it comes from changes (docs/TESTING.md).

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
    /// The posed object the cursor was on at its pixel and how far along
    /// the asked ray (`Scene::drawn_hover_under`).
    under: Option<(usize, f32)>,
    /// How far (m) behind the opaque depth of its pixel `hit` is: 0 in
    /// front of it.
    behind: f32,
}

/// What `Kept::behind` keeps of an answer.
fn behind(hit: Hit, question: &Question) -> f32 {
    match (hit.0, question.depth) {
        (Some(point), Some(depth)) => (point - depth).dot(question.ray.1).max(0.0),
        _ => 0.0,
    }
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

/// Where the cursor's target comes from (AURORA_HOVER_TRACE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// The search of the CPU: nothing posed near the ray.
    Cpu,
    /// The same, for want of an answer of the GPU that speaks for the pixel.
    Fallback,
    /// The answer of the GPU for this pixel.
    Answer,
    /// The last answer's object, met by the pointer's ray on the CPU.
    Followed,
    /// The last answer's posed object, searched from the depth under the
    /// pointer.
    Between,
    /// The CPU found another surface in front of the answer's.
    Foreground,
}

fn trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AURORA_HOVER_TRACE").is_some())
}

#[derive(Default)]
pub(super) struct HoverDrawn {
    pending: Option<(u64, Question, Instant)>,
    kept: Option<Kept>,
    last_request: Option<Instant>,
    /// AURORA_HOVER_TRACE: what the last line said.
    traced: Option<(Source, Option<usize>)>,
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
        let (hit, source) = self.answer(renderer, scene, world, cursor, ray, depth, far, click_waiting);
        if trace_enabled() && self.traced != Some((source, hit.1)) {
            self.traced = Some((source, hit.1));
            let local = |i: usize| world.objects.get(i).map(|o| o.key.local_id);
            let along = |p: Vec3| ray.map(|(o, d)| (p - o).dot(d));
            // what the object is: rigged (and to whose skeleton), a mesh
            // the GPU poses, worn, with a face that is not drawn (alpha 0)
            let kind = hit.1.map(|i| {
                let s = scene.drawn_status(world, i);
                let undrawn = scene.gpu.get(i).is_some_and(|g| g.faces.len() < g.built_faces);
                (s.rigged, s.posed_by_gpu, s.worn, undrawn)
            });
            log::info!(
                "hover trace: {source:?} object={:?} touch={} kind={kind:?} at={:?} depth={:?} under={:?} cursor={:.0},{:.0}",
                hit.1.and_then(local),
                hit.1
                    .is_some_and(|i| crate::interaction::target(world, i, &std::collections::HashMap::new()).is_some()),
                hit.0.and_then(along),
                depth.and_then(along),
                self.kept.and_then(|k| k.under).map(|(i, at)| (local(i), at)),
                cursor.0,
                cursor.1,
            );
        }
        hit
    }

    #[allow(clippy::too_many_arguments)]
    fn answer(
        &mut self,
        renderer: &mut Renderer,
        scene: &mut crate::scene::Scene,
        world: &crate::world::World,
        cursor: (f32, f32),
        ray: Option<(Vec3, Vec3)>,
        depth: Option<Vec3>,
        far: f32,
        click_waiting: bool,
    ) -> (Hit, Source) {
        let cpu = scene.hover_pick(world, ray, depth, far);
        let Some(ray) = ray.filter(|_| !click_waiting && scene.hover_needs_drawn()) else {
            self.cancel(renderer);
            return (cpu, Source::Cpu);
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
                    let question = if here { question } else { asked };
                    self.kept = Some(Kept {
                        question,
                        hit,
                        at: now,
                        under: scene.drawn_hover_under(world, &drawn, asked.ray),
                        behind: behind(hit, &question),
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
            return (cpu, Source::Fallback);
        };
        // An animated opaque attachment changes the depth at every
        // frame. Keep its cursor between queries, just as a Blend
        // attachment over static depth, until the next posed answer.
        let posed = kept
            .hit
            .1
            .and_then(|i| scene.gpu.get(i))
            .is_some_and(|g| g.rigged || g.owner_avatar.is_some());
        let (hit, source) = if kept.question.same(&question, posed) {
            (kept.hit, Source::Answer)
        } else if let Some(point) = kept.hit.1.and_then(|idx| scene.drawn_hover_on(world, idx, ray, depth, kept.behind)) {
            ((Some(point), kept.hit.1), Source::Followed)
        } else if let Some(hit) = kept
            .between(&question)
            .and_then(|(under, depth)| scene.drawn_hover_between(world, under, ray, depth, far))
        {
            (hit, Source::Between)
        } else {
            return (cpu, Source::Fallback);
        };
        if foreground(cpu, hit, ray) {
            (cpu, Source::Foreground)
        } else {
            (hit, source)
        }
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
            behind: 0.0,
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
        // an answer of an object the CPU finds by itself speaks for its
        // pixel alone
        let alone = Kept { under: None, ..kept };
        assert_eq!(alone.between(&moved(Some(on_body))), None);
        // the answer met its object 10 cm under the surface of the body
        assert!((behind(kept.hit, &q) - 0.1).abs() < 1e-6);
        // in front of it, or with nothing opaque behind: not under anything
        assert_eq!(behind((Some(Vec3::X * 4.9), Some(3)), &q), 0.0);
        assert_eq!(behind(kept.hit, &Question { depth: None, ..q }), 0.0);
        assert_eq!(behind((None, None), &q), 0.0);
    }
}
