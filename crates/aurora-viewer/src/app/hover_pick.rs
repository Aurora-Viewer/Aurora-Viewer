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
//! cursor's target or where it comes from changes, and a `hover trace
//! cost` line every 5 s with the time spent on the CPU (docs/TESTING.md).

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

/// The question an answer of the GPU speaks for: the pixel asked, seen by
/// the camera of the frame that drew it. The request leaves with the frame
/// after the question, or later when no read back is free, and the camera
/// may have moved in between: measured along the ray asked, a face drawn
/// once the camera is back from far away lies as far out as the camera
/// was. The depth and the reach stay those asked.
fn answered(asked: Question, drawn: &aurora_render::Drawn) -> Question {
    Question {
        ray: drawn.ray.unwrap_or(asked.ray),
        ..asked
    }
}

/// What the hover keeps of `drawn`, the answer for the pixel `asked`,
/// measured from the camera that drew it ([`answered`]). `question` is
/// the pointer's now, at the centre of its pixel, and `ray` the same
/// unrounded (the CPU's body clearance). The answer for a pixel the
/// pointer has left is kept too: it tells what is drawn around it
/// ([`Kept::between`]).
#[allow(clippy::too_many_arguments)]
fn keep(
    scene: &crate::scene::Scene,
    world: &crate::world::World,
    asked: Question,
    drawn: &aurora_render::Drawn,
    objects: &crate::scene::drawn::DrawnObjects,
    question: Question,
    ray: (Vec3, Vec3),
    now: Instant,
) -> Kept {
    let asked = answered(asked, drawn);
    let here = asked.same_ray(&question);
    let hit = if here {
        scene.drawn_hover_of(world, drawn, objects, ray, question.depth, question.far)
    } else {
        scene.drawn_hover_of(world, drawn, objects, asked.ray, asked.depth, asked.far)
    };
    let question = if here { question } else { asked };
    Kept {
        question,
        hit,
        at: now,
        under: scene.drawn_hover_under_of(world, drawn, objects, asked.ray),
        behind: behind(hit, &question),
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
    /// the ray of the camera that drew it, the asked one when the answer
    /// has none (`answered`, `Scene::drawn_hover_under`).
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

/// AURORA_HOVER_TRACE: the time one part of the hover took, over the
/// frames of a `hover trace cost` line.
#[derive(Clone, Copy, Default)]
struct Span {
    calls: u32,
    total: Duration,
    max: Duration,
}

impl Span {
    fn add(&mut self, took: Duration) {
        self.calls += 1;
        self.total += took;
        self.max = self.max.max(took);
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let us = |d: Duration| d.as_secs_f64() * 1e6;
        let mean = if self.calls == 0 {
            0.0
        } else {
            us(self.total) / f64::from(self.calls)
        };
        write!(
            f,
            "{} calls, mean {mean:.1} us, max {:.1} us, {:.3} ms in all",
            self.calls,
            us(self.max),
            us(self.total) / 1000.0
        )
    }
}

/// AURORA_HOVER_TRACE: what the hover cost since the last `hover trace
/// cost` line.
#[derive(Default)]
struct Cost {
    since: Option<Instant>,
    frames: u32,
    /// The search of the CPU, at each frame.
    cpu: Span,
    /// What an answer of the GPU is turned into, when one comes in.
    answer: Span,
    /// Of which the scan for the objects of its draw records.
    scan: Span,
    /// The last answer's object looked for under a pointer that moved
    /// (`Scene::drawn_hover_on`, `Scene::drawn_hover_between`).
    between: Span,
    /// How those ended: followed, between, neither (the CPU's answer).
    ends: [u32; 3],
}

impl Cost {
    const EVERY: Duration = Duration::from_secs(5);

    fn frame(&mut self, now: Instant) {
        self.frames += 1;
        let since = *self.since.get_or_insert(now);
        let over = now.duration_since(since);
        if over < Self::EVERY {
            return;
        }
        log::info!(
            "hover trace cost: {} frames in {:.1} s; CPU search: {}; answers: {}; of which the scan: {}; between two answers: {} (followed {}, between {}, neither {})",
            self.frames,
            over.as_secs_f32(),
            self.cpu,
            self.answer,
            self.scan,
            self.between,
            self.ends[0],
            self.ends[1],
            self.ends[2],
        );
        *self = Self::default();
    }
}

#[derive(Default)]
pub(super) struct HoverDrawn {
    pending: Option<(u64, Question, Instant)>,
    kept: Option<Kept>,
    last_request: Option<Instant>,
    /// AURORA_HOVER_TRACE: what the last line said.
    traced: Option<(Source, Option<usize>)>,
    /// AURORA_HOVER_TRACE.
    cost: Cost,
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
        if trace_enabled() {
            self.cost.frame(Instant::now());
        }
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
        // (AURORA_HOVER_TRACE: the clock is read only then)
        let timed = trace_enabled();
        let started = timed.then(Instant::now);
        let cpu = scene.hover_pick(world, ray, depth, far);
        if let Some(started) = started {
            self.cost.cpu.add(started.elapsed());
        }
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
                    let started = timed.then(Instant::now);
                    let objects = scene.drawn_objects(&drawn);
                    if let Some(started) = started {
                        self.cost.scan.add(started.elapsed());
                    }
                    self.kept = Some(keep(scene, world, asked, &drawn, &objects, question, ray, now));
                    if let Some(started) = started {
                        self.cost.answer.add(started.elapsed());
                    }
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
        } else {
            let started = timed.then(Instant::now);
            let followed = kept.hit.1.and_then(|idx| scene.drawn_hover_on(world, idx, ray, depth, kept.behind));
            let found = match followed {
                Some(point) => Some(((Some(point), kept.hit.1), Source::Followed)),
                None => kept
                    .between(&question)
                    .and_then(|(under, depth)| scene.drawn_hover_between(world, under, ray, depth, far))
                    .map(|hit| (hit, Source::Between)),
            };
            if let Some(started) = started {
                self.cost.between.add(started.elapsed());
                let end = match found {
                    Some((_, Source::Followed)) => 0,
                    Some(_) => 1,
                    None => 2,
                };
                self.cost.ends[end] += 1;
            }
            let Some(found) = found else {
                return (cpu, Source::Fallback);
            };
            found
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

    /// An answer is measured along the ray of the camera that drew it, not
    /// the one asked a frame or more before (#5021): asked from 2^25 m out,
    /// then drawn by the camera back at the avatar, the face lies 2 m ahead,
    /// not 2^25 m. Depth and reach stay those asked, and an answer no frame
    /// drew keeps the ray asked.
    #[test]
    fn an_answer_speaks_for_the_camera_that_drew_it() {
        let asked = Question {
            ray: (Vec3::new(0.0, -33_554_432.0, 1.5), Vec3::Y),
            depth: Some(Vec3::new(0.0, -2.0, 1.5)),
            far: 128.0,
        };
        let drew = (Vec3::new(0.0, -5.0, 1.5), Vec3::Y);
        let face = aurora_render::DrawnFace {
            record: 7,
            point: Vec3::new(0.0, -3.0, 1.5),
            st: glam::Vec2::ZERO,
            normal: -Vec3::Y,
            binormal: Vec3::Z,
        };
        let drawn = aurora_render::Drawn {
            opaque: Some(face),
            blend: None,
            ray: Some(drew),
        };
        let a = answered(asked, &drawn);
        // the distance the hover walks from (`touch_depth_of`'s `at`)
        let at = (face.point - a.ray.0).dot(a.ray.1);
        assert!((at - 2.0).abs() < 1e-3, "{at}");
        assert_eq!((a.depth, a.far), (asked.depth, asked.far));
        let a = answered(asked, &aurora_render::Drawn::default());
        assert_eq!((a.ray, a.depth, a.far), (asked.ray, asked.depth, asked.far));
    }

    /// The same through the scene: a rigged mesh body's face, asked from
    /// 2^25 m out and drawn by the camera back at the avatar. What the
    /// hover keeps of the answer is measured along the drawer's ray: the
    /// face 2.5 m ahead, and the pixels around it on the same surface
    /// (`between`). Measured along the ray asked, it lay 2^25 m out, and
    /// the next question from the camera now at the avatar found nothing
    /// to take for the object under the pointer.
    #[test]
    fn what_the_hover_keeps_of_an_answer_is_measured_from_the_camera_that_drew_it() {
        let (mut world, mut scene, mesh, _) = crate::scene::picking::tests::indexed_scene_with_two_objects();
        let wearer = world
            .objects
            .avatars()
            .find(|(_, a)| a.full_id != world.agent_id)
            .map(|(idx, _)| idx);
        let o = world.objects.get_mut(mesh).unwrap();
        o.click_action = crate::interaction::code::TOUCH;
        o.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        scene.gpu[mesh].rigged = true;
        scene.gpu[mesh].owner_avatar = wearer;
        let on_body = Vec3::new(2.5, 0.0, 50.0);
        let drew = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let asked = Question {
            ray: (Vec3::new(-33_554_432.0, 0.0, 50.0), Vec3::X),
            depth: Some(on_body),
            far: 128.0,
        };
        let drawn = aurora_render::Drawn {
            opaque: Some(aurora_render::DrawnFace {
                record: 0,
                point: on_body,
                st: glam::Vec2::splat(0.5),
                normal: -Vec3::X,
                binormal: Vec3::Z,
            }),
            blend: None,
            ray: Some(drew),
        };
        // the pointer now, a pixel beside the one asked, from the camera
        // back at the avatar
        let beside = (Vec3::new(0.0, 0.002, 50.0), Vec3::X);
        let now = Question {
            ray: beside,
            depth: Some(on_body + Vec3::Y * 0.002),
            far: 128.0,
        };
        let objects = scene.drawn_objects(&drawn);
        let kept = keep(&scene, &world, asked, &drawn, &objects, now, beside, Instant::now());
        let (under, at) = kept.under.expect("the mesh under the pointer");
        assert_eq!(under, mesh);
        assert!((at - 2.5).abs() < 1e-3, "{at}");
        assert_eq!(kept.question.ray, drew);
        assert_eq!(kept.between(&now), Some((mesh, on_body + Vec3::Y * 0.002)));
        // an answer no frame drew is measured along the ray asked, as before
        let undrawn = aurora_render::Drawn { ray: None, ..drawn };
        let kept = keep(&scene, &world, asked, &undrawn, &objects, now, beside, Instant::now());
        assert_eq!(kept.question.ray, asked.ray);
        assert_eq!(kept.between(&now), None);
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

    #[test]
    fn the_cost_line_gives_calls_mean_longest_and_sum() {
        let mut span = Span::default();
        span.add(Duration::from_micros(10));
        span.add(Duration::from_micros(30));
        assert_eq!(span.to_string(), "2 calls, mean 20.0 us, max 30.0 us, 0.040 ms in all");
        assert_eq!(Span::default().to_string(), "0 calls, mean 0.0 us, max 0.0 us, 0.000 ms in all");
    }
}
